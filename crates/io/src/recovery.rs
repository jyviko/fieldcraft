//! Autosave storage and crash recovery.
//!
//! A running app keeps one entry in the recovery folder: `<id>.modelcraft` (the model as of the
//! last autosave), `<id>.json` (its [`Entry`]: the model's own file, when it has one, and when it
//! was autosaved) and `<id>.lock`, which the app holds locked while it runs. An entry whose lock
//! nobody holds belongs to an app that crashed or closed with unsaved changes: [`orphans`] lists
//! it for the next launch to offer.
//!
//! Adapted from SolidCraft's `crates/engine/src/recovery.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md. The engine decides when to autosave; this module only stores.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs::{read_capped, write_atomic};
use crate::{EXTENSION, Error, MAX_FILE_BYTES, Result};

/// The recovery folder: `MODELCRAFT_RECOVERY_DIR`, else `ModelCraft/recovery` (`modelcraft/` on
/// Linux) in the platform's per-user data folder.
pub fn default_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|d| !d.is_empty()).map(PathBuf::from);
    if let Some(d) = env("MODELCRAFT_RECOVERY_DIR") {
        return Some(d);
    }
    let (root, name) = if cfg!(windows) {
        (env("LOCALAPPDATA")?, "ModelCraft")
    } else if cfg!(target_os = "macos") {
        (env("HOME")?.join("Library/Application Support"), "ModelCraft")
    } else {
        (env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))?, "modelcraft")
    };
    Some(root.join(name).join("recovery"))
}

/// What an entry's `<id>.json` holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    /// The model's own file, if it had one.
    pub path: Option<String>,
    /// When it was autosaved (seconds since 1970).
    pub saved_at: u64,
    pub nodes: usize,
    pub elements: usize,
}

/// Seconds since 1970 (0 where the platform has no clock, such as the browser's plain wasm).
fn unix_now() -> u64 {
    if cfg!(target_arch = "wasm32") {
        return 0;
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn file(dir: &Path, id: &str, ext: &str) -> PathBuf {
    dir.join(format!("{id}.{ext}"))
}

/// Whether a running app holds entry `id`'s lock.
fn held(dir: &Path, id: &str) -> bool {
    File::open(file(dir, id, "lock")).is_ok_and(|l| l.try_lock().is_err())
}

fn missing(id: &str) -> Error {
    Error::Io(format!("no recovered model `{}`", id.chars().take(64).collect::<String>()))
}

/// Largest entry description read.
const MAX_ENTRY_BYTES: u64 = 64 << 10;

/// Most entries listed.
const MAX_LISTED: usize = 1000;

fn read_entry(path: &Path) -> Option<Entry> {
    serde_json::from_slice(&read_capped(path, MAX_ENTRY_BYTES).ok()?).ok()
}

/// Entries left by apps that are no longer running, newest first.
pub fn orphans(dir: &Path) -> Vec<Entry> {
    let mut out: Vec<Entry> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                return None;
            }
            let entry = read_entry(&p)?;
            if !valid_id(&entry.id) || p.file_stem().and_then(|s| s.to_str()) != Some(entry.id.as_str()) || !file(dir, &entry.id, EXTENSION).is_file()
            {
                return None;
            }
            // Held by a running app: not an orphan.
            (!held(dir, &entry.id)).then_some(entry)
        })
        .take(MAX_LISTED)
        .collect();
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then_with(|| a.id.cmp(&b.id)));
    out
}

/// An entry's autosaved file bytes and its description.
pub fn load(dir: &Path, id: &str) -> Result<(Vec<u8>, Entry)> {
    if !valid_id(id) {
        return Err(missing(id));
    }
    let entry = read_entry(&file(dir, id, "json")).filter(|e| e.id == id).ok_or_else(|| missing(id))?;
    let bytes = read_capped(&file(dir, id, EXTENSION), MAX_FILE_BYTES)?;
    Ok((bytes, entry))
}

/// Delete an entry (unless a running app holds it).
pub fn remove(dir: &Path, id: &str) -> Result<()> {
    if !valid_id(id) {
        return Err(missing(id));
    }
    if held(dir, id) {
        return Err(Error::Io(format!("recovered model `{id}` belongs to a running app")));
    }
    for ext in ["json", EXTENSION, "lock"] {
        let _ = fs::remove_file(file(dir, id, ext));
    }
    Ok(())
}

/// One app's entry in the recovery folder, locked while the app runs.
#[derive(Debug)]
pub struct Slot {
    dir: PathBuf,
    id: String,
    lock: Option<File>,
    has_entry: bool,
}

impl Slot {
    /// Take a new entry in `dir` (created if needed), holding its lock.
    pub fn new(dir: &Path) -> Result<Slot> {
        let io = |e: std::io::Error| Error::Io(format!("{}: {e}", dir.display()));
        fs::create_dir_all(dir).map_err(io)?;
        // Locks of apps that ended with nothing to recover.
        for e in fs::read_dir(dir).into_iter().flatten().flatten().take(10 * MAX_LISTED) {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("lock")
                && !p.with_extension("json").exists()
                && File::open(&p).is_ok_and(|l| l.try_lock().is_ok())
            {
                let _ = fs::remove_file(&p);
            }
        }
        // Unique within the process too: an app started again in the same second (tests that
        // "crash" and restart) must not take over the crashed app's entry.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("{}-{}-{seq}", unix_now(), std::process::id());
        let lock = OpenOptions::new().create(true).truncate(false).write(true).open(file(dir, &id, "lock")).map_err(io)?;
        lock.lock().map_err(io)?;
        Ok(Slot { dir: dir.to_path_buf(), id, lock: Some(lock), has_entry: false })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether the slot holds an autosave.
    pub fn has_entry(&self) -> bool {
        self.has_entry
    }

    /// Store an autosave: the file's bytes, then its description (so a listed entry always has
    /// its file).
    pub fn write(&mut self, bytes: &[u8], path: Option<&str>, nodes: usize, elements: usize) -> Result<()> {
        write_atomic(&file(&self.dir, &self.id, EXTENSION), bytes, false)?;
        let entry = Entry { id: self.id.clone(), path: path.map(str::to_string), saved_at: unix_now(), nodes, elements };
        let meta = serde_json::to_vec_pretty(&entry).map_err(|e| Error::Io(e.to_string()))?;
        write_atomic(&file(&self.dir, &self.id, "json"), &meta, false)?;
        self.has_entry = true;
        Ok(())
    }

    /// Drop the autosave (nothing is unsaved any more).
    pub fn clear(&mut self) {
        if self.has_entry {
            for ext in ["json", EXTENSION] {
                let _ = fs::remove_file(file(&self.dir, &self.id, ext));
            }
            self.has_entry = false;
        }
    }

    /// The app is closing: the lock is released; with no autosave the lock file goes too, while
    /// an autosave stays for the next launch to offer.
    pub fn close(mut self) {
        self.lock = None;
        if !self.has_entry {
            let _ = fs::remove_file(file(&self.dir, &self.id, "lock"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modelcraft-recovery-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_crashed_apps_autosave_is_offered_and_a_running_ones_is_not() {
        let d = dir("crash");
        let mut a = Slot::new(&d).unwrap();
        a.write(b"model bytes", Some("/m/part.modelcraft"), 8, 1).unwrap();
        assert!(orphans(&d).is_empty(), "held by a running app");
        assert!(remove(&d, a.id()).is_err(), "a running app's entry stays");
        let id = a.id().to_string();
        // The app dies without closing (the lock goes with the process).
        drop(a);
        let found = orphans(&d);
        assert_eq!(found.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), [id.as_str()]);
        let (bytes, entry) = load(&d, &id).unwrap();
        assert_eq!(bytes, b"model bytes");
        assert_eq!((entry.path.as_deref(), entry.nodes, entry.elements), (Some("/m/part.modelcraft"), 8, 1));
        // A new app gets its own entry and still sees the crashed one.
        let b = Slot::new(&d).unwrap();
        assert_ne!(b.id(), id);
        assert_eq!(orphans(&d).len(), 1);
        remove(&d, &id).unwrap();
        assert!(orphans(&d).is_empty());
        b.close();
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0, "a clean close with nothing unsaved leaves nothing");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn clearing_drops_the_autosave_and_closing_with_one_keeps_it() {
        let d = dir("clear");
        let mut a = Slot::new(&d).unwrap();
        a.write(b"x", None, 0, 0).unwrap();
        a.clear();
        assert!(!a.has_entry() && !file(&d, a.id(), EXTENSION).exists());
        a.write(b"y", None, 0, 0).unwrap();
        a.close();
        assert_eq!(orphans(&d).len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hostile_ids_and_entries_are_refused() {
        let d = dir("hostile");
        fs::create_dir_all(&d).unwrap();
        for id in ["", "../etc/passwd", "a/b", &"x".repeat(65), "a.b"] {
            assert!(load(&d, id).is_err(), "{id}");
            assert!(remove(&d, id).is_err(), "{id}");
        }
        // An entry naming another id, junk JSON and a huge description are not listed.
        fs::write(d.join("1-1-1.json"), br#"{"id": "2-2-2", "path": null, "saved_at": 1, "nodes": 0, "elements": 0}"#).unwrap();
        fs::write(d.join("1-1-1.modelcraft"), b"x").unwrap();
        fs::write(d.join("2-2-2.modelcraft"), b"x").unwrap();
        fs::write(d.join("3-3-3.json"), b"{not json").unwrap();
        fs::write(d.join("4-4-4.json"), vec![b' '; (MAX_ENTRY_BYTES + 1) as usize]).unwrap();
        fs::write(d.join("4-4-4.modelcraft"), b"x").unwrap();
        assert!(orphans(&d).is_empty());
        assert!(load(&d, "1-1-1").is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
