//! `cargo xtask skill-results [--part NAME] [--no-write]`: the meshing skill on every corpus part
//! and sample (the testkit's `skill` binary, which writes docs/meshing/skill-results.md), then
//! each part's section picture drawn by `fieldcraft-cli snapshot` from the commands that meshed
//! it (docs/meshing/skill/<part>.png).

use std::path::Path;

pub fn run(root: &Path, rest: &[&str]) -> Result<(), String> {
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), Into::into);
    let work = target.join("skill-results");
    let work_arg = work.to_string_lossy().into_owned();
    let mut c = crate::cargo();
    c.args(["run", "--release", "-p", "fieldcraft-testkit", "--bin", "skill", "--", "--work", &work_arg]).args(rest);
    crate::run(c, "skill")?;
    if rest.contains(&"--no-write") {
        return Ok(());
    }
    let mut c = crate::cargo();
    c.args(["build", "--release", "-p", "fieldcraft-cli"]);
    crate::run(c, "build fieldcraft-cli")?;
    let cli = target.join("release").join(if cfg!(windows) { "fieldcraft-cli.exe" } else { "fieldcraft-cli" });
    let list = work.join("pictures.json");
    let text = std::fs::read_to_string(&list).map_err(|e| format!("{}: {e}", list.display()))?;
    let pictures: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", list.display()))?;
    let dir = root.join("docs/meshing/skill");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for p in &pictures {
        let (Some(script), Some(out), Some(section)) = (p["script"].as_str(), p["out"].as_str(), p["section"].as_str()) else {
            return Err(format!("{}: bad entry {p}", list.display()));
        };
        let out = root.join(out);
        let mut c = std::process::Command::new(&cli);
        c.args(["snapshot", script, "--size", "960x600", "--section", section, "--out"]).arg(&out);
        crate::run(c, &format!("snapshot {}", out.display()))?;
    }
    Ok(())
}
