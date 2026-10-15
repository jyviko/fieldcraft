//! `fieldcraft-cli update --check` against a local release server (FIELDCRAFT_UPDATE_HOST).

use std::process::Command;

use fieldcraft_update::mock::Server;
use serde_json::{Value, json};

fn cli(host: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fieldcraft-cli")).args(args).env("FIELDCRAFT_UPDATE_HOST", host).output().unwrap()
}

#[test]
fn update_check_reports_the_latest_release() {
    let server = Server::start().unwrap();
    server.release("latest", &json!({"tag_name": "v99.0.0", "html_url": "https://github.com/jyviko/fieldcraft/releases/tag/v99.0.0", "assets": []}));
    let out = cli(&server.url(), &["update", "--check"]);
    assert!(out.status.success(), "{out:?}");
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!((report["latest"].as_str(), report["newer"].as_bool(), report["signed"].as_bool()), (Some("99.0.0"), Some(true), Some(false)));
    // A test build is not a release package: it checks but would not install.
    assert_eq!(report["asset"], Value::Null);
    assert_eq!(server.requests(), 1);

    // The same version: not newer, and --install has nothing to do.
    server.release("latest", &json!({"tag_name": format!("v{}", env!("CARGO_PKG_VERSION")), "assets": []}));
    let out = cli(&server.url(), &["update", "--check"]);
    assert_eq!(serde_json::from_slice::<Value>(&out.stdout).unwrap()["newer"], false);
    let out = cli(&server.url(), &["update", "--install"]);
    assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("is up to date"), "{out:?}");

    // Unreachable: an error, not a crash.
    let out = cli("http://127.0.0.1:1", &["update", "--check"]);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("could not reach"), "{out:?}");
    assert!(!cli(&server.url(), &["update"]).status.success());
}
