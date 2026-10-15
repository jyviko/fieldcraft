//! The log file: what the desktop app logs (through the `log` facade) goes to
//! `<data folder>/logs/fieldcraft.log`, one line per record:
//! `2026-10-14T10:03:22Z WARN [4242] fieldcraft_engine: message`. Warnings and errors also go to
//! the standard error stream.
//!
//! Bounded: past [`MAX_BYTES`] the file becomes `fieldcraft.log.1` (replacing the one before) and
//! a new one starts, so the logs never take more than twice that. A record is cut at
//! [`MAX_LINE`] bytes. The process id on every line lets a crash report take only the lines of
//! the run that crashed ([`tail`]), even when two apps share the file.
//!
//! Nothing is installed in tests, the CLI or the web build (which logs to the browser console).

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The log file's name.
pub const FILE_NAME: &str = "fieldcraft.log";
/// The log file's size limit.
pub const MAX_BYTES: u64 = 2 << 20;
/// A record's size limit.
pub const MAX_LINE: usize = 4 << 10;

struct FileLogger {
    path: PathBuf,
    level: log::LevelFilter,
    out: Mutex<Out>,
}

struct Out {
    file: Option<File>,
    bytes: u64,
}

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

/// Send the `log` records at `level` and above to the log file in `dir` (created if needed).
/// Returns the file's path. Call once; later calls change nothing.
pub fn install(dir: &Path, level: log::LevelFilter) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(FILE_NAME);
    let file = open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = file.metadata().map_or(0, |m| m.len());
    let logger = FileLogger { path: path.clone(), level, out: Mutex::new(Out { file: Some(file), bytes }) };
    if LOGGER.set(logger).is_err() {
        return Err("a log file is already installed".into());
    }
    if let Some(l) = LOGGER.get() {
        log::set_logger(l).map_err(|e| e.to_string())?;
        log::set_max_level(level);
    }
    Ok(path)
}

fn open(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// The log file, once [`install`]ed.
pub fn path() -> Option<&'static Path> {
    LOGGER.get().map(|l| l.path.as_path())
}

/// The last `max` bytes of the log file's lines written by process `pid` (every line when
/// `None`), from the rotated file too when the current one is short. `None` without a log file.
pub fn tail(pid: Option<u32>, max: usize) -> Option<String> {
    let path = path().map(Path::to_path_buf).or_else(default_path)?;
    Some(tail_of(&path, pid, max))
}

/// [`tail`] of the log file at `path`.
fn tail_of(path: &Path, pid: Option<u32>, max: usize) -> String {
    let mut text = String::new();
    for p in [rotated(path), path.to_path_buf()] {
        // The whole file (it is bounded): another process's lines may come between.
        text.push_str(&read_end(&p, MAX_BYTES));
    }
    let mark = pid.map(|p| format!(" [{p}] "));
    let mut lines: Vec<&str> = text.lines().filter(|l| mark.as_deref().is_none_or(|m| l.contains(m))).collect();
    let mut total = 0;
    let mut keep = lines.len();
    for (i, l) in lines.iter().enumerate().rev() {
        total += l.len() + 1;
        if total > max {
            break;
        }
        keep = i;
    }
    lines.drain(..keep);
    lines.join("\n")
}

/// The log file of this computer's app (for a crash report written before [`install`]).
fn default_path() -> Option<PathBuf> {
    fieldcraft_io::recovery::data_dir().map(|d| d.join("logs").join(FILE_NAME))
}

/// The file the log rotates into.
pub fn rotated(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".1");
    PathBuf::from(s)
}

/// The last `max` bytes of a file, from the start of a line.
fn read_end(path: &Path, max: u64) -> String {
    let Ok(mut f) = File::open(path) else { return String::new() };
    let len = f.metadata().map_or(0, |m| m.len());
    let from = len.saturating_sub(max);
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(from)).is_err() || f.take(max).read_to_end(&mut buf).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    match (from > 0, text.find('\n')) {
        (true, Some(i)) => text.get(i + 1..).unwrap_or_default().to_string(),
        _ => text,
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = crate::crash::limited(&record.args().to_string(), MAX_LINE).replace('\n', "\n    ");
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let line = format!("{} {:<5} [{}] {}: {message}\n", crate::crash::utc(now), record.level(), std::process::id(), record.target());
        if record.level() <= log::Level::Warn {
            eprint!("{line}");
        }
        let Ok(mut out) = self.out.lock() else { return };
        if out.bytes + line.len() as u64 > MAX_BYTES {
            out.file = None;
            let _ = fs::rename(&self.path, rotated(&self.path));
            out.file = open(&self.path).ok();
            out.bytes = 0;
        }
        if let Some(f) = &mut out.file
            && f.write_all(line.as_bytes()).is_err()
        {
            out.file = None;
        }
        out.bytes += line.len() as u64;
    }

    fn flush(&self) {
        if let Ok(mut out) = self.out.lock()
            && let Some(f) = &mut out.file
        {
            let _ = f.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_one_process_and_the_end() {
        let d = std::env::temp_dir().join(format!("fieldcraft-logtail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let p = d.join(FILE_NAME);
        fs::write(rotated(&p), "t INFO [7] a: old seven\nt INFO [8] a: old eight\n").unwrap();
        fs::write(&p, "t INFO [7] a: one\nt WARN [8] a: other\nt INFO [7] a: two\n").unwrap();
        assert_eq!(tail_of(&p, Some(7), 1000), "t INFO [7] a: old seven\nt INFO [7] a: one\nt INFO [7] a: two");
        assert_eq!(tail_of(&p, Some(7), 40), "t INFO [7] a: one\nt INFO [7] a: two");
        assert_eq!(tail_of(&p, None, 1000).lines().count(), 5);
        // From the middle of a line, the partial line is dropped.
        assert_eq!(read_end(&p, 20), "t INFO [7] a: two\n");
        let _ = fs::remove_dir_all(&d);
    }
}
