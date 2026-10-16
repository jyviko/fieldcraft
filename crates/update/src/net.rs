//! Talking to the release server: the GitHub Releases API for the release description, and its
//! download links for the files. HTTPS only (checked against the operating system's
//! certificates), except against a local test server named by [`crate::HOST_ENV`].

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::{Check, Error, REPO, Release, Result, Version};

/// The longest a release description may be (bytes).
const MAX_JSON: u64 = 4 * 1024 * 1024;
/// The largest asset downloaded (bytes; FieldCraft's are well under 200 MB).
pub const MAX_ASSET: u64 = 2 * 1024 * 1024 * 1024;
/// How long connecting and the release description may take.
const TIMEOUT: Duration = Duration::from_secs(15);
/// How long a whole download may take.
const DOWNLOAD_TIME: Duration = Duration::from_secs(30 * 60);

/// Which release to ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Which {
    /// The latest published, non-draft, non-pre-release release.
    Latest,
    /// The release of this version (to reinstall it).
    Version(Version),
}

/// A client for one release server.
pub struct Client {
    host: String,
    agent: ureq::Agent,
    downloads: ureq::Agent,
}

impl Client {
    /// A client for `host` (`https://api.github.com`, or a local test server).
    pub fn new(host: &str) -> Client {
        // rustls's process-wide crypto (ring); another part of the app may have installed it.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let tls = || ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).tls_config(tls()).build().into();
        let downloads = ureq::Agent::config_builder()
            .timeout_connect(Some(TIMEOUT))
            .timeout_recv_response(Some(TIMEOUT))
            .timeout_recv_body(Some(DOWNLOAD_TIME))
            .tls_config(tls())
            .build()
            .into();
        Client { host: host.trim_end_matches('/').to_string(), agent, downloads }
    }

    /// The client for [`crate::host`].
    pub fn from_env() -> Client {
        Client::new(&crate::host())
    }

    /// Whether `url` may be fetched: HTTPS, or anything when testing against a local server.
    fn allowed(&self, url: &str) -> Result<()> {
        if url.starts_with("https://") || self.host != crate::DEFAULT_HOST {
            Ok(())
        } else {
            Err(Error::Network(format!("refusing a download that is not HTTPS: {}", url.chars().take(120).collect::<String>())))
        }
    }

    /// The release `which` describes.
    pub fn release(&self, which: &Which) -> Result<Release> {
        let path = match which {
            Which::Latest => format!("/repos/{REPO}/releases/latest"),
            Which::Version(v) => format!("/repos/{REPO}/releases/tags/v{v}"),
        };
        let url = format!("{}{path}", self.host);
        let reply = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", &format!("FieldCraft/{}", crate::current()))
            .config()
            .http_status_as_error(false)
            .build()
            .call()
            .map_err(|e| Error::Network(format!("could not reach the release server: {e}")))?;
        let status = reply.status().as_u16();
        let body = reply
            .into_body()
            .with_config()
            .limit(MAX_JSON)
            .read_to_string()
            .map_err(|e| Error::Network(format!("the release server's reply: {e}")))?;
        match status {
            200 => {}
            404 => {
                return Err(Error::Release(match which {
                    Which::Latest => "no FieldCraft release is published yet".into(),
                    Which::Version(v) => format!("there is no published release of FieldCraft {v}"),
                }));
            }
            403 | 429 => return Err(Error::Network("the release server is limiting requests; try again later".into())),
            other => return Err(Error::Network(format!("the release server answered {other}"))),
        }
        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| Error::Release(format!("the release description is not JSON: {e}")))?;
        Release::from_github(&json)
    }

    /// Compare the latest release with the running version.
    pub fn check(&self) -> Result<Check> {
        Ok(Check { current: crate::current(), latest: self.release(&Which::Latest)? })
    }

    /// A small file (the checksum list, its signature), at most `max` bytes.
    pub fn small(&self, url: &str, max: usize) -> Result<Vec<u8>> {
        self.allowed(url)?;
        let reply = self.agent.get(url).header("User-Agent", "FieldCraft").call().map_err(|e| Error::Network(format!("download failed: {e}")))?;
        reply.into_body().with_config().limit(max as u64).read_to_vec().map_err(|e| Error::Network(format!("download failed: {e}")))
    }

    /// Download `url` to `to`, at most `max` bytes; its SHA-256 (hex). `progress` hears the bytes
    /// so far and the total (0 when not known).
    pub fn download(&self, url: &str, to: &Path, max: u64, progress: &dyn Fn(u64, u64)) -> Result<String> {
        self.allowed(url)?;
        let reply = self.downloads.get(url).header("User-Agent", "FieldCraft").call().map_err(|e| Error::Network(format!("download failed: {e}")))?;
        let total = reply.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        if total > max {
            return Err(Error::Network(format!("the download is {total} bytes, more than the {max} accepted")));
        }
        let mut body = reply.into_body();
        let mut reader = body.with_config().limit(max).reader();
        let mut file = std::fs::File::create(to).map_err(|e| Error::Install(format!("{}: {e}", to.display())))?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            let n = reader.read(&mut buf).map_err(|e| Error::Network(format!("download failed: {e}")))?;
            let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
            hash.update(chunk);
            file.write_all(chunk).map_err(|e| Error::Install(format!("{}: {e}", to.display())))?;
            done += n as u64;
            progress(done, total);
        }
        file.sync_all().map_err(|e| Error::Install(format!("{}: {e}", to.display())))?;
        Ok(crate::verify::hex(&hash.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::mock::Server;

    #[test]
    fn the_latest_release_is_asked_for_and_compared() {
        let server = Server::start().unwrap();
        server.release("latest", &json!({"tag_name": "v99.0.0", "html_url": "https://example.invalid/r", "assets": []}));
        let check = Client::new(&server.url()).check().unwrap();
        assert!(check.newer());
        assert_eq!(check.latest.version.to_string(), "99.0.0");
        assert_eq!(server.requests(), 1);

        // Errors are messages, never panics.
        let missing = Client::new(&server.url()).release(&Which::Version(Version::parse("1.2.3").unwrap())).unwrap_err();
        assert!(missing.to_string().contains("no published release of FieldCraft 1.2.3"), "{missing}");
        server.route("/repos/tangency-labs/fieldcraft/releases/latest", 200, b"not json".to_vec());
        assert!(matches!(Client::new(&server.url()).check(), Err(Error::Release(_))));
        server.route("/repos/tangency-labs/fieldcraft/releases/latest", 403, b"{}".to_vec());
        assert!(matches!(Client::new(&server.url()).check(), Err(Error::Network(_))));
        assert!(matches!(Client::new("http://127.0.0.1:1").check(), Err(Error::Network(_))));
    }

    #[test]
    fn downloads_are_hashed_and_capped() {
        let server = Server::start().unwrap();
        server.route("/f.bin", 200, b"0123456789".to_vec());
        let dir = std::env::temp_dir().join(format!("fieldcraft-update-net-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let to = dir.join("f.bin");
        let c = Client::new(&server.url());
        let sha = c.download(&format!("{}/f.bin", server.url()), &to, 100, &|_, _| {}).unwrap();
        assert_eq!(sha, crate::verify::sha256_hex(b"0123456789"));
        assert!(c.download(&format!("{}/f.bin", server.url()), &to, 5, &|_, _| {}).is_err());
        assert!(c.small(&format!("{}/f.bin", server.url()), 5).is_err());
        // Against GitHub, only HTTPS.
        assert!(Client::new(crate::DEFAULT_HOST).small("http://example.invalid/x", 5).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
