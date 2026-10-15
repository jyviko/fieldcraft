//! `fieldcraft-cli crashes` on a crash folder of its own (`FIELDCRAFT_CRASH_DIR`).

use std::path::{Path, PathBuf};
use std::process::Command;

use fieldcraft_engine::Session;
use serde_json::{Value, json};

const ID: &str = "20261014-101500-00000000deadbeef";

/// A replayable report: a model with two nodes and a replay that adds a third.
fn folder() -> PathBuf {
    let d = std::env::temp_dir().join(format!("fieldcraft-cli-crashes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let r = d.join(ID);
    std::fs::create_dir_all(&r).unwrap();
    let mut s = Session::default();
    for l in 1..=2 {
        s.execute("mesh.node.add", &json!({"label": l, "xyz": [l, 0, 0]})).unwrap();
    }
    std::fs::write(r.join("model.fieldcraft"), s.file_bytes(None).unwrap()).unwrap();
    let replay = json!({"commands": [
        {"command": "mesh.node.add", "params": {"label": 3, "xyz": [3, 0, 0]}},
        {"command": "file.save", "params": {"path": d.join("overwritten.fieldcraft")}},
    ]});
    std::fs::write(r.join("replay.json"), replay.to_string()).unwrap();
    let report = json!({
        "format": "fieldcraft-crash/1", "id": ID, "kind": "unclean_exit", "signature": "00000000deadbeef", "count": 2,
        "first_seen": "2026-10-14T10:15:00Z", "message": "FieldCraft ended without a clean exit while running `mesh.node.add`",
        "location": "unclean exit in mesh.node.add", "command": {"command": "mesh.node.add", "params": {}},
        "app": {"version": "9.9.9"}, "backtrace": [],
    });
    std::fs::write(r.join("report.json"), report.to_string()).unwrap();
    d
}

fn cli(dir: &Path, args: &[&str]) -> (String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_fieldcraft-cli")).args(args).env("FIELDCRAFT_CRASH_DIR", dir).output().unwrap();
    (format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)), out.status.success())
}

#[test]
fn crashes_list_show_replay_and_clear() {
    let d = folder();
    let (out, ok) = cli(&d, &["crashes"]);
    assert!(ok, "{out}");
    assert!(out.contains(ID) && out.contains("unclean_exit ×2 in mesh.node.add") && out.contains("[replayable]"), "{out}");
    let (out, ok) = cli(&d, &["crashes", "list", "--json"]);
    assert!(ok);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["reports"][0]["id"], ID);
    let (out, ok) = cli(&d, &["crashes", "show", ID]);
    assert!(ok);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(v["model_path"].as_str().unwrap().ends_with("model.fieldcraft"));
    let (out, ok) = cli(&d, &["crashes", "replay", ID]);
    assert!(ok, "{out}");
    assert!(out.contains("1. mesh.node.add: ok"), "{out}");
    // A command that writes a file is refused outside the scratch folder: the replay never
    // writes over the user's files.
    assert!(out.contains("2. file.save: ") && out.contains("outside the write root"), "{out}");
    assert!(!d.join("overwritten.fieldcraft").exists());
    assert!(out.contains("did not reproduce"), "{out}");
    let (out, ok) = cli(&d, &["crashes", "show", "../etc"]);
    assert!(!ok && out.contains("no crash report"), "{out}");
    let (out, ok) = cli(&d, &["crashes", "dir"]);
    assert!(ok && out.trim() == d.to_string_lossy(), "{out}");
    let (out, ok) = cli(&d, &["crashes", "clear"]);
    assert!(ok && out.contains("deleted 1 report(s)"), "{out}");
    let (out, _) = cli(&d, &["crashes"]);
    assert!(out.contains("(none)"), "{out}");
    let (out, ok) = cli(&d, &["crashes", "nope"]);
    assert!(!ok && out.contains("usage"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
