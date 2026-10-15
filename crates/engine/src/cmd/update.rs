//! `app.update.check`: is a newer FieldCraft published? The same check as the desktop app's
//! Help › Check for Updates and `fieldcraft-cli update --check` (docs/updates.md). It only
//! reports: nothing is downloaded or installed, and no dialog opens.
//!
//! Checking reaches the network, so it is switched on only by the programs that may
//! ([`enable_checks`]: the desktop app and the CLI); everywhere else (tests, scratch sessions,
//! the browser) the command refuses without asking anything.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const CHECK: CommandSpec = CommandSpec {
    id: "app.update.check",
    label: "Check for Updates",
    params: r#"{} — asks GitHub Releases for the latest published FieldCraft release and compares it with this version: current, latest, newer, url (its page and release notes), published, signed (whether it can be verified and installed from the app) and asset (the file this install would download; null when it cannot update itself). Reports only: nothing is downloaded or installed. Desktop app and fieldcraft-cli only"#,
    form: Some(&[]),
    kind: CommandKind::Session,
    requires: &[],
    run: check,
};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Let `app.update.check` reach the release server (the desktop app and the CLI do).
pub fn enable_checks(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

#[cfg(target_arch = "wasm32")]
fn check(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Params("not in the browser: the web app offers Reload to update when a new version is deployed".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn check(_: &mut Session, _: &Value) -> Result<Value> {
    if !ENABLED.load(Ordering::SeqCst) {
        return Err(EngineError::Params("update checks are off in this session (the desktop app and fieldcraft-cli check for updates)".into()));
    }
    let check = fieldcraft_update::net::Client::from_env().check().map_err(|e| EngineError::Params(e.to_string()))?;
    let asset = fieldcraft_update::install::here().asset_name(&check.latest.version, std::env::consts::ARCH);
    Ok(check.to_json(asset.as_deref()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Off unless a program switches it on: tests and scratch sessions never reach the network.
    #[test]
    fn checks_are_off_unless_enabled() {
        let e = Session::default().execute("app.update.check", &json!({})).unwrap_err();
        assert!(e.to_string().contains("update checks are off"), "{e}");
    }
}
