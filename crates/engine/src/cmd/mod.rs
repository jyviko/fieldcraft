//! The command registry. Each area has its own module; [`ALL`] lists every command.

mod geometry;
mod mesh;
mod model;

use serde_json::Value;

use crate::{CommandSpec, EngineError, Result};

pub static ALL: &[CommandSpec] = &[
    model::INFO,
    geometry::OPEN,
    geometry::BODIES,
    geometry::TESSELLATE,
    mesh::NODE_ADD,
    mesh::NODE_REMOVE,
    mesh::ELEMENT_ADD,
    mesh::ELEMENT_REMOVE,
];

/// A required label parameter (1..=u32::MAX; the model checks its own range).
pub(crate) fn label(p: &Value, key: &str) -> Result<u32> {
    p.get(key)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| EngineError::Params(format!("`{key}` must be a positive whole number")))
}

/// A required `[x, y, z]` parameter.
pub(crate) fn xyz(p: &Value, key: &str) -> Result<[f64; 3]> {
    let bad = || EngineError::Params(format!("`{key}` must be [x, y, z]"));
    let a = p.get(key).and_then(Value::as_array).filter(|a| a.len() == 3).ok_or_else(bad)?;
    let mut out = [0.0; 3];
    for (o, v) in out.iter_mut().zip(a) {
        *o = v.as_f64().ok_or_else(bad)?;
    }
    Ok(out)
}
