//! FieldCraft in the browser.
//!
//! Runs the same [`fieldcraft_ui_egui::ModelApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory (output in `dist/web`); docs/web.md says what works in the browser.
//!
//! Differences from the desktop app, all behind the UI's host services:
//! - files live in the page ([`fieldcraft_io::fs`] keeps them in memory): File › Open and drops
//!   read the computer's files into it; Save, Save As and Write Deck hand the written file out as
//!   a download;
//! - autosaves for recovery and the preferences are kept in the browser's local storage
//!   ([`storage`], [`usage`]), and anonymous usage (when agreed) goes out with `fetch`;
//! - no control channel (a page can't listen on a socket);
//! - background jobs (meshing, solving) run on the page's one thread once the window has shown
//!   that they run.
//!
//! Address flags: `?sample` or `?sample=cantilever` opens a sample, `?webgl` forces WebGL2.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod storage;
#[cfg(target_arch = "wasm32")]
mod usage;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("fieldcraft-web only runs in the browser: build it with `trunk build --release` in apps/fieldcraft-web (docs/web.md)");
}

/// The sample an address asks for (`?sample` opens the first sample, `?sample=NAME` that one).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn sample_in_query(query: &str) -> Option<String> {
    let first = fieldcraft_engine::sample::SAMPLES.first().map_or("", |s| s.name);
    query.trim_start_matches('?').split('&').find_map(|kv| {
        let (key, value) = kv.split_once('=').unwrap_or((kv, ""));
        (key == "sample").then(|| if value.is_empty() { first } else { value }.to_string())
    })
}

/// The name a file from the computer is kept under in the page: its last path part, without
/// characters a path would read as folders.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = last.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.is_empty() || clean == "." || clean == ".." { "file".to_string() } else { clean }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_from_the_address() {
        assert_eq!(sample_in_query(""), None);
        assert_eq!(sample_in_query("?sample").as_deref(), Some("parts"));
        assert_eq!(sample_in_query("?webgl&sample=cantilever").as_deref(), Some("cantilever"));
        assert_eq!(sample_in_query("?sample=").as_deref(), Some("parts"));
        assert_eq!(sample_in_query("?samples=x"), None);
    }

    #[test]
    fn file_names_stay_in_their_folder() {
        assert_eq!(safe_name("beam.fieldcraft"), "beam.fieldcraft");
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\work\\plate.solidcraft"), "plate.solidcraft");
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name("a\u{0}b.3mf"), "ab.3mf");
    }
}
