//! Autosave storage and crash recovery.
//!
//! A running app keeps one entry in the recovery folder: `<id>.fieldcraft` (the model as of the
//! last autosave), `<id>.json` (its [`Entry`]: the model's own file, when it has one, and when it
//! was autosaved) and `<id>.lock`, which the app holds locked while it runs. An entry whose lock
//! nobody holds belongs to an app that crashed or closed with unsaved changes: [`orphans`] lists
//! it for the next launch to offer.
//!
//! In the browser the folder is [`BROWSER_DIR`] in the page's files ([`crate::memory`]), which
//! the web host keeps in the browser's storage, and an entry is held while the page that wrote it
//! is open (the page holds no file locks; another tab's entry counts as left behind).
//!
//! Adapted from SolidCraft's `crates/engine/src/recovery.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md. The engine decides when to autosave; this module only stores.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs::{create_dir_all, is_file, list, read_capped, remove as remove_file, write_atomic};
use crate::{EXTENSION, Error, MAX_FILE_BYTES, Result};

/// The recovery folder of the browser build.
pub const BROWSER_DIR: &str = "/recovery";

/// The recovery folder of automated runs (a control channel, CI): a temporary folder, so their
/// autosaves never reach the user's own recovery folder.
pub fn automation_dir() -> PathBuf {
    std::env::temp_dir().join("fieldcraft-automation").join("recovery")
}

/// Set by [`use_automation_dir`].
static AUTOMATED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// This process is driven by automation (a control channel): [`data_dir`] is the temporary
/// `fieldcraft-automation` folder from now on, so unless `FIELDCRAFT_RECOVERY_DIR` names a
/// folder, [`default_dir`] is [`automation_dir`].
pub fn use_automation_dir() {
    let _ = AUTOMATED.set(());
}

/// The recovery folder: `FIELDCRAFT_RECOVERY_DIR`, else `recovery` in [`data_dir`];
/// [`BROWSER_DIR`] in the browser.
pub fn default_dir() -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return Some(PathBuf::from(BROWSER_DIR));
    }
    if let Some(d) = std::env::var_os("FIELDCRAFT_RECOVERY_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    data_dir().map(|d| d.join("recovery"))
}

/// The app's per-user data folder, which holds the recovery, crash and log folders: the
/// platform's (`FieldCraft`, `fieldcraft` on Linux), or `fieldcraft-automation` in the temporary
/// folder in automated runs ([`use_automation_dir`], or the `CI` environment variable set).
/// `None` in the browser and where the platform names no such folder.
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    let env = |k: &str| std::env::var_os(k).filter(|d| !d.is_empty()).map(PathBuf::from);
    if AUTOMATED.get().is_some() || env("CI").is_some() {
        return automation_dir().parent().map(Path::to_path_buf);
    }
    let (root, name) = if cfg!(windows) {
        (env("LOCALAPPDATA")?, "FieldCraft")
    } else if cfg!(target_os = "macos") {
        (env("HOME")?.join("Library/Application Support"), "FieldCraft")
    } else {
        (env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))?, "fieldcraft")
    };
    Some(root.join(name))
}

/// Entries older than this many days are pruned ([`prune`]).
pub const MAX_AGE_DAYS: u64 = 14;

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

/// Milliseconds since 1970 from the host, where the platform has no clock of its own (the
/// browser's plain wasm); see [`set_clock`].
static CLOCK: std::sync::OnceLock<fn() -> u64> = std::sync::OnceLock::new();

/// Give the module a clock (milliseconds since 1970) on a platform without one: the web host
/// passes the browser's. Entries then record when they were saved and get ids no other page
/// takes. The first clock given stays.
pub fn set_clock(now_ms: fn() -> u64) {
    let _ = CLOCK.set(now_ms);
}

/// Milliseconds since 1970 (0 without a clock).
fn unix_now_ms() -> u64 {
    if cfg!(target_arch = "wasm32") {
        return CLOCK.get().map_or(0, |now| now());
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis().try_into().unwrap_or(u64::MAX)).unwrap_or(0)
}

/// Seconds since 1970 (0 without a clock).
fn unix_now() -> u64 {
    unix_now_ms() / 1000
}

impl Entry {
    /// Seconds since the autosave (0 without a clock).
    pub fn age(&self) -> u64 {
        unix_now().saturating_sub(self.saved_at)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn file(dir: &Path, id: &str, ext: &str) -> PathBuf {
    dir.join(format!("{id}.{ext}"))
}

/// Entry locks on the desktop: `<id>.lock`, locked while the app runs.
#[cfg(not(target_arch = "wasm32"))]
mod lock {
    use std::fs::{self, File, OpenOptions};
    use std::path::Path;

    use super::{MAX_LISTED, file};
    use crate::{Error, Result};

    /// A held entry lock.
    #[derive(Debug)]
    pub struct Lock(#[allow(dead_code)] File);

    /// Whether a running app holds entry `id`'s lock.
    pub fn held(dir: &Path, id: &str) -> bool {
        File::open(file(dir, id, "lock")).is_ok_and(|l| l.try_lock().is_err())
    }

    pub fn take(dir: &Path, id: &str) -> Result<Lock> {
        let io = |e: std::io::Error| Error::Io(format!("{}: {e}", dir.display()));
        let lock = OpenOptions::new().create(true).truncate(false).write(true).open(file(dir, id, "lock")).map_err(io)?;
        lock.lock().map_err(io)?;
        Ok(Lock(lock))
    }

    /// Remove the locks of apps that ended with nothing to recover.
    pub fn sweep(dir: &Path) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten().take(10 * MAX_LISTED) {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("lock")
                && !p.with_extension("json").exists()
                && File::open(&p).is_ok_and(|l| l.try_lock().is_ok())
            {
                let _ = fs::remove_file(&p);
            }
        }
    }

    /// A process-unique part of new entry ids.
    pub fn instance() -> String {
        std::process::id().to_string()
    }

    /// Let go of the lock; its file goes unless an autosave stays for the next launch.
    pub fn release(lock: Lock, dir: &Path, id: &str, keep: bool) {
        drop(lock);
        if !keep {
            let _ = fs::remove_file(file(dir, id, "lock"));
        }
    }
}

/// Entry "locks" in the browser: the entries this page holds, kept in memory (a page that is
/// gone holds none).
#[cfg(target_arch = "wasm32")]
mod lock {
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::path::Path;

    use crate::Result;

    thread_local! {
        static HELD: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    }

    /// Releases the entry when dropped (the slot is gone).
    #[derive(Debug)]
    pub struct Lock(String);

    impl Drop for Lock {
        fn drop(&mut self) {
            HELD.with(|h| h.borrow_mut().remove(&self.0));
        }
    }

    pub fn held(_dir: &Path, id: &str) -> bool {
        HELD.with(|h| h.borrow().contains(id))
    }

    pub fn take(_dir: &Path, id: &str) -> Result<Lock> {
        HELD.with(|h| h.borrow_mut().insert(id.to_string()));
        Ok(Lock(id.to_string()))
    }

    pub fn sweep(_dir: &Path) {}

    /// The page's part of new entry ids: the milliseconds of the clock (the id has its seconds),
    /// so pages started in the same second take different ids (unless in the same millisecond).
    pub fn instance() -> String {
        format!("w{}", super::unix_now_ms() % 1000)
    }

    pub fn release(lock: Lock, _dir: &Path, _id: &str, _keep: bool) {
        drop(lock);
    }
}

use lock::held;

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
    let mut out: Vec<Entry> = list(dir)
        .into_iter()
        .filter_map(|p| {
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                return None;
            }
            let entry = read_entry(&p)?;
            if !valid_id(&entry.id) || p.file_stem().and_then(|s| s.to_str()) != Some(entry.id.as_str()) || !is_file(&file(dir, &entry.id, EXTENSION))
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

/// Delete the entries nobody needs any more: those older than [`MAX_AGE_DAYS`] and those whose
/// model was saved to its own file after the autosave (the file is newer than the entry).
/// Entries held by running apps stay. Returns the ids deleted.
pub fn prune(dir: &Path) -> Vec<String> {
    prune_at(dir, unix_now())
}

fn prune_at(dir: &Path, now: u64) -> Vec<String> {
    let stale = |e: &Entry| {
        let old = now.saturating_sub(e.saved_at) > MAX_AGE_DAYS * 86_400;
        let saved_since = e.path.as_deref().and_then(|p| crate::fs::modified(Path::new(p))).is_some_and(|m| m > e.saved_at);
        old || saved_since
    };
    orphans(dir).into_iter().filter(stale).filter(|e| remove(dir, &e.id).is_ok()).map(|e| e.id).collect()
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
        let _ = remove_file(&file(dir, id, ext));
    }
    Ok(())
}

/// One app's entry in the recovery folder, locked while the app runs.
#[derive(Debug)]
pub struct Slot {
    dir: PathBuf,
    id: String,
    lock: Option<lock::Lock>,
    has_entry: bool,
}

impl Slot {
    /// Take a new entry in `dir` (created if needed), holding its lock.
    pub fn new(dir: &Path) -> Result<Slot> {
        create_dir_all(dir)?;
        lock::sweep(dir);
        prune(dir);
        // Unique within the process too: an app started again in the same second (tests that
        // "crash" and restart) must not take over the crashed app's entry.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("{}-{}-{seq}", unix_now(), lock::instance());
        let lock = lock::take(dir, &id)?;
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

    /// The autosaved model's file (there once [`Slot::write`] has written it).
    pub fn file(&self) -> PathBuf {
        file(&self.dir, &self.id, EXTENSION)
    }

    /// Drop the autosave (nothing is unsaved any more).
    pub fn clear(&mut self) {
        if self.has_entry {
            for ext in ["json", EXTENSION] {
                let _ = remove_file(&file(&self.dir, &self.id, ext));
            }
            self.has_entry = false;
        }
    }

    /// The app is closing: the lock is released; with no autosave the lock file goes too, while
    /// an autosave stays for the next launch to offer.
    pub fn close(mut self) {
        if let Some(lock) = self.lock.take() {
            lock::release(lock, &self.dir, &self.id, self.has_entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fieldcraft-recovery-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_crashed_apps_autosave_is_offered_and_a_running_ones_is_not() {
        let d = dir("crash");
        let mut a = Slot::new(&d).unwrap();
        a.write(b"model bytes", Some("/m/part.fieldcraft"), 8, 1).unwrap();
        assert!(orphans(&d).is_empty(), "held by a running app");
        assert!(remove(&d, a.id()).is_err(), "a running app's entry stays");
        let id = a.id().to_string();
        // The app dies without closing (the lock goes with the process).
        drop(a);
        let found = orphans(&d);
        assert_eq!(found.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), [id.as_str()]);
        let (bytes, entry) = load(&d, &id).unwrap();
        assert_eq!(bytes, b"model bytes");
        assert_eq!((entry.path.as_deref(), entry.nodes, entry.elements), (Some("/m/part.fieldcraft"), 8, 1));
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

    /// Write a left-behind entry saved at `saved_at` for `path`.
    fn left_behind(d: &Path, id: &str, path: Option<&Path>, saved_at: u64) {
        fs::create_dir_all(d).unwrap();
        let entry = Entry { id: id.into(), path: path.map(|p| p.to_string_lossy().to_string()), saved_at, nodes: 0, elements: 0 };
        fs::write(file(d, id, "json"), serde_json::to_vec(&entry).unwrap()).unwrap();
        fs::write(file(d, id, EXTENSION), b"x").unwrap();
    }

    #[test]
    fn old_entries_and_entries_saved_since_are_pruned() {
        let d = dir("prune");
        let now = unix_now();
        let day = 86_400;
        let model = d.join("model.fieldcraft");
        fs::create_dir_all(&d).unwrap();
        fs::write(&model, b"saved").unwrap();
        let written = crate::fs::modified(&model).unwrap();
        left_behind(&d, "1-1-1", None, now - (MAX_AGE_DAYS + 1) * day);
        left_behind(&d, "2-2-2", None, now - (MAX_AGE_DAYS - 1) * day);
        left_behind(&d, "3-3-3", Some(&model), written - 60);
        left_behind(&d, "4-4-4", Some(&model), written + 60);
        left_behind(&d, "5-5-5", Some(&d.join("gone.fieldcraft")), now);
        let mut pruned = prune_at(&d, now);
        pruned.sort();
        assert_eq!(pruned, ["1-1-1", "3-3-3"], "too old, and saved since");
        let listed = orphans(&d);
        assert!(listed.windows(2).all(|w| w[0].saved_at >= w[1].saved_at), "newest first");
        let mut kept: Vec<String> = listed.into_iter().map(|e| e.id).collect();
        kept.sort();
        assert_eq!(kept, ["2-2-2", "4-4-4", "5-5-5"]);
        // A running app's entry stays however old it is.
        let mut a = Slot::new(&d).unwrap();
        a.write(b"x", None, 0, 0).unwrap();
        assert!(!prune_at(&d, now + 365 * day).contains(&a.id().to_string()));
        // Taking a slot prunes.
        left_behind(&d, "6-6-6", None, 1);
        let _b = Slot::new(&d).unwrap();
        assert!(!file(&d, "6-6-6", "json").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn automated_runs_use_the_temporary_folder() {
        // `FIELDCRAFT_RECOVERY_DIR` wins when the test runner sets it.
        if std::env::var_os("FIELDCRAFT_RECOVERY_DIR").is_some_and(|d| !d.is_empty()) {
            return;
        }
        use_automation_dir();
        assert_eq!(default_dir(), Some(automation_dir()));
        assert!(automation_dir().starts_with(std::env::temp_dir()));
        assert_eq!(data_dir().map(|d| d.join("recovery")), Some(automation_dir()));
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
        fs::write(d.join("1-1-1.fieldcraft"), b"x").unwrap();
        fs::write(d.join("2-2-2.fieldcraft"), b"x").unwrap();
        fs::write(d.join("3-3-3.json"), b"{not json").unwrap();
        fs::write(d.join("4-4-4.json"), vec![b' '; (MAX_ENTRY_BYTES + 1) as usize]).unwrap();
        fs::write(d.join("4-4-4.fieldcraft"), b"x").unwrap();
        assert!(orphans(&d).is_empty());
        assert!(load(&d, "1-1-1").is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
