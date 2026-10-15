//! Sending the anonymous usage events (docs/privacy.md) to PostHog Cloud EU: a background
//! thread posts the queued events ([`fieldcraft_engine::usage`]) in batches every half minute and
//! when the window closes. A failed post (offline, a slow network) drops its batch silently;
//! nothing waits on it but the thread.
//!
//! The project key is a build setting, `FIELDCRAFT_POSTHOG_KEY`, which only the release workflow
//! sets: local, development and fork builds have none and send nothing.

use std::sync::mpsc;
use std::time::Duration;

use fieldcraft_engine::usage;
use fieldcraft_ui_egui::privacy::UsageHost;

/// The PostHog project key the release build was made with.
const KEY: Option<&str> = option_env!("FIELDCRAFT_POSTHOG_KEY");
/// How often queued events are sent.
const INTERVAL: Duration = Duration::from_secs(30);
/// The longest one post may take.
const TIMEOUT: Duration = Duration::from_secs(10);
/// How long closing the window waits for the last batch.
const LAST_WAIT: Duration = Duration::from_secs(2);

fn key() -> Option<&'static str> {
    KEY.map(str::trim).filter(|k| !k.is_empty())
}

/// What the window needs to ask and record, when this run may share usage: the build has a key,
/// `FIELDCRAFT_TELEMETRY` is not `0`, and neither CI nor `--control` automation runs it (unless
/// `FIELDCRAFT_TELEMETRY=1`).
pub fn host(automation: bool) -> Option<UsageHost> {
    let telemetry = std::env::var("FIELDCRAFT_TELEMETRY").ok();
    let ci = std::env::var_os("CI").is_some_and(|v| !v.is_empty());
    usage::allowed(key().is_some(), telemetry.as_deref(), automation, ci).then(|| UsageHost {
        platform: "desktop",
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        new_id: Box::new(new_id),
    })
}

/// A random (version 4) UUID from the operating system's random source.
fn new_id() -> Option<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).ok()?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Some(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

/// The sending thread; [`Sender::finish`] sends what is left and stops it.
pub struct Sender {
    stop: mpsc::Sender<()>,
    finished: mpsc::Receiver<()>,
}

/// Start the sending thread (only when the build has a key). `FIELDCRAFT_TELEMETRY_HOST`
/// replaces PostHog's host, for checking the payload against a local listener.
pub fn start() -> Option<Sender> {
    let key = key()?;
    let host = std::env::var("FIELDCRAFT_TELEMETRY_HOST").ok().filter(|h| !h.trim().is_empty()).unwrap_or_else(|| usage::DEFAULT_HOST.into());
    let url = format!("{}/batch/", host.trim_end_matches('/'));
    // rustls's process-wide crypto (ring); another part of the app may have installed it first.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .build()
        .into();
    let (stop, stopped) = mpsc::channel::<()>();
    let (done, finished) = mpsc::channel::<()>();
    let spawned = std::thread::Builder::new().name("fieldcraft usage".into()).spawn(move || {
        loop {
            let last = !matches!(stopped.recv_timeout(INTERVAL), Err(mpsc::RecvTimeoutError::Timeout));
            while let Some(batch) = usage::take_batch() {
                if let Err(e) = agent.post(&url).header("Content-Type", "application/json").send(batch.body(key)) {
                    // Offline or refused: this batch is dropped, the rest waits for the next try.
                    log::debug!("usage: {} event(s) not sent: {e}", batch.events.len());
                    break;
                }
            }
            if last {
                let _ = done.send(());
                return;
            }
        }
    });
    match spawned {
        Ok(_) => Some(Sender { stop, finished }),
        Err(e) => {
            log::warn!("usage: no sending thread: {e}");
            None
        }
    }
}

impl Sender {
    /// Send what is queued (waiting briefly) and stop.
    pub fn finish(self) {
        let _ = self.stop.send(());
        let _ = self.finished.recv_timeout(LAST_WAIT);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn install_ids_are_random_version_4_uuids() {
        let (a, b) = (super::new_id().unwrap(), super::new_id().unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'4');
        assert!(matches!(a.as_bytes()[19], b'8' | b'9' | b'a' | b'b'), "{a}");
    }
}
