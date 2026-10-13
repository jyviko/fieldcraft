//! modelcraft-cli commands
//! modelcraft-cli run <script.json>
//! modelcraft-cli features
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::process::ExitCode;

use modelcraft_engine::Session;
use serde_json::json;

const USAGE: &str = "\
usage:
  modelcraft-cli commands              list every command with its parameters
  modelcraft-cli run <script.json>     run a script (a JSON array of {\"command\", \"params\"})
                                       and print the model summary
  modelcraft-cli features              print docs/features.md (every capability, ready or
                                       coming soon) from the capability catalogue
";

/// Largest script accepted (bytes).
const MAX_SCRIPT_BYTES: u64 = 256 * 1024 * 1024;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("commands") => {
            for c in modelcraft_engine::commands() {
                println!("{:<24} {:<18} {}", c.id, c.label, c.params);
            }
            Ok(())
        }
        Some("features") => {
            print!("{}", modelcraft_engine::catalogue::features_page());
            Ok(())
        }
        Some("run") => args.get(1).ok_or_else(|| format!("run needs a script\n\n{USAGE}")).and_then(|p| run(p)),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: &str) -> Result<(), String> {
    let size = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?.len();
    if size > MAX_SCRIPT_BYTES {
        return Err(format!("{path}: larger than {} MB", MAX_SCRIPT_BYTES / (1024 * 1024)));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let script: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let mut session = Session::default();
    session.run_script(&script).map_err(|e| e.to_string())?;
    let info = session.execute("model.info", &json!({})).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?);
    Ok(())
}
