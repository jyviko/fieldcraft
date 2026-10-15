use fieldcraft_model::{Severity, UnitSystem};
use serde_json::{Value, json};

use super::{listed, name};
use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const INFO: CommandSpec =
    CommandSpec { id: "model.info", label: "Model Info", params: "{}", form: Some(&[]), kind: CommandKind::Model, requires: &[], run: info };

pub const UNITS: CommandSpec = CommandSpec {
    id: "model.units",
    label: "Unit System",
    params: r#"{"system": "m-kg-s-N-Pa|mm-t-s-N-MPa|mm-kg-ms-kN-GPa|mm-g-ms-N-MPa|in-lbf-s-psi|unspecified"} — labels the system the model's numbers are in; nothing is converted"#,
    form: Some(super::forms::UNITS),
    kind: CommandKind::Model,
    requires: &[],
    run: units,
};

pub const CHECK: CommandSpec =
    CommandSpec { id: "model.check", label: "Check Model", params: "{}", form: Some(&[]), kind: CommandKind::Model, requires: &[], run: check };

pub const SET_RESOLVE: CommandSpec = CommandSpec {
    id: "model.set.resolve",
    label: "Resolve Set",
    params: r#"{"name": "root"} — the set's nodes, elements and element faces on the current mesh (the first 100 of each); fails naming every member that is not on the mesh"#,
    form: Some(super::forms::SET_RESOLVE),
    kind: CommandKind::Model,
    requires: &[Requirement::Set],
    run: set_resolve,
};

fn info(s: &mut Session, _: &Value) -> Result<Value> {
    let counts: serde_json::Map<String, Value> = s.model.element_counts().into_iter().map(|(k, n)| (k.name().to_string(), json!(n))).collect();
    let definitions: serde_json::Map<String, Value> = s.model.definition_counts().into_iter().map(|(k, n)| (k.to_string(), json!(n))).collect();
    Ok(json!({"nodes": s.model.nodes().len(), "elements": counts, "units": s.model.units().label(), "definitions": definitions}))
}

fn units(s: &mut Session, p: &Value) -> Result<Value> {
    let label = p.get("system").and_then(Value::as_str).unwrap_or_default();
    let system = UnitSystem::from_label(label).ok_or_else(|| {
        let known: Vec<&str> = UnitSystem::ALL.iter().map(|u| u.label()).collect();
        EngineError::Params(format!("`system` must be one of {}", known.join(", ")))
    })?;
    s.model.set_units(system);
    // SolidCraft designs are in millimetres.
    let warning = match system.length() {
        Some(length) if length != "mm" && s.design.is_some() => {
            Some(format!("the open design is in millimetres but this system's length unit is {length}"))
        }
        _ => None,
    };
    Ok(json!({"system": system.label(), "warning": warning}))
}

fn check(s: &mut Session, _: &Value) -> Result<Value> {
    let diagnostics = s.model.check();
    let errors = diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
    let first_error = diagnostics.iter().find(|d| d.severity == Severity::Error).map(ToString::to_string);
    let progress = crate::progress::to_json(&s.progress_given(first_error));
    Ok(json!({
        "ready": errors == 0,
        "errors": errors,
        "warnings": diagnostics.len() - errors,
        "diagnostics": diagnostics,
        "progress": progress,
    }))
}

fn set_resolve(s: &mut Session, p: &Value) -> Result<Value> {
    let r = s.model.resolve_set(name(p)?)?;
    Ok(json!({"nodes": listed(&r.nodes), "elements": listed(&r.elements), "faces": listed(&r.faces)}))
}
