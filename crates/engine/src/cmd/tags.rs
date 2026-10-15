//! Geometry tags on the mesh: which body, face, edge or vertex each mesh entity lies on. Meshers
//! write them; these commands set them by hand (for a mesh from elsewhere) and clear them.

use fieldcraft_model::{ElementFace, GeometryRef, MAX_ELEMENTS};
use serde_json::{Value, json};

use super::{opt, req};
use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const TAG: CommandSpec = CommandSpec {
    id: "mesh.tag",
    label: "Tag Mesh with Geometry",
    params: r#"{"geometry": {"body": "Body1"} | {"body", "face"} | {"body", "edge"} | {"body", "vertex"}, "elements": [labels] (filling a body, face or edge), "faces": [{"element": 1, "face": 3}] (on a face), "edges": [{"element": 1, "edge": 2}] (on an edge), "nodes": [labels] (on a vertex)} — records where the mesh lies on the geometry, so sets named by geometry resolve"#,
    form: Some(super::forms::TAG),
    kind: CommandKind::Model,
    requires: &[Requirement::Mesh],
    run: tag,
};

pub const TAG_CLEAR: CommandSpec = CommandSpec {
    id: "mesh.tag.clear",
    label: "Clear Geometry Tags",
    params: "{}",
    form: Some(&[]),
    kind: CommandKind::Model,
    requires: &[Requirement::Mesh],
    run: tag_clear,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementEdge {
    element: u32,
    edge: u8,
}

/// An optional list parameter of at most [`MAX_ELEMENTS`] entries.
fn list<'a, T: serde::Deserialize<'a>>(p: &'a Value, key: &str) -> Result<Vec<T>> {
    if p.get(key).and_then(Value::as_array).is_some_and(|a| a.len() > MAX_ELEMENTS) {
        return Err(EngineError::param(key, format!("must be a list of at most {MAX_ELEMENTS} entries")));
    }
    Ok(opt(p, key)?.unwrap_or_default())
}

fn tag(s: &mut Session, p: &Value) -> Result<Value> {
    let geometry: GeometryRef = req(p, "geometry")?;
    let elements: Vec<u32> = list(p, "elements")?;
    let nodes: Vec<u32> = list(p, "nodes")?;
    let faces: Vec<ElementFace> = list(p, "faces")?;
    let edges: Vec<ElementEdge> = list(p, "edges")?;
    let count = elements.len() + nodes.len() + faces.len() + edges.len();
    if count == 0 {
        return Err(EngineError::Params("list `elements`, `faces`, `edges` or `nodes` to tag".into()));
    }
    let t = s.model.geometry_tag(&geometry)?;
    for e in elements {
        s.model.tag_element(e, t)?;
    }
    for f in faces {
        s.model.tag_element_face(f.element, f.face, t)?;
    }
    for e in edges {
        s.model.tag_element_edge(e.element, e.edge, t)?;
    }
    for n in nodes {
        s.model.tag_node(n, t)?;
    }
    Ok(json!({"geometry": geometry, "tagged": count}))
}

fn tag_clear(s: &mut Session, _: &Value) -> Result<Value> {
    s.model.clear_geometry_tags();
    Ok(Value::Null)
}
