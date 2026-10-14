//! Autosaves kept in the browser.
//!
//! The engine's recovery folder is [`fieldcraft_io::recovery::BROWSER_DIR`] among the page's
//! files, which last only as long as the page. This mirrors it to the browser's local storage
//! (kept across visits, shared by the site's tabs): each file as base64 under
//! [`PREFIX`]`<file name>`. At start the stored files go back into the folder, so the desktop's
//! recovery flow (File › Recover…, `file.recovery.*`) offers what a closed or crashed page left.
//!
//! Local storage holds a few megabytes per site: an autosave larger than [`MAX_AUTOSAVE_BYTES`]
//! (or one the browser refuses) is not kept, and the status bar says so.

use std::collections::BTreeSet;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fieldcraft_io::recovery::BROWSER_DIR;

const PREFIX: &str = "fieldcraft-recovery/";

/// Largest stored file taken back (local storage itself holds less).
const MAX_STORED_CHARS: usize = 64 << 20;

/// Largest autosave kept: base64 makes it a third larger, and browsers give a site about 5 MB.
pub const MAX_AUTOSAVE_BYTES: usize = 3 << 20;

fn local_storage() -> Result<web_sys::Storage, String> {
    web_sys::window().ok_or("no window")?.local_storage().map_err(crate::web::js_err)?.ok_or_else(|| "browser storage is turned off".into())
}

/// The mirror: the stored files this page knows of (took back or wrote), by file name.
#[derive(Default)]
pub struct Store {
    known: BTreeSet<String>,
}

pub fn dir() -> &'static Path {
    Path::new(BROWSER_DIR)
}

/// A file name of the recovery folder that is safe as a key (the engine's ids and extensions).
fn valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 100 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') && !name.starts_with('.')
}

impl Store {
    /// Put the stored files back into the recovery folder.
    pub fn load() -> Store {
        let mut store = Store::default();
        let s = match local_storage() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("autosaves: {e}");
                return store;
            }
        };
        let keys: Vec<String> = (0..s.length().unwrap_or(0)).filter_map(|i| s.key(i).ok().flatten()).collect();
        for key in keys {
            let Some(name) = key.strip_prefix(PREFIX).filter(|n| valid(n)) else { continue };
            let Some(text) = s.get_item(&key).ok().flatten().filter(|t| t.len() <= MAX_STORED_CHARS) else { continue };
            let Ok(bytes) = STANDARD.decode(text.as_bytes()).map_err(|e| log::warn!("autosave {name} is damaged: {e}")) else { continue };
            match fieldcraft_io::fs::insert(&dir().join(name), bytes) {
                Ok(()) => {
                    store.known.insert(name.to_string());
                }
                Err(e) => log::warn!("autosave {name}: {e}"),
            }
        }
        store
    }

    /// Keep a file the engine wrote in the recovery folder.
    pub fn put(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        let name =
            path.file_name().and_then(|n| n.to_str()).filter(|n| valid(n)).ok_or_else(|| format!("{}: not a recovery file", path.display()))?;
        if bytes.len() > MAX_AUTOSAVE_BYTES {
            return Err(format!("the model ({} MB) is too large for the browser's storage", bytes.len() >> 20));
        }
        self.known.insert(name.to_string());
        local_storage()?
            .set_item(&format!("{PREFIX}{name}"), &STANDARD.encode(bytes))
            .map_err(|e| format!("browser storage is full or turned off ({})", crate::web::js_err(e)))
    }

    /// Drop the stored files whose entries left the recovery folder (cleared once saved,
    /// recovered and discarded). Only files this page knows of: another tab's stay.
    pub fn sync_removals(&mut self) {
        let present: BTreeSet<String> =
            fieldcraft_io::fs::list(dir()).iter().filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string)).collect();
        let gone: Vec<String> = self.known.difference(&present).cloned().collect();
        if gone.is_empty() {
            return;
        }
        let Ok(s) = local_storage() else { return };
        for name in gone {
            if s.remove_item(&format!("{PREFIX}{name}")).is_ok() {
                self.known.remove(&name);
            }
        }
    }
}
