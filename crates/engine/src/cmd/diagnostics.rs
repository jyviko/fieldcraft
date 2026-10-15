//! Crash reports and bug report drafts (docs/crash-reports.md): list and read the reports the
//! desktop app wrote, draft a bug report, and (with `FIELDCRAFT_CRASH_TEST=1`) crash on purpose
//! to check that reporting works.

use std::path::PathBuf;

use fieldcraft_io::roots::Access;
use serde_json::{Value, json};

use super::{file_param, opt, req};
use crate::{CommandKind, CommandSpec, EngineError, Result, Session, bug_report, crash};

pub const CRASH_REPORTS: CommandSpec = CommandSpec {
    id: "diagnostics.crash_reports",
    label: "Crash Reports",
    params: r#"{"limit"?: 20, "dir"?: "…"} — the crash reports the desktop app wrote on this computer, newest first: id, kind (command_panic, recovered_panic, panic, unclean_exit), time, message, command, count, version, path and whether it can be replayed (default folder: FIELDCRAFT_CRASH_DIR, a temporary folder in runs driven by automation, else the per-user data folder)"#,
    form: Some(super::forms::CRASH_REPORTS),
    kind: CommandKind::Session,
    requires: &[],
    run: crash_reports,
};

pub const CRASH_REPORT: CommandSpec = CommandSpec {
    id: "diagnostics.crash_report",
    label: "Crash Report",
    params: r#"{"id": "20261014-101500-…", "dir"?: "…"} — one crash report: what happened, where, the backtrace, the command and recent commands, and the paths of its model, replay and log files"#,
    form: Some(super::forms::CRASH_REPORT),
    kind: CommandKind::Session,
    requires: &[],
    run: crash_report,
};

pub const BUG_REPORT: CommandSpec = CommandSpec {
    id: "diagnostics.bug_report",
    label: "Report a Bug",
    params: r#"{"title"?: "…", "observed"?: "what happened", "crash"?: "<crash report id>", "share_session"?: false, "include_model"?: false} — drafts a GitHub bug report: the issue link (the bug form filled with the version, the OS, what happened and, for a crash, where), and the files to attach. Only with share_session does it carry the model's commands, and only with include_model as well the model file. Nothing is sent: the desktop app's Help › Report a Bug shows the draft and opens the link when the user asks"#,
    form: Some(super::forms::BUG_REPORT),
    kind: CommandKind::Session,
    requires: &[],
    run: bug_report_draft,
};

pub const CRASH_TEST: CommandSpec = CommandSpec {
    id: "diagnostics.crash_test",
    label: "Crash Test",
    params: r#"{"mode": "panic|recovered|abort"} — crashes on purpose to check that crash reports work (only with FIELDCRAFT_CRASH_TEST=1): panic, an internal error in this command; recovered, one caught inside it; abort, the app ends at once (reported at its next start)"#,
    form: Some(super::forms::CRASH_TEST),
    kind: CommandKind::Session,
    requires: &[],
    run: crash_test,
};

fn crash_dir(s: &Session, p: &Value) -> Result<PathBuf> {
    if p.get("dir").is_some() {
        return Ok(PathBuf::from(file_param(s, p, "dir", Access::Read)?));
    }
    crash::default_dir().ok_or_else(|| EngineError::Params("no crash folder: set FIELDCRAFT_CRASH_DIR or give `dir`".into()))
}

fn crash_reports(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = crash_dir(s, p)?;
    let limit = opt::<usize>(p, "limit")?.unwrap_or(crash::MAX_REPORTS);
    let mut reports = crash::list(&dir);
    reports.truncate(limit);
    Ok(json!({"dir": dir.to_string_lossy(), "reports": reports}))
}

fn crash_report(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = crash_dir(s, p)?;
    let id = req::<String>(p, "id")?;
    crash::read(&dir, &id).map_err(EngineError::Params)
}

/// The draft input for `p` from the session: its commands and model (when shared), or a crash
/// report's replay and model.
pub fn bug_report_input(s: &Session, p: &Value) -> Result<bug_report::Input> {
    let share = opt::<bool>(p, "share_session")?.unwrap_or(false);
    let include_model = opt::<bool>(p, "include_model")?.unwrap_or(false);
    let mut input = bug_report::Input {
        title: opt::<String>(p, "title")?.unwrap_or_default(),
        observed: opt::<String>(p, "observed")?.unwrap_or_default(),
        share_session: share,
        include_model,
        ..Default::default()
    };
    match opt::<String>(p, "crash")? {
        Some(id) => {
            let report = crash::read(&crash_dir(s, p)?, &id).map_err(EngineError::Params)?;
            if share {
                let read = |k: &str| {
                    report
                        .get(k)
                        .and_then(Value::as_str)
                        .and_then(|f| fieldcraft_io::fs::read_capped(std::path::Path::new(f), crash::MAX_MODEL_BYTES as u64).ok())
                };
                let replay: Value = read("replay_path").and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
                input.commands = replay.get("commands").and_then(Value::as_array).cloned().unwrap_or_default();
                input.model = if include_model { read("model_path") } else { None };
            }
            input.crash = Some(report);
        }
        None if share => {
            input.commands = s.journal.iter().map(|(command, params)| json!({"command": command, "params": params})).collect();
            input.model = if include_model { Some(s.file_bytes(None)?) } else { None };
        }
        None => {}
    }
    Ok(input)
}

fn bug_report_draft(s: &mut Session, p: &Value) -> Result<Value> {
    let input = bug_report_input(s, p)?;
    Ok(bug_report::draft(&input, &crash::Build::current().version_line()).to_json())
}

// The one deliberate panic and abort: they prove that crash reports work, and run only when
// asked for with FIELDCRAFT_CRASH_TEST=1.
#[allow(clippy::panic)]
fn crash_test(_: &mut Session, p: &Value) -> Result<Value> {
    let mode = req::<String>(p, "mode")?;
    if std::env::var_os("FIELDCRAFT_CRASH_TEST").is_none_or(|v| v != "1") {
        return Err(EngineError::Params("the crash test runs only with FIELDCRAFT_CRASH_TEST=1".into()));
    }
    match mode.as_str() {
        "panic" => std::panic::panic_any(String::from("crash test: an internal error in a command")),
        "recovered" => {
            let caught = std::panic::catch_unwind(|| std::panic::panic_any(String::from("crash test: an internal error caught inside a command")));
            Ok(json!({"caught": caught.is_err()}))
        }
        "abort" => std::process::abort(),
        _ => Err(EngineError::invalid("mode", "must be panic, recovered or abort")),
    }
}
