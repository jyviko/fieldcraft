//! A request to terminate (SIGTERM, SIGINT on Unix) closes the window the normal way.
//!
//! Without this, `kill PID`, Ctrl-C in the terminal, a system shutdown or a test harness's timeout
//! ended the process on the spot: no `on_exit`, so no last autosave and no clean end of the crash
//! journal, and the next start filed an `unclean_exit` crash report for what was no crash (#248:
//! "ended without a clean exit between commands (last: control ui.capture)" was an automation run
//! stopped with SIGTERM after its capture). Now the first signal asks the window to close, which
//! autosaves, ends the journal and exits as a click on Close does; the journal notes the signal,
//! so a run that still dies while closing says why. A second signal, for a window that does not
//! close, ends the process at once with the signal's default action.
//!
//! SIGHUP is left alone: `nohup fieldcraft &` ignores it on purpose so the app outlives its
//! terminal, and a handler would override that (a safe API cannot ask whether a signal was
//! ignored at launch).
//!
//! Windows has no such signals: a window there is closed through its messages, which eframe
//! already turns into a normal close.

use std::io;

/// The signals that ask the app to terminate.
#[cfg(unix)]
const SIGNALS: [i32; 2] = [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT];

/// The signal's usual name, for the log and the crash journal.
#[cfg(unix)]
fn name(signal: i32) -> &'static str {
    match signal {
        signal_hook::consts::SIGTERM => "SIGTERM",
        signal_hook::consts::SIGINT => "SIGINT",
        _ => "signal",
    }
}

/// Call `close` (on a thread of its own) on the first termination signal; a second one ends the
/// process with the signal's default action.
#[cfg(unix)]
pub fn watch(close: impl Fn() + Send + 'static) -> io::Result<()> {
    let mut signals = signal_hook::iterator::Signals::new(SIGNALS)?;
    std::thread::Builder::new().name("terminate".into()).spawn(move || {
        let mut asked = false;
        for signal in signals.forever() {
            if asked {
                log::warn!("{}: the window did not close; ending now", name(signal));
                if let Err(e) = signal_hook::low_level::emulate_default_handler(signal) {
                    log::error!("{}: {e}", name(signal));
                    std::process::exit(128 + signal);
                }
                continue;
            }
            asked = true;
            log::info!("{}: closing", name(signal));
            fieldcraft_engine::crash::activity(&format!("terminating ({})", name(signal)));
            close();
        }
    })?;
    Ok(())
}

/// Nothing to watch without Unix signals.
#[cfg(not(unix))]
pub fn watch(_close: impl Fn() + Send + 'static) -> io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;

    use fieldcraft_engine::crash;

    /// The child half: start crash reporting as the app does, then get SIGTERM. With the watcher
    /// the "window" (here the main thread) closes the normal way; without it the process dies.
    #[test]
    fn child() {
        let Some(dir) = std::env::var_os("FIELDCRAFT_TERMINATE_CHILD") else { return };
        let build = crash::Build { version: "9.9.9", commit: "abc1234" };
        let fresh = crash::install(PathBuf::from(dir), build);
        if std::env::var_os("FIELDCRAFT_TERMINATE_NEXT").is_some() {
            // The next start: what it reports of the run before.
            println!("FRESH {}", fresh.iter().map(|s| s.message.as_str()).collect::<Vec<_>>().join(" | "));
            crash::end_session();
            return;
        }
        crash::activity("control ui.capture");
        if std::env::var_os("FIELDCRAFT_TERMINATE_WATCH").is_some() {
            let (tx, rx) = mpsc::channel();
            super::watch(move || {
                let _ = tx.send(());
            })
            .unwrap();
            signal_hook::low_level::raise(signal_hook::consts::SIGTERM).unwrap();
            // What `on_exit` does once the window has closed.
            rx.recv().unwrap();
            crash::end_session();
            println!("CLOSED");
            return;
        }
        signal_hook::low_level::raise(signal_hook::consts::SIGTERM).unwrap();
        // Not reached: SIGTERM's default action ends the process.
        std::thread::sleep(std::time::Duration::from_secs(10));
    }

    #[test]
    fn sigterm_closes_cleanly_and_leaves_no_crash_report() {
        let root = std::env::temp_dir().join(format!("fieldcraft-terminate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let run = |dir: &Path, mode: &str| {
            let mut c = std::process::Command::new(std::env::current_exe().unwrap());
            c.args(["--exact", "terminate::tests::child", "--test-threads=1", "--nocapture"]).env("FIELDCRAFT_TERMINATE_CHILD", dir);
            match mode {
                "watch" => c.env("FIELDCRAFT_TERMINATE_WATCH", "1"),
                "next" => c.env("FIELDCRAFT_TERMINATE_NEXT", "1"),
                _ => &mut c,
            };
            c.output().unwrap()
        };
        // What the next start in `dir` reports of the run before.
        let reported_at_next_start = |dir: &Path| {
            let out = run(dir, "next");
            assert!(out.status.success(), "{out:?}");
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            stdout.lines().find_map(|l| l.split_once("FRESH ").map(|(_, r)| r.trim().to_string())).unwrap_or_else(|| panic!("{stdout}"))
        };

        // Without the watcher (the bug): killed, and the journal it leaves reads as a crash.
        let unwatched = root.join("unwatched");
        let out = run(&unwatched, "kill");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(out.status.signal(), Some(signal_hook::consts::SIGTERM), "{out:?}");
        let reported = reported_at_next_start(&unwatched);
        assert!(reported.contains("ended without a clean exit") && reported.contains("control ui.capture"), "{reported}");

        // With it: the close runs, the journal ends, and nothing is reported as a crash.
        let watched = root.join("watched");
        let out = run(&watched, "watch");
        assert!(out.status.success(), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stdout).contains("CLOSED"), "{out:?}");
        assert_eq!(reported_at_next_start(&watched), "");
        let _ = std::fs::remove_dir_all(&root);
    }
}
