//! `corpus [--part NAME] [--no-write]`: see [`modelcraft_testkit::runner`]; run it as
//! `cargo xtask corpus`.
//!
//! `corpus --part NAME --design PATH` writes that part's `.solidcraft` design to `PATH` instead
//! (its recipe run, or its shape generated), for opening the part in the app or over MCP.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: corpus [--part NAME] [--no-write] | corpus --part NAME --design PATH";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut only = None;
    let mut design = None;
    let mut write = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--part" => only = it.next().map(String::as_str),
            "--design" => design = it.next().map(String::as_str),
            "--no-write" => write = false,
            other => {
                eprintln!("error: unknown argument `{other}`\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    let result = match (design, only) {
        (Some(path), Some(name)) => write_design(name, Path::new(path)),
        (Some(_), None) => Err(format!("--design needs --part\n{USAGE}")),
        (None, _) => modelcraft_testkit::runner::run(&modelcraft_testkit::corpus::workspace_root(), only, write),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Write the design of the corpus part `name` to `path`.
fn write_design(name: &str, path: &Path) -> Result<(), String> {
    let parts = modelcraft_testkit::corpus::parts().map_err(|e| e.to_string())?;
    let part = parts.iter().find(|p| p.name == name).ok_or_else(|| format!("no corpus part named `{name}`"))?;
    let bytes = modelcraft_testkit::corpus::design_bytes(part).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!("corpus: wrote {} ({name})", path.display());
    Ok(())
}
