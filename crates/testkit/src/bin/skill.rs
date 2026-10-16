//! `skill [--part NAME] [--no-write] --work DIR`: see [`fieldcraft_testkit::skill`]; run it as
//! `cargo xtask skill-results`, which also draws the section pictures.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: skill --work DIR [--part NAME] [--no-write]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut only, mut work, mut write) = (None, None, true);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--part" => only = it.next().map(String::as_str),
            "--work" => work = it.next().map(String::as_str),
            "--no-write" => write = false,
            other => {
                eprintln!("error: unknown argument `{other}`\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    let Some(work) = work else {
        eprintln!("error: --work is needed\n{USAGE}");
        return ExitCode::FAILURE;
    };
    match fieldcraft_testkit::skill::run(&fieldcraft_testkit::corpus::workspace_root(), Path::new(work), only, write) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
