use modelcraft_model::{Severity, UnitSystem};
use serde_json::{Value, json};

use crate::{CommandSpec, EngineError, Result, Session};

pub const INFO: CommandSpec = CommandSpec { id: "model.info", label: "Model Info", params: "{}", run: info };

pub const UNITS: CommandSpec = CommandSpec {
    id: "model.units",
    label: "Unit System",
    params: r#"{"system": "m-kg-s-N-Pa|mm-t-s-N-MPa|mm-kg-ms-kN-GPa|mm-g-ms-N-MPa|in-lbf-s-psi|unspecified"} — labels the system the model's numbers are in; nothing is converted"#,
    run: units,
};

pub const CHECK: CommandSpec = CommandSpec { id: "model.check", label: "Check Model", params: "{}", run: check };

pub const SET_RESOLVE: CommandSpec = CommandSpec {
    id: "model.set.resolve",
    label: "Resolve Set",
    params: r#"{"name": "root"} — the set's nodes, elements and element faces on the current mesh (the first 100 of each); fails naming every member that is not on the mesh"#,
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
    Ok(json!({
        "ready": errors == 0,
        "errors": errors,
        "warnings": diagnostics.len() - errors,
        "diagnostics": diagnostics,
    }))
}

/// Entries listed in a report: the first ones, and how many there are.
const LISTED: usize = 100;

fn set_resolve(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| EngineError::Params("`name` must be a set name".into()))?;
    let r = s.model.resolve_set(name)?;
    let listed = |v: Value, count: usize| json!({"count": count, "labels": v});
    Ok(json!({
        "nodes": listed(json!(r.nodes.iter().take(LISTED).collect::<Vec<_>>()), r.nodes.len()),
        "elements": listed(json!(r.elements.iter().take(LISTED).collect::<Vec<_>>()), r.elements.len()),
        "faces": listed(json!(r.faces.iter().take(LISTED).collect::<Vec<_>>()), r.faces.len()),
    }))
}
