//! Geometry tags on the mesh: which body, face, edge or vertex each mesh entity lies on. Meshers
//! write them; these commands set them by hand (for a mesh from elsewhere) and clear them.

use modelcraft_model::{ElementFace, GeometryRef, MAX_ELEMENTS};
use serde_json::{Value, json};

use super::labels;
use crate::{CommandSpec, EngineError, Result, Session};

pub const TAG: CommandSpec = CommandSpec {
    id: "mesh.tag",
    label: "Tag Mesh with Geometry",
    params: r#"{"geometry": {"body": "Body1"} | {"body", "face"} | {"body", "edge"} | {"body", "vertex"}, "elements": [labels] (filling a body, face or edge), "faces": [{"element": 1, "face": 3}] (on a face), "edges": [{"element": 1, "edge": 2}] (on an edge), "nodes": [labels] (on a vertex)} — records where the mesh lies on the geometry, so sets named by geometry resolve"#,
    run: tag,
};

pub const TAG_CLEAR: CommandSpec = CommandSpec { id: "mesh.tag.clear", label: "Clear Geometry Tags", params: "{}", run: tag_clear };

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementEdge {
    element: u32,
    edge: u8,
}

fn list<T: serde::de::DeserializeOwned>(p: &Value, key: &str) -> Result<Vec<T>> {
    match p.get(key) {
        None => Ok(Vec::new()),
        Some(v) if v.as_array().is_some_and(|a| a.len() <= MAX_ELEMENTS) => {
            serde_json::from_value(v.clone()).map_err(|e| EngineError::Params(format!("`{key}`: {e}")))
        }
        Some(_) => Err(EngineError::Params(format!("`{key}` must be a list of at most {MAX_ELEMENTS} entries"))),
    }
}

fn tag(s: &mut Session, p: &Value) -> Result<Value> {
    let geometry: GeometryRef = p
        .get("geometry")
        .ok_or_else(|| EngineError::Params("`geometry` must name a body, face, edge or vertex".into()))
        .and_then(|g| serde_json::from_value(g.clone()).map_err(|e| EngineError::Params(format!("`geometry`: {e}"))))?;
    let elements = if p.get("elements").is_some() { labels(p, "elements", MAX_ELEMENTS)? } else { Vec::new() };
    let nodes = if p.get("nodes").is_some() { labels(p, "nodes", MAX_ELEMENTS)? } else { Vec::new() };
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
