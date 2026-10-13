use std::io::Read;
use std::sync::Arc;

use modelcraft_geometry::{MAX_DESIGN_BYTES, Shape, open_design};
use serde_json::{Value, json};

use crate::{CommandSpec, EngineError, Result, Session};

pub const OPEN: CommandSpec = CommandSpec {
    id: "geometry.open",
    label: "Open Design",
    params: r#"{"path": "part.solidcraft"} — rebuilds the SolidCraft design and makes its bodies the geometry; opening it again after an edit picks up the change"#,
    run: open,
};

pub const BODIES: CommandSpec = CommandSpec {
    id: "geometry.bodies",
    label: "Bodies",
    params: "{} — the open design's bodies: measures and persistent face names (by face index)",
    run: bodies,
};

pub const TESSELLATE: CommandSpec = CommandSpec {
    id: "geometry.tessellate",
    label: "Tessellate Body",
    params: r#"{"body": "Body1", "tolerance": 0.05} — triangulates the body's boundary within the chordal tolerance (mm); reports triangles per face"#,
    run: tessellate,
};

/// Longest path accepted.
const MAX_PATH: usize = 4096;

fn read_design_file(path: &str) -> Result<Vec<u8>> {
    let fail = |e: std::io::Error| EngineError::Params(format!("cannot read `{path}`: {e}"));
    let file = std::fs::File::open(path).map_err(fail)?;
    let meta = file.metadata().map_err(fail)?;
    if !meta.is_file() {
        return Err(EngineError::Params(format!("`{path}` is not a file")));
    }
    if meta.len() > MAX_DESIGN_BYTES as u64 {
        return Err(EngineError::Params(format!("`{path}` is larger than {} MB", MAX_DESIGN_BYTES >> 20)));
    }
    let mut bytes = Vec::new();
    // The file may grow after the size check.
    file.take(MAX_DESIGN_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(fail)?;
    Ok(bytes)
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= MAX_PATH)
        .ok_or_else(|| EngineError::Params("`path` must be the design file's path".into()))?;
    let design = open_design(&read_design_file(path)?)?;
    let out = json!({
        "name": design.name,
        "bodies": design.bodies.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        "errors": design.errors.iter().map(|e| json!({"feature": e.feature, "message": e.message})).collect::<Vec<_>>(),
    });
    s.design = Some(Arc::new(design));
    Ok(out)
}

fn bodies(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(design) = &s.design else { return Ok(json!([])) };
    let mut out = Vec::with_capacity(design.bodies.len());
    for b in &design.bodies {
        let m = b.measure()?;
        out.push(json!({
            "name": b.name,
            "volume": m.volume,
            "area": m.area,
            "centroid": m.centroid,
            "bbox": [m.bbox_min, m.bbox_max],
            "faces": b.faces().iter().map(|f| json!({"name": f.name, "exact": f.exact})).collect::<Vec<_>>(),
        }));
    }
    Ok(Value::Array(out))
}

fn body<'a>(s: &'a Session, p: &Value) -> Result<&'a Shape> {
    let name = p.get("body").and_then(Value::as_str).ok_or_else(|| EngineError::Params("`body` must be a body name".into()))?;
    s.design.as_deref().and_then(|d| d.body(name)).ok_or_else(|| EngineError::Params(format!("no body `{name}` (open a design first)")))
}

fn tessellate(s: &mut Session, p: &Value) -> Result<Value> {
    let shape = body(s, p)?;
    let tolerance = p.get("tolerance").and_then(Value::as_f64).ok_or_else(|| EngineError::Params("`tolerance` must be a number (mm)".into()))?;
    let mesh = shape.tessellate(tolerance)?;
    let mut per_face = vec![0usize; shape.faces().len()];
    for &f in &mesh.tri_face {
        if let Some(n) = per_face.get_mut(f as usize) {
            *n += 1;
        }
    }
    let faces: Vec<Value> = shape.faces().iter().zip(per_face).map(|(f, n)| json!({"name": f.name, "triangles": n})).collect();
    Ok(json!({"body": shape.name, "vertices": mesh.positions.len(), "triangles": mesh.triangles.len(), "faces": faces}))
}
