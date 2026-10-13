//! The command registry. Each area has its own module; [`ALL`] lists every command.

pub(crate) mod deck;
pub(crate) mod define;
mod edit;
pub(crate) mod file;
mod geometry;
mod mesh;
mod model;
mod tags;

use modelcraft_model::MAX_LABEL;
use serde::Deserialize;
use serde_json::{Value, json};

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
    mesh::SURFACE_CHECK,
    tags::TAG,
    tags::TAG_CLEAR,
    model::UNITS,
    model::CHECK,
    model::SET_RESOLVE,
    define::SET_CREATE,
    define::SET_EDIT,
    define::SET_DELETE,
    define::MATERIAL_CREATE,
    define::MATERIAL_EDIT,
    define::MATERIAL_DELETE,
    define::SECTION_CREATE,
    define::SECTION_EDIT,
    define::SECTION_DELETE,
    define::REFERENCE_POINT_CREATE,
    define::REFERENCE_POINT_EDIT,
    define::REFERENCE_POINT_DELETE,
    define::COUPLING_CREATE,
    define::COUPLING_EDIT,
    define::COUPLING_DELETE,
    define::BOUNDARY_CONDITION_CREATE,
    define::BOUNDARY_CONDITION_EDIT,
    define::BOUNDARY_CONDITION_DELETE,
    define::LOAD_CREATE,
    define::LOAD_EDIT,
    define::LOAD_DELETE,
    define::STEP_CREATE,
    define::STEP_EDIT,
    define::STEP_DELETE,
    define::STEP_ACTIVATE,
    define::ACTIVITY,
    file::OPEN,
    file::SAVE,
    file::SAVE_AS,
    file::RECOVERY_LIST,
    file::RECOVER,
    file::RECOVERY_DISCARD,
    edit::UNDO,
    edit::REDO,
    deck::WRITE,
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

/// A required path parameter.
pub(crate) fn path_param<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= modelcraft_io::MAX_PATH_LEN && !s.contains('\0'))
        .ok_or_else(|| EngineError::Params(format!("`{key}` must be a file path")))
}

/// An optional number parameter.
pub(crate) fn optional_number(p: &Value, key: &str, default: f64) -> Result<f64> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_f64().ok_or_else(|| EngineError::Params(format!("`{key}` must be a number"))),
    }
}

/// The `name` parameter.
pub(crate) fn name(p: &Value) -> Result<&str> {
    p.get("name").and_then(Value::as_str).ok_or_else(|| EngineError::Params("`name` must be a name".into()))
}

/// An optional parameter read as `T`'s JSON form; `None` when it is absent.
pub(crate) fn opt<'a, T: Deserialize<'a>>(p: &'a Value, key: &str) -> Result<Option<T>> {
    p.get(key).map(|v| T::deserialize(v).map_err(|e| EngineError::Params(format!("`{key}`: {e}")))).transpose()
}

/// A required parameter read as `T`'s JSON form.
pub(crate) fn req<'a, T: Deserialize<'a>>(p: &'a Value, key: &str) -> Result<T> {
    opt(p, key)?.ok_or_else(|| EngineError::Params(format!("`{key}` is required")))
}

/// Entries listed in a report: the first ones, and how many there are.
pub(crate) const LISTED: usize = 100;

pub(crate) fn listed<T: serde::Serialize>(v: &[T]) -> Value {
    json!({"count": v.len(), "labels": v.get(..LISTED).unwrap_or(v)})
}
