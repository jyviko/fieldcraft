//! The command registry. Each area has its own module; [`ALL`] lists every command.

mod geometry;
mod mesh;
mod model;

use modelcraft_model::MAX_LABEL;
use serde_json::Value;

use crate::{CommandSpec, EngineError, Result};

pub static ALL: &[CommandSpec] = &[
    model::INFO,
    geometry::OPEN,
    geometry::BODIES,
    geometry::TESSELLATE,
    geometry::IMPORT_MESH,
    mesh::NODE_ADD,
    mesh::NODE_REMOVE,
    mesh::ELEMENT_ADD,
    mesh::ELEMENT_REMOVE,
    mesh::QUALITY,
    mesh::DELAUNAY_POINTS,
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
    p.get(key).and_then(xyz_value).ok_or_else(|| EngineError::Params(format!("`{key}` must be [x, y, z]")))
}

/// A JSON `[x, y, z]`.
fn xyz_value(v: &Value) -> Option<[f64; 3]> {
    match v.as_array()?.as_slice() {
        [x, y, z] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
        _ => None,
    }
}

/// A required list of `[x, y, z]` points, at most `max_len` long.
pub(crate) fn points(p: &Value, key: &str, max_len: usize) -> Result<Vec<[f64; 3]>> {
    let bad = |what: String| EngineError::Params(format!("`{key}` must be a list of at most {max_len} [x, y, z]{what}"));
    let raw = p.get(key).and_then(Value::as_array).filter(|a| a.len() <= max_len).ok_or_else(|| bad(String::new()))?;
    raw.iter().enumerate().map(|(i, v)| xyz_value(v).ok_or_else(|| bad(format!(" (entry {i} is not)")))).collect()
}

/// A required list of labels (each in 1..=MAX_LABEL), at most `max_len` long.
pub(crate) fn labels(p: &Value, key: &str, max_len: usize) -> Result<Vec<u32>> {
    let raw = p
        .get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() <= max_len)
        .ok_or_else(|| EngineError::Params(format!("`{key}` must be a list of at most {max_len} labels")))?;
    raw.iter()
        .map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| (1..=MAX_LABEL).contains(n)))
        .collect::<Option<Vec<u32>>>()
        .ok_or_else(|| EngineError::Params(format!("`{key}` must be labels in 1..={MAX_LABEL}")))
}
