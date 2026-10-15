//! `cargo xtask affected --base <rev>`: which workspace crates a change touches, and which CI
//! jobs it needs (docs/releasing.md, "What CI runs when").
//!
//! A changed file belongs to the package whose directory holds it (from `cargo metadata`). A
//! file outside every package (an oracle case under `tests/`, an icon under `assets/`) belongs to
//! each package whose Rust sources name its top-level directory as a path (`"../../tests/…"`,
//! `"docs/features.md"`), the way they read it. The affected set is those packages and every
//! workspace package that depends on one of them, through any kind of dependency.
//!
//! Flags:
//! - `everything`: the workspace manifest, the lock file, the toolchain, xtask or the CI
//!   workflows changed, or a path under `crates/` or `apps/` belongs to no package: run it all.
//! - `solver`: the affected set holds the oracle's package or a package with heavy tests
//!   (`#[ignore]`d, run by `cargo xtask test --heavy`); the heavy and oracle jobs run.
//! - `wasm`: the affected set holds a crate the wasm check builds (below the apps, and the web app).
//!
//! The rules work on a small model ([`Workspace`]) so they are unit-tested without git or cargo.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::root;

/// The package whose `oracle` binary `cargo xtask oracle` runs.
pub const ORACLE_PACKAGE: &str = "fieldcraft-testkit";

/// Paths whose change runs everything: they reach every crate or the gate itself. A path matches
/// when it equals an entry or lies under an entry ending in `/`.
const EVERYTHING: &[&str] =
    &["Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml", ".cargo/", "clippy.toml", "rustfmt.toml", "xtask/", ".github/"];

/// Directories that hold only packages: a file there outside every package is a crate being
/// added, removed or moved, so the whole workspace runs.
const PACKAGE_ROOTS: &[&str] = &["crates/", "apps/"];

/// A workspace package, as the rules see it.
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    /// Its directory relative to the workspace root, with `/` separators and no trailing one.
    pub dir: String,
    /// The workspace packages it depends on (normal, dev and build).
    pub deps: Vec<String>,
    /// It has `#[ignore]`d (heavy) tests.
    pub heavy: bool,
    /// The top-level repository entries (`tests`, `assets`, `docs`, …) its Rust sources name.
    pub reads: BTreeSet<String>,
    /// The wasm check builds it.
    pub wasm: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub packages: Vec<Package>,
}

/// What a change needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Affected packages, sorted. Every package when `everything`.
    pub crates: BTreeSet<String>,
    pub everything: bool,
    pub solver: bool,
    pub wasm: bool,
    /// Why `everything` is set: the first path that set it.
    pub reason: Option<String>,
}

impl Workspace {
    fn owner(&self, file: &str) -> Option<&Package> {
        self.packages.iter().filter(|p| file.starts_with(&format!("{}/", p.dir))).max_by_key(|p| p.dir.len())
    }

    /// The plan for a list of changed paths (relative to the workspace root, `/` separators).
    pub fn plan(&self, changed: &[String]) -> Plan {
        let mut plan = Plan::default();
        let mut direct = BTreeSet::new();
        for file in changed {
            if EVERYTHING.iter().any(|e| if e.ends_with('/') { file.starts_with(e) } else { file == e }) {
                plan.everything = true;
                plan.reason.get_or_insert_with(|| file.clone());
                continue;
            }
            if let Some(p) = self.owner(file) {
                direct.insert(p.name.clone());
                continue;
            }
            if PACKAGE_ROOTS.iter().any(|r| file.starts_with(r)) {
                plan.everything = true;
                plan.reason.get_or_insert_with(|| file.clone());
                continue;
            }
            let top = file.split('/').next().unwrap_or(file);
            direct.extend(self.packages.iter().filter(|p| p.reads.contains(top)).map(|p| p.name.clone()));
        }
        plan.crates = if plan.everything { self.packages.iter().map(|p| p.name.clone()).collect() } else { self.dependents(direct) };
        let selected = |p: &&Package| plan.crates.contains(&p.name);
        plan.solver = plan.everything || self.packages.iter().filter(selected).any(|p| p.heavy || p.name == ORACLE_PACKAGE);
        plan.wasm = plan.everything || self.packages.iter().filter(selected).any(|p| p.wasm);
        plan
    }

    /// `set` and every package that depends on one in it, directly or not.
    fn dependents(&self, mut set: BTreeSet<String>) -> BTreeSet<String> {
        loop {
            let more: Vec<String> =
                self.packages.iter().filter(|p| !set.contains(&p.name) && p.deps.iter().any(|d| set.contains(d))).map(|p| p.name.clone()).collect();
            if more.is_empty() {
                return set;
            }
            set.extend(more);
        }
    }
}

/// The workspace from `cargo metadata` (the no-deps form [`crate::metadata`] returns), reading
/// each package's Rust sources for heavy tests and the top-level paths they name.
pub fn workspace(meta: &Value) -> Result<Workspace, String> {
    let root = root();
    let tops: Vec<String> = std::fs::read_dir(&root)
        .map_err(|e| format!("read {}: {e}", root.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "target" && !n.starts_with("target-"))
        .collect();
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let members: BTreeSet<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut packages = Vec::new();
    for p in pkgs {
        let name = p["name"].as_str().ok_or("metadata: package without name")?.to_owned();
        let manifest = Path::new(p["manifest_path"].as_str().ok_or("metadata: package without manifest_path")?);
        let dir = manifest.parent().ok_or("metadata: manifest without a directory")?;
        let rel = dir.strip_prefix(&root).map_err(|_| format!("{name}: {} is outside the workspace", dir.display()))?;
        let deps = p["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["name"].as_str())
            .filter(|d| members.contains(d) && *d != name)
            .map(str::to_owned)
            .collect();
        let mut sources = String::new();
        read_sources(dir, &mut sources)?;
        let reads = tops
            .iter()
            .filter(|t| [format!("\"{t}/"), format!("/{t}/"), format!("\"{t}\""), format!("/{t}\"")].iter().any(|n| sources.contains(n.as_str())))
            .cloned()
            .collect();
        packages.push(Package {
            heavy: sources.contains("#[ignore"),
            wasm: crate::wasm_crate(&name),
            name,
            dir: rel.to_string_lossy().replace('\\', "/"),
            deps,
            reads,
        });
    }
    Ok(Workspace { packages })
}

/// Append every `.rs` file under `dir` (build output and hidden directories left out).
fn read_sources(dir: &Path, out: &mut String) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read {}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(|e| format!("{}: {e}", path.display()))?;
        if kind.is_dir() && !name.starts_with('.') && name != "target" {
            read_sources(&path, out)?;
        } else if kind.is_file() && name.ends_with(".rs") {
            out.push_str(&std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?);
            out.push('\n');
        }
    }
    Ok(())
}

/// The files that differ between `base` and `head` (the working tree when `None`; renames count
/// as a deletion and an addition, so both sides count).
pub fn changed_files(base: &str, head: Option<&str>) -> Result<Vec<String>, String> {
    let git = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git").args(args).current_dir(root()).output().map_err(|e| format!("git: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
        }
    };
    for rev in std::iter::once(base).chain(head) {
        let commit = format!("{rev}^{{commit}}");
        git(&["rev-parse", "--verify", "--quiet", &commit]).map_err(|_| format!("`{rev}` is not a commit in this repository"))?;
    }
    let mut args = vec!["diff", "--name-only", "--no-renames", base];
    args.extend(head);
    let out = git(&args)?;
    Ok(out.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect())
}

/// The plan for the change since `base`.
pub fn plan_since(base: &str, head: Option<&str>) -> Result<(Plan, usize), String> {
    let changed = changed_files(base, head)?;
    let ws = workspace(&crate::metadata()?)?;
    Ok((ws.plan(&changed), changed.len()))
}

/// `cargo xtask affected --base <rev> [--head <rev>] [--github-output]`.
pub fn run(args: &[&str]) -> Result<(), String> {
    let mut base = None;
    let mut head = None;
    let mut github = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--base" => base = Some(*it.next().ok_or("--base needs a revision")?),
            "--head" => head = Some(*it.next().ok_or("--head needs a revision")?),
            "--github-output" => github = true,
            other => return Err(format!("affected: unknown argument `{other}`")),
        }
    }
    let base = base.ok_or("affected: --base <rev> is required")?;
    let (plan, files) = plan_since(base, head)?;
    print(&plan, base, files);
    if github {
        let path = std::env::var("GITHUB_OUTPUT").map_err(|_| "--github-output: GITHUB_OUTPUT is not set")?;
        let lines = format!("everything={}\nsolver={}\nwasm={}\ncrates={}\n", plan.everything, plan.solver, plan.wasm, plan.crates.len());
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(lines.as_bytes()))
            .map_err(|e| format!("write {path}: {e}"))?;
    }
    Ok(())
}

pub fn print(plan: &Plan, base: &str, files: usize) {
    println!("affected since {base}: {files} changed file(s)");
    match &plan.reason {
        Some(r) => println!("everything: {r} changed"),
        None if plan.crates.is_empty() => println!("crates: none"),
        None => println!("crates ({}): {}", plan.crates.len(), plan.crates.iter().cloned().collect::<Vec<_>>().join(" ")),
    }
    println!("flags: everything={} solver={} wasm={}", plan.everything, plan.solver, plan.wasm);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, dir: &str, deps: &[&str], heavy: bool, wasm: bool, reads: &[&str]) -> Package {
        Package {
            name: name.into(),
            dir: dir.into(),
            deps: deps.iter().map(|d| d.to_string()).collect(),
            heavy,
            reads: reads.iter().map(|r| r.to_string()).collect(),
            wasm,
        }
    }

    /// model <- solver <- engine <- ui-egui <- app; testkit (the oracle) uses engine; ui-egui reads assets.
    fn ws() -> Workspace {
        Workspace {
            packages: vec![
                pkg("model", "crates/model", &[], false, true, &[]),
                pkg("solver", "crates/solver", &["model"], true, true, &[]),
                pkg("engine", "crates/engine", &["solver", "model"], false, true, &["docs"]),
                pkg("ui-egui", "crates/ui-egui", &["engine"], false, true, &["assets"]),
                pkg("app", "apps/app", &["ui-egui"], false, false, &["assets"]),
                pkg(ORACLE_PACKAGE, "crates/testkit", &["engine"], false, false, &["tests", "docs"]),
                pkg("xtask", "xtask", &[], true, false, &[]),
            ],
        }
    }

    fn plan(files: &[&str]) -> Plan {
        ws().plan(&files.iter().map(|f| f.to_string()).collect::<Vec<_>>())
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn ui_change_reaches_only_its_dependents() {
        let p = plan(&["crates/ui-egui/src/theme.rs"]);
        assert_eq!(p.crates, set(&["ui-egui", "app"]));
        assert!(!p.solver && !p.everything);
        assert!(p.wasm);
    }

    #[test]
    fn app_only_change_skips_wasm() {
        let p = plan(&["apps/app/src/main.rs"]);
        assert_eq!(p.crates, set(&["app"]));
        assert!(!p.wasm && !p.solver);
    }

    #[test]
    fn solver_change_reaches_the_oracle() {
        let p = plan(&["crates/solver/src/heat.rs"]);
        assert_eq!(p.crates, set(&["solver", "engine", "ui-egui", "app", ORACLE_PACKAGE]));
        assert!(p.solver && p.wasm && !p.everything);
    }

    #[test]
    fn engine_change_runs_the_oracle_through_the_testkit() {
        let p = plan(&["crates/engine/src/lib.rs"]);
        assert!(p.crates.contains(ORACLE_PACKAGE));
        assert!(p.solver);
    }

    #[test]
    fn lock_file_workflow_and_xtask_run_everything() {
        for f in ["Cargo.lock", "Cargo.toml", ".github/workflows/ci.yml", "xtask/src/main.rs", "rust-toolchain.toml"] {
            let p = plan(&[f]);
            assert!(p.everything && p.solver && p.wasm, "{f}");
            assert_eq!(p.crates.len(), 7, "{f}");
            assert_eq!(p.reason.as_deref(), Some(f));
        }
    }

    #[test]
    fn a_crate_manifest_is_that_crate() {
        let p = plan(&["crates/model/Cargo.toml"]);
        assert!(!p.everything);
        assert!(p.crates.contains("model") && p.solver);
    }

    #[test]
    fn unowned_path_under_crates_runs_everything() {
        assert!(plan(&["crates/removed/src/lib.rs"]).everything);
    }

    #[test]
    fn files_outside_packages_reach_the_packages_that_read_them() {
        let p = plan(&["tests/oracle/beam/recipe.json"]);
        assert_eq!(p.crates, set(&[ORACLE_PACKAGE]));
        assert!(p.solver && !p.wasm);
        let p = plan(&["assets/icons/move.svg"]);
        assert_eq!(p.crates, set(&["ui-egui", "app"]));
        assert!(!p.solver);
    }

    #[test]
    fn prose_reaches_nothing() {
        let p = plan(&["skills/meshing/SKILL.md", "README.md"]);
        assert!(p.crates.is_empty());
        assert!(!p.everything && !p.solver && !p.wasm);
    }

    #[test]
    fn nested_directory_belongs_to_the_deepest_package() {
        let mut w = ws();
        w.packages.push(pkg("inner", "crates/engine/inner", &[], false, false, &[]));
        let p = w.plan(&["crates/engine/inner/src/lib.rs".to_string()]);
        assert_eq!(p.crates, set(&["inner"]));
    }

    #[test]
    fn no_change_is_an_empty_plan() {
        assert_eq!(plan(&[]), Plan::default());
    }
}
