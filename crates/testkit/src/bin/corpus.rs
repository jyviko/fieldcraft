//! `corpus [--part NAME] [--no-write]`: see [`modelcraft_testkit::runner`]; run it as
//! `cargo xtask corpus`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut only = None;
    let mut write = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--part" => only = it.next().map(String::as_str),
            "--no-write" => write = false,
            other => {
                eprintln!("error: unknown argument `{other}`\nusage: corpus [--part NAME] [--no-write]");
                return ExitCode::FAILURE;
            }
        }
    }
    let root = modelcraft_testkit::corpus::corpus_dir();
    let root = root.ancestors().nth(2).unwrap_or(&root);
    match modelcraft_testkit::runner::run(root, only, write) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
