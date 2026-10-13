//! Dependency layering rules (plan/architecture.md §3).
//!
//! Adapted from SolidCraft's xtask (MIT OR Apache-2.0), see ATTRIBUTION.md.
//!
//! The rule engine works on a small, metadata-independent model so it can be
//! unit-tested; `from_metadata` builds that model from `cargo metadata`.

use serde_json::Value;

/// Where a workspace crate sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Regular layered crate.
    Layer(u8),
    /// Layer 0, and additionally may depend on no workspace crate at all.
    Standalone,
    /// Test tooling: may depend on anything up to L7 (it sits at [`TESTKIT_LAYER`]
    /// for rule purposes); other crates may use it only as a dev-dependency.
    Testkit,
    /// Binaries and build tooling: exempt from the rules.
    Exempt,
}

impl Class {
    fn layer(self) -> Option<u8> {
        match self {
            Class::Layer(l) => Some(l),
            Class::Standalone => Some(0),
            Class::Testkit => Some(TESTKIT_LAYER),
            Class::Exempt => None,
        }
    }
}

/// The layering table. Names are package names without the `modelcraft-`
/// prefix.
pub const TABLE: &[(&str, Class)] = &[
    // L0: the solver-neutral model.
    ("model", Class::Layer(0)),
    // L1: geometry and math kernels.
    ("geometry", Class::Layer(1)),
    ("mesh", Class::Layer(1)),
    ("fem", Class::Layer(1)),
    ("decks", Class::Layer(1)),
    ("render", Class::Layer(1)),
    // L2: solvers (the explicit and lattice Boltzmann solvers join here).
    ("solver", Class::Layer(2)),
    // L3: coupling between solvers (`fsi`, later).
    // L4 and up: the document, files, commands, UI.
    ("doc", Class::Layer(4)),
    ("io", Class::Layer(5)),
    ("engine", Class::Layer(6)),
    ("ui-egui", Class::Layer(7)),
    ("mcp", Class::Layer(7)),
    ("testkit", Class::Testkit),
    // apps and tooling
    ("modelcraft", Class::Exempt),
    ("cli", Class::Exempt),
    ("web", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// Explicit orderings *within* a layer (earlier may be used by later).
pub const INTRA_LAYER_ORDER: &[&[&str]] = &[];

fn intra_layer_allowed(from: &str, to: &str) -> bool {
    let (from, to) = (short_name(from), short_name(to));
    INTRA_LAYER_ORDER.iter().any(|chain| match (chain.iter().position(|n| *n == from), chain.iter().position(|n| *n == to)) {
        (Some(f), Some(t)) => t < f,
        _ => false,
    })
}

/// External crates that constitute a UI toolkit / windowing dependency.
/// Entries ending in `*` are prefixes.
pub const UI_CRATES: &[&str] = &["egui", "eframe", "egui-wgpu", "wgpu", "winit", "egui_kittest", "rfd", "bevy*"];

/// The only crate that may depend on SolidCraft (`solidcraft-*`): the geometry boundary.
pub const SOLIDCRAFT_GATEWAY: &str = "geometry";

/// First layer allowed to use UI crates.
pub const UI_MIN_LAYER: u8 = 7;

/// The layer the testkit sits at for rule purposes: above every layered crate.
pub const TESTKIT_LAYER: u8 = UI_MIN_LAYER + 1;

/// The layer reported for apps and tooling when a layered crate depends on one.
const APPS_LAYER: u8 = TESTKIT_LAYER + 1;

pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("modelcraft-").unwrap_or(pkg)
}

pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

fn is_ui_crate(name: &str) -> bool {
    UI_CRATES.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    /// `true` if the dependency is a workspace member.
    pub workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, dep: String, from: u8, to: u8, kind: DepKind },
    StandaloneHasWorkspaceDep { krate: String, dep: String },
    TestkitAsNormalDep { krate: String },
    UiBelowUiLayer { krate: String, dep: String, layer: u8 },
    SolidCraftOutsideGeometry { krate: String, dep: String },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE (see plan/architecture.md §3)")
            }
            Violation::Upward { krate, dep, from, to, kind } => {
                write!(f, "{krate} (L{from}) -> {dep} (L{to}) [{kind:?}]: may only depend on strictly lower layers")
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep } => {
                write!(f, "{krate}: standalone crate must not depend on workspace crate {dep}")
            }
            Violation::TestkitAsNormalDep { krate } => {
                write!(f, "{krate}: modelcraft-testkit may only be a dev-dependency")
            }
            Violation::UiBelowUiLayer { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on UI crate `{dep}`; UI toolkits are only allowed in L{UI_MIN_LAYER}+")
            }
            Violation::SolidCraftOutsideGeometry { krate, dep } => {
                write!(f, "{krate} depends on `{dep}`; only modelcraft-{SOLIDCRAFT_GATEWAY} may use SolidCraft")
            }
        }
    }
}

/// Check all rules. Returns violations sorted for stable output.
pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let Some(class) = classify(&c.name) else {
            out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        if class == Class::Exempt {
            continue;
        }
        let layer = class.layer().unwrap_or(0);
        for d in &c.deps {
            // Self dev-dependencies (e.g. to enable features in tests) are fine.
            if d.name == c.name {
                continue;
            }
            if d.workspace {
                if class == Class::Standalone {
                    out.push(Violation::StandaloneHasWorkspaceDep { krate: c.name.clone(), dep: d.name.clone() });
                    continue;
                }
                match classify(&d.name) {
                    // Unregistered deps are reported on their own entry.
                    None => {}
                    Some(Class::Testkit) if d.kind != DepKind::Dev => {
                        out.push(Violation::TestkitAsNormalDep { krate: c.name.clone() });
                    }
                    Some(Class::Testkit) => {}
                    Some(dc) => {
                        let to = dc.layer().unwrap_or(u8::MAX);
                        if to >= layer && !(to == layer && intra_layer_allowed(&c.name, &d.name)) {
                            out.push(Violation::Upward {
                                krate: c.name.clone(),
                                dep: d.name.clone(),
                                from: layer,
                                to: to.min(APPS_LAYER),
                                kind: d.kind,
                            });
                        }
                    }
                }
            } else if d.name.starts_with("solidcraft-") && short_name(&c.name) != SOLIDCRAFT_GATEWAY {
                out.push(Violation::SolidCraftOutsideGeometry { krate: c.name.clone(), dep: d.name.clone() });
            } else if layer < UI_MIN_LAYER && is_ui_crate(&d.name) {
                out.push(Violation::UiBelowUiLayer { krate: c.name.clone(), dep: d.name.clone(), layer });
            }
        }
    }
    out.sort_by_key(|v| v.to_string());
    out.dedup();
    out
}

/// Build the model from `cargo metadata --format-version 1 --no-deps`.
pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let members: Vec<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p["name"].as_str().ok_or("package without name")?.to_owned();
        let mut deps = Vec::new();
        for d in p["dependencies"].as_array().into_iter().flatten() {
            let dname = d["name"].as_str().unwrap_or_default().to_owned();
            let kind = match d["kind"].as_str() {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let workspace = members.contains(&dname.as_str()) || d["path"].is_string();
            deps.push(Dep { name: dname, kind, workspace });
        }
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn describe(class: Option<Class>) -> String {
    match class {
        Some(Class::Layer(l)) => format!("L{l}"),
        Some(Class::Standalone) => "L0 standalone".into(),
        Some(Class::Testkit) => "testkit".into(),
        Some(Class::Exempt) => "exempt".into(),
        None => "UNREGISTERED".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }
    use DepKind::*;

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("modelcraft-model", &[("serde", Normal, false)]),
            c("modelcraft-doc", &[("modelcraft-model", Normal, true)]),
            c("modelcraft-engine", &[("modelcraft-doc", Normal, true), ("modelcraft-testkit", Dev, true)]),
            c("modelcraft-ui-egui", &[("modelcraft-engine", Normal, true), ("egui", Normal, false)]),
            c("modelcraft-cli", &[("modelcraft-ui-egui", Normal, true)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_dependency_flagged() {
        let v = check(&[c("modelcraft-doc", &[("modelcraft-engine", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 4, to: 6, .. }]));
        let v = check(&[c("modelcraft-fem", &[("modelcraft-solver", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 2, .. }]));
    }

    #[test]
    fn sideways_dependency_flagged() {
        let v = check(&[c("modelcraft-mesh", &[("modelcraft-fem", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 1, .. }]));
    }

    #[test]
    fn self_dev_dependency_ignored() {
        assert!(check(&[c("modelcraft-doc", &[("modelcraft-doc", Dev, true)])]).is_empty());
    }

    #[test]
    fn upward_dev_dependency_flagged() {
        let v = check(&[c("modelcraft-model", &[("modelcraft-doc", Dev, true)])]);
        assert!(matches!(v[..], [Violation::Upward { kind: Dev, .. }]));
    }

    #[test]
    fn solvers_sit_above_the_kernels() {
        let g = [
            c("modelcraft-fem", &[("modelcraft-model", Normal, true)]),
            c("modelcraft-solver", &[("modelcraft-fem", Normal, true), ("modelcraft-mesh", Normal, true)]),
            c("modelcraft-engine", &[("modelcraft-solver", Normal, true)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
        let v = check(&[c("modelcraft-solver", &[("modelcraft-engine", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 2, to: 6, .. }]));
        let v = check(&[c("modelcraft-engine", &[("modelcraft-cli", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 6, to: APPS_LAYER, .. }]));
    }

    #[test]
    fn ui_crates_below_the_ui_layer_flagged() {
        for dep in ["egui", "eframe", "winit", "egui_kittest", "rfd", "bevy_ecs", "bevy"] {
            let v = check(&[c("modelcraft-engine", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::UiBelowUiLayer { layer: 6, .. }]), "{dep}");
            let v = check(&[c("modelcraft-solver", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::UiBelowUiLayer { layer: 2, .. }]), "{dep}");
        }
        assert!(check(&[c("modelcraft-engine", &[("egui_extras_not", Normal, false)])]).is_empty());
        assert!(check(&[c("modelcraft-ui-egui", &[("winit", Normal, false)])]).is_empty());
    }

    #[test]
    fn only_geometry_uses_solidcraft() {
        assert!(check(&[c("modelcraft-geometry", &[("solidcraft-kernel", Normal, false)])]).is_empty());
        let v = check(&[c("modelcraft-mesh", &[("solidcraft-geom", Normal, false)])]);
        assert!(matches!(v[..], [Violation::SolidCraftOutsideGeometry { .. }]));
    }

    #[test]
    fn unregistered_crate_is_error() {
        let v = check(&[c("modelcraft-mystery", &[])]);
        assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "modelcraft-mystery"));
        assert!(v[0].to_string().contains("register"));
    }

    #[test]
    fn testkit_only_as_dev_dependency() {
        let v = check(&[c("modelcraft-doc", &[("modelcraft-testkit", Normal, true)])]);
        assert!(matches!(v[..], [Violation::TestkitAsNormalDep { .. }]));
        assert!(check(&[c("modelcraft-doc", &[("modelcraft-testkit", Dev, true)])]).is_empty());
        assert!(check(&[c("modelcraft-testkit", &[("modelcraft-engine", Normal, true)])]).is_empty());
    }

    #[test]
    fn apps_and_xtask_exempt() {
        for app in ["modelcraft", "modelcraft-cli", "xtask"] {
            assert!(check(&[c(app, &[("egui", Normal, false), ("modelcraft-ui-egui", Normal, true)])]).is_empty());
        }
    }

    #[test]
    fn metadata_parsing() {
        let meta: Value = serde_json::from_str(
            r#"{"packages":[
                {"name":"modelcraft-doc","dependencies":[
                    {"name":"modelcraft-model","kind":null,"path":"/x/crates/model"},
                    {"name":"serde","kind":null},
                    {"name":"proptest","kind":"dev"}]},
                {"name":"modelcraft-model","dependencies":[]}
            ]}"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        assert_eq!(g.len(), 2);
        let doc = g.iter().find(|c| c.name == "modelcraft-doc").unwrap();
        assert!(doc.deps[0].workspace && !doc.deps[1].workspace);
        assert_eq!(doc.deps[2].kind, Dev);
        assert!(check(&g).is_empty());
    }
}
