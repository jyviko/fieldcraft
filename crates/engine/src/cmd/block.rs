//! `mesh.block`: a structured mesh of one six-sided block from its corners and curved edges
//! ([`modelcraft_mesh::block`]), joined to the mesh already there and tagged as a body with named
//! faces, edges and vertices, so sets by geometry select it.

use std::collections::{BTreeMap, BTreeSet};

use modelcraft_mesh::block::{Block, BlockMesh, Curve, block};
use modelcraft_model::{ElementKind, Entity, GeometryRef, MAX_LABEL, Model};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const BLOCK: CommandSpec = CommandSpec {
    id: "mesh.block",
    label: "Mesh Block",
    params: r#"{"body": "Plate", "kind": "hex8|hex20|tet4|tet10|wedge6", "corners": [[x, y, z] × 8] (hex8 order: 1-2-3-4 counter-clockwise seen from 5-6-7-8), "divisions": [n12, n14, n15] (elements along 1→2, 1→4, 1→5), "bias": [1, 1, 1] (optional: last over first element size along each direction), "edges": [{"corners": [1, 2], "arc": {"center": [x, y, z]}} | {"corners": [2, 3], "ellipse": {"center": [x, y, z], "axes": [[a], [b]] (perpendicular)}} | {"corners": [5, 6], "name": "tip_edge"}] (optional: curved edges, the shorter arc between the corners, and edge names), "faces": {"1": "bottom", "4": "tip"} (optional: names of block faces, numbered as hex8 faces: 1 = 1-2-3-4, 2 = 5-6-7-8, 3 = 1-2-6-5, 4 = 2-3-7-6, 5 = 3-4-8-7, 6 = 4-1-5-8), "vertices": {"1": "A"} (optional: names of corners), "merge": distance (optional, default 1e-6 of the block's diagonal; 0 never joins)} — adds a mapped mesh of the block (transfinite interpolation of its edges; quadratic nodes on the curves), uses existing nodes within `merge` of new ones so blocks that share a face join (the shared face must have the same elements on both sides, or nothing changes), and tags the elements with the body and element faces, edges and nodes with the named faces, edges and vertices"#,
    kind: CommandKind::Model,
    run: block_mesh,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    body: String,
    kind: String,
    corners: [[f64; 3]; 8],
    divisions: [usize; 3],
    #[serde(default = "uniform")]
    bias: [f64; 3],
    #[serde(default)]
    edges: Vec<EdgeParams>,
    #[serde(default)]
    faces: BTreeMap<String, String>,
    #[serde(default)]
    vertices: BTreeMap<String, String>,
    merge: Option<f64>,
}

fn uniform() -> [f64; 3] {
    [1.0; 3]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeParams {
    corners: [usize; 2],
    name: Option<String>,
    arc: Option<ArcParams>,
    ellipse: Option<EllipseParams>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArcParams {
    center: [f64; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EllipseParams {
    center: [f64; 3],
    axes: [[f64; 3]; 2],
}

/// A 1-based number in `1..=n` given as a JSON object key, 0-based.
fn numbered(key: &str, n: usize, what: &str) -> Result<usize> {
    key.parse::<usize>()
        .ok()
        .filter(|k| (1..=n).contains(k))
        .map(|k| k - 1)
        .ok_or_else(|| EngineError::Params(format!("{what} `{key}` must be a number from 1 to {n}")))
}

fn block_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    let p: Params = serde_json::from_value(p.clone()).map_err(|e| EngineError::Params(format!("mesh.block: {e}")))?;
    let kind = ElementKind::from_name(&p.kind).ok_or_else(|| EngineError::Params(format!("unknown element kind `{}`", p.kind)))?;
    let mut curves = Vec::new();
    let mut edge_names = Vec::new();
    for e in &p.edges {
        let pair = [0, 1].map(|i| e.corners[i].wrapping_sub(1));
        let index =
            Block::edge_index(pair).ok_or_else(|| EngineError::Params(format!("corners {:?} are not the two ends of a block edge", e.corners)))?;
        let curve = match (&e.arc, &e.ellipse) {
            (Some(a), None) => Some(Curve::Arc { center: a.center }),
            (None, Some(el)) => Some(Curve::Ellipse { center: el.center, axes: el.axes }),
            (None, None) => None,
            (Some(_), Some(_)) => return Err(EngineError::Params(format!("edge {:?} is an arc or an ellipse, not both", e.corners))),
        };
        if let Some(c) = curve {
            curves.push((pair, c));
        }
        if let Some(n) = &e.name {
            edge_names.push((index, n.clone()));
        }
        if curve.is_none() && e.name.is_none() {
            return Err(EngineError::Params(format!("edge {:?} gives neither a curve nor a name", e.corners)));
        }
    }
    let faces: Vec<(usize, String)> = p.faces.iter().map(|(k, n)| Ok((numbered(k, 6, "face")?, n.clone()))).collect::<Result<_>>()?;
    let vertices: Vec<(usize, String)> = p.vertices.iter().map(|(k, n)| Ok((numbered(k, 8, "vertex")?, n.clone()))).collect::<Result<_>>()?;
    // Check every name before meshing.
    let body = GeometryRef::body(p.body.clone())?;
    let face_refs: Vec<(usize, GeometryRef)> =
        faces.iter().map(|(k, n)| Ok((*k, GeometryRef::face(p.body.clone(), n.clone())?))).collect::<Result<_>>()?;
    let edge_refs: Vec<(usize, GeometryRef)> =
        edge_names.iter().map(|(k, n)| Ok((*k, GeometryRef::new(p.body.clone(), Entity::Edge(n.clone()))?))).collect::<Result<_>>()?;
    let vertex_refs: Vec<(usize, GeometryRef)> =
        vertices.iter().map(|(k, n)| Ok((*k, GeometryRef::new(p.body.clone(), Entity::Vertex(n.clone()))?))).collect::<Result<_>>()?;

    let mesh = block(&Block { kind, corners: p.corners, divisions: p.divisions, bias: p.bias, curves })?;
    let diagonal = {
        let lo = (0..3).map(|d| p.corners.iter().map(|c| c[d]).fold(f64::INFINITY, f64::min));
        let hi = (0..3).map(|d| p.corners.iter().map(|c| c[d]).fold(f64::NEG_INFINITY, f64::max));
        lo.zip(hi).map(|(l, h)| (h - l) * (h - l)).sum::<f64>().sqrt()
    };
    let merge = p.merge.unwrap_or(1e-6 * diagonal);
    if !(merge.is_finite() && merge >= 0.0 && merge <= 1e-2 * diagonal) {
        return Err(EngineError::Params(format!("`merge` must be from 0 to 1/100 of the block's diagonal ({diagonal})")));
    }

    let next = |last: Option<u32>| last.map_or(Some(1), |l| l.checked_add(1));
    let first_node = next(s.model.nodes().keys().next_back().copied());
    let first_element = next(s.model.elements().keys().next_back().copied());
    let fits = |first: Option<u32>, count: usize| first.filter(|&f| u64::from(f) + count as u64 <= u64::from(MAX_LABEL) + 1);
    let (Some(first_node), Some(first_element)) = (fits(first_node, mesh.points.len()), fits(first_element, mesh.elements.len())) else {
        return Err(EngineError::Params(format!("the block needs labels beyond {MAX_LABEL}")));
    };

    // Each block point: an existing node within `merge`, or a new label.
    let existing = joined(&s.model, &mesh.points, merge)?;
    let mut labels = Vec::with_capacity(mesh.points.len());
    let mut added = 0u32;
    for (i, xyz) in mesh.points.iter().enumerate() {
        match existing.get(i).copied().flatten() {
            Some(label) => labels.push(label),
            None => {
                let label = first_node + added;
                s.model.set_node(label, *xyz)?;
                labels.push(label);
                added += 1;
            }
        }
    }
    let merged: BTreeSet<u32> = existing.iter().flatten().copied().collect();
    let elements: Vec<Vec<u32>> = mesh.elements.iter().map(|e| e.iter().map(|&i| labels.get(i as usize).copied().unwrap_or(0)).collect()).collect();
    if !merged.is_empty() {
        check_conforming(&s.model, kind, &mesh, &elements, &merged, merge)?;
    }

    let body_tag = s.model.geometry_tag(&body)?;
    for (i, nodes) in elements.into_iter().enumerate() {
        let label = first_element + i as u32;
        s.model.set_element(label, kind, nodes)?;
        s.model.tag_element(label, body_tag)?;
    }
    for (k, r) in &face_refs {
        let t = s.model.geometry_tag(r)?;
        for &(e, f, bf) in &mesh.faces {
            if usize::from(bf) == *k {
                s.model.tag_element_face(first_element + e, f, t)?;
            }
        }
    }
    for (k, r) in &edge_refs {
        let t = s.model.geometry_tag(r)?;
        for &(e, edge, be) in &mesh.edges {
            if usize::from(be) == *k {
                s.model.tag_element_edge(first_element + e, edge, t)?;
            }
        }
    }
    for (k, r) in &vertex_refs {
        let t = s.model.geometry_tag(r)?;
        let node = mesh.corners.get(*k).and_then(|&i| labels.get(i as usize)).copied().unwrap_or(0);
        s.model.tag_node(node, t)?;
    }
    Ok(json!({
        "body": p.body,
        "kind": kind.name(),
        "elements": mesh.elements.len(),
        "first_element": first_element,
        "nodes": added,
        "first_node": first_node,
        "joined": merged.len(),
    }))
}

/// For each block point, the existing node within `merge` of it (the nearest; the lowest label
/// on a tie). Two points may not join the same node.
fn joined(model: &Model, points: &[[f64; 3]], merge: f64) -> Result<Vec<Option<u32>>> {
    if merge == 0.0 || model.nodes().is_empty() {
        return Ok(vec![None; points.len()]);
    }
    let cell = |x: f64| (x / merge).floor() as i64;
    let mut grid: BTreeMap<[i64; 3], Vec<(u32, [f64; 3])>> = BTreeMap::new();
    for (&label, node) in model.nodes() {
        grid.entry(node.xyz.map(cell)).or_default().push((label, node.xyz));
    }
    let mut out = Vec::with_capacity(points.len());
    let mut taken: BTreeMap<u32, usize> = BTreeMap::new();
    for (i, p) in points.iter().enumerate() {
        let c = p.map(cell);
        let mut best: Option<(f64, u32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &(label, xyz) in grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]).into_iter().flatten() {
                        let d = ((xyz[0] - p[0]).powi(2) + (xyz[1] - p[1]).powi(2) + (xyz[2] - p[2]).powi(2)).sqrt();
                        if d <= merge && best.is_none_or(|(bd, bl)| d < bd || (d == bd && label < bl)) {
                            best = Some((d, label));
                        }
                    }
                }
            }
        }
        if let Some((_, label)) = best
            && let Some(other) = taken.insert(label, i)
        {
            return Err(EngineError::Params(format!(
                "block points {other} and {i} both lie within `merge` of node {label}: lower `merge` or refine the block"
            )));
        }
        out.push(best.map(|(_, l)| l));
    }
    Ok(out)
}

/// Faces as sorted node lists.
fn key(nodes: impl Iterator<Item = u32>) -> Vec<u32> {
    let mut k: Vec<u32> = nodes.collect();
    k.sort_unstable();
    k
}

/// Where the block joins existing nodes it must meet the mesh face to face: an element face
/// (of the block, or of the mesh) whose corners all landed on shared nodes must be a face of
/// the other side too, with the same nodes (no mixed linear and quadratic sides); and no node on
/// the block's boundary may lie on an edge of the mesh between two of its nodes, nor the other
/// way round (no hanging nodes where the divisions differ). Edges are tested as straight
/// segments, so a mismatch along a curved shared edge is caught only through its faces.
fn check_conforming(model: &Model, kind: ElementKind, mesh: &BlockMesh, elements: &[Vec<u32>], merged: &BTreeSet<u32>, merge: f64) -> Result<()> {
    let refuse = |what: String| {
        Err(EngineError::Params(format!("{what}: the block must meet the mesh face to face, with the same divisions and element kind")))
    };
    let corners_of = |k: ElementKind, nodes: &[u32], local: &[usize]| -> Vec<u32> {
        key(local.iter().filter(|&&i| i < k.corner_count()).filter_map(|&i| nodes.get(i).copied()))
    };
    // The block's boundary faces, by sorted corners, with their full node lists.
    let mut new_faces: BTreeMap<Vec<u32>, Vec<u32>> = BTreeMap::new();
    for &(e, f, _) in &mesh.faces {
        let (Some(nodes), Some(local)) = (elements.get(e as usize), usize::from(f).checked_sub(1).and_then(|i| kind.faces().get(i))) else { continue };
        new_faces.insert(corners_of(kind, nodes, local), key(local.iter().filter_map(|&i| nodes.get(i).copied())));
    }
    let mut old_faces: BTreeMap<Vec<u32>, Vec<u32>> = BTreeMap::new();
    let mut old_edges: BTreeSet<[u32; 2]> = BTreeSet::new();
    for e in model.elements().values().filter(|e| e.nodes.iter().any(|n| merged.contains(n))) {
        for local in e.kind.faces() {
            old_faces.insert(corners_of(e.kind, &e.nodes, local), key(local.iter().filter_map(|&i| e.nodes.get(i).copied())));
        }
    }
    for (corners, nodes) in &new_faces {
        if corners.iter().all(|n| merged.contains(n)) && old_faces.get(corners) != Some(nodes) {
            return refuse(format!("the block face on nodes {corners:?} lies on existing nodes but is no element face there"));
        }
    }
    for (corners, nodes) in &old_faces {
        if corners.iter().all(|n| merged.contains(n)) && new_faces.get(corners) != Some(nodes) {
            return refuse(format!("the element face on nodes {corners:?} lies on the block but no block element shares it"));
        }
    }
    // Hanging nodes: boundary nodes of either side on the other side's element edges.
    let xyz = |label: u32| model.node(label).map(|n| n.xyz);
    let block_points: BTreeMap<u32, [f64; 3]> =
        mesh.elements.iter().zip(elements).flat_map(|(pts, labels)| pts.iter().zip(labels).filter_map(|(&p, &l)| Some((l, *mesh.points.get(p as usize)?)))).collect();
    let mut new_edges: BTreeSet<[u32; 2]> = BTreeSet::new();
    for &(e, f, _) in &mesh.faces {
        let (Some(nodes), Some(local)) = (elements.get(e as usize), usize::from(f).checked_sub(1).and_then(|i| kind.faces().get(i))) else { continue };
        let cs: Vec<u32> = local.iter().filter(|&&i| i < kind.corner_count()).filter_map(|&i| nodes.get(i).copied()).collect();
        for (i, &a) in cs.iter().enumerate() {
            let b = cs.get((i + 1) % cs.len()).copied().unwrap_or(a);
            new_edges.insert(if a < b { [a, b] } else { [b, a] });
        }
    }
    for e in model.elements().values().filter(|e| e.nodes.iter().any(|n| merged.contains(n))) {
        for local in e.kind.edges() {
            if let (Some(&a), Some(&b)) = (local.first().and_then(|&i| e.nodes.get(i)), local.get(1).and_then(|&i| e.nodes.get(i))) {
                old_edges.insert(if a < b { [a, b] } else { [b, a] });
            }
        }
    }
    let new_boundary: BTreeSet<u32> = new_faces.values().flatten().copied().filter(|n| !merged.contains(n)).collect();
    let on_segment = |p: [f64; 3], a: [f64; 3], b: [f64; 3]| {
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        if !(len2 > 0.0) {
            return false;
        }
        let t = (ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2;
        let d = [ap[0] - t * ab[0], ap[1] - t * ab[1], ap[2] - t * ab[2]];
        let tol = merge / len2.sqrt();
        t > tol && t < 1.0 - tol && (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= merge
    };
    for &n in &new_boundary {
        let Some(&p) = block_points.get(&n) else { continue };
        for [a, b] in &old_edges {
            if let (Some(xa), Some(xb)) = (xyz(*a), xyz(*b))
                && on_segment(p, xa, xb)
            {
                return refuse(format!("a block node at {p:?} lies on the edge between nodes {a} and {b}"));
            }
        }
    }
    let old_nodes: BTreeSet<u32> = old_faces.values().flatten().copied().filter(|n| !merged.contains(n)).collect();
    for n in old_nodes {
        let Some(p) = xyz(n) else { continue };
        for [a, b] in &new_edges {
            if let (Some(&xa), Some(&xb)) = (block_points.get(a), block_points.get(b))
                && on_segment(p, xa, xb)
            {
                return refuse(format!("node {n} at {p:?} lies on a block edge"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn cube(x0: f64) -> serde_json::Value {
        json!([[x0, 0, 0], [x0 + 1.0, 0, 0], [x0 + 1.0, 1, 0], [x0, 1, 0], [x0, 0, 1], [x0 + 1.0, 0, 1], [x0 + 1.0, 1, 1], [x0, 1, 1]])
    }

    #[test]
    fn blocks_join_face_to_face_and_name_their_geometry() {
        let mut s = Session::default();
        let r = s
            .execute(
                "mesh.block",
                &json!({"body": "Bar", "kind": "hex20", "corners": cube(0.0), "divisions": [2, 2, 2], "faces": {"6": "root"}, "vertices": {"7": "corner"}, "edges": [{"corners": [5, 8], "name": "top_root"}]}),
            )
            .unwrap();
        assert_eq!((r["elements"].clone(), r["nodes"].clone(), r["joined"].clone()), (json!(8), json!(81), json!(0)));
        let r = s
            .execute("mesh.block", &json!({"body": "Bar", "kind": "hex20", "corners": cube(1.0), "divisions": [2, 2, 2], "faces": {"4": "tip"}}))
            .unwrap();
        assert_eq!((r["first_element"].clone(), r["nodes"].clone(), r["joined"].clone()), (json!(9), json!(81 - 21), json!(21)));
        for (set, geometry, nodes) in [
            ("root", json!({"body": "Bar", "face": "root"}), 21),
            ("tip", json!({"body": "Bar", "face": "tip"}), 21),
            ("edge", json!({"body": "Bar", "edge": "top_root"}), 5),
            ("corner", json!({"body": "Bar", "vertex": "corner"}), 1),
        ] {
            s.execute("model.set.create", &json!({"name": set, "geometry": [geometry]})).unwrap();
            let r = s.execute("model.set.resolve", &json!({"name": set})).unwrap();
            assert_eq!(r["nodes"]["count"], nodes, "{set}: {r}");
        }
        let q = s.execute("mesh.quality", &json!({})).unwrap();
        assert_eq!((q["valid"].clone(), q["volume"].as_f64().map(|v| (v - 2.0).abs() < 1e-12)), (json!(16), Some(true)));

        // A third block that meets the second with other divisions or another kind changes nothing.
        let model = s.model.clone();
        for bad in [
            json!({"body": "Bar", "kind": "hex20", "corners": cube(2.0), "divisions": [2, 3, 2]}),
            json!({"body": "Bar", "kind": "hex20", "corners": cube(2.0), "divisions": [1, 1, 1]}),
            json!({"body": "Bar", "kind": "hex8", "corners": cube(2.0), "divisions": [4, 4, 4]}),
            json!({"body": "Bar", "kind": "tet10", "corners": cube(2.0), "divisions": [2, 2, 2]}),
        ] {
            assert!(s.execute("mesh.block", &bad).is_err(), "{bad}");
            assert_eq!(s.model, model);
        }
        // A finer block first, then a coarser one against it: caught from the other side.
        let mut t = Session::default();
        t.execute("mesh.block", &json!({"body": "A", "kind": "hex8", "corners": cube(0.0), "divisions": [2, 2, 2]})).unwrap();
        assert!(t.execute("mesh.block", &json!({"body": "B", "kind": "hex8", "corners": cube(1.0), "divisions": [1, 1, 1]})).is_err());
        // Divisions that share no node inside the face: the hanging nodes on its edges are caught.
        assert!(t.execute("mesh.block", &json!({"body": "B", "kind": "hex8", "corners": cube(1.0), "divisions": [3, 3, 3]})).is_err());
        // A ring closed by its last block touches existing nodes at the corners of faces it does
        // not share (the ends): that is no conflict.
        let mut ring = Session::default();
        for k in 0..4 {
            let at = |r: f64, q: usize, z: f64| {
                let a = std::f64::consts::FRAC_PI_2 * q as f64;
                let c = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v };
                json!([c(r * a.cos()), c(r * a.sin()), z])
            };
            let corners = json!([at(1.0, k, 0.0), at(2.0, k, 0.0), at(2.0, k + 1, 0.0), at(1.0, k + 1, 0.0), at(1.0, k, 1.0), at(2.0, k, 1.0), at(2.0, k + 1, 1.0), at(1.0, k + 1, 1.0)]);
            let edges = json!([{"corners": [1, 4], "arc": {"center": [0, 0, 0]}}, {"corners": [2, 3], "arc": {"center": [0, 0, 0]}}, {"corners": [5, 8], "arc": {"center": [0, 0, 1]}}, {"corners": [6, 7], "arc": {"center": [0, 0, 1]}}]);
            ring.execute("mesh.block", &json!({"body": "Ring", "kind": "hex20", "corners": corners, "divisions": [2, 3, 2], "edges": edges})).unwrap();
        }
        assert_eq!(ring.model.elements().len(), 48);
        assert!(t.execute("mesh.block", &json!({"body": "B", "kind": "hex8", "corners": cube(1.0), "divisions": [1, 1, 1], "merge": 0})).is_ok());
    }

    #[test]
    fn curved_edges_and_hostile_parameters() {
        let mut s = Session::default();
        let corners = json!([[1, 0, 0], [2, 0, 0], [0, 2, 0], [0, 1, 0], [1, 0, 1], [2, 0, 1], [0, 2, 1], [0, 1, 1]]);
        let edges = json!([
            {"corners": [2, 3], "arc": {"center": [0, 0, 0]}},
            {"corners": [6, 7], "arc": {"center": [0, 0, 1]}},
            {"corners": [1, 4], "ellipse": {"center": [0, 0, 0], "axes": [[1, 0, 0], [0, 1, 0]]}},
            {"corners": [5, 8], "arc": {"center": [0, 0, 1]}, "name": "inner_top"}
        ]);
        let r =
            s.execute("mesh.block", &json!({"body": "Ring", "kind": "tet10", "corners": corners, "divisions": [2, 4, 1], "edges": edges})).unwrap();
        assert_eq!(r["elements"], 48);
        let q = s.execute("mesh.quality", &json!({})).unwrap();
        let exact = std::f64::consts::PI * 3.0 / 4.0;
        assert!((q["volume"].as_f64().unwrap() - exact).abs() < 1e-3 * exact, "{q}");

        let journal = s.journal.len();
        for bad in [
            json!({}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1]}),
            json!({"body": "B", "kind": "quad4", "corners": cube(5.0), "divisions": [1, 1, 1]}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "faces": {"7": "x"}}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "faces": {"one": "x"}}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "vertices": {"0": "x"}}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "edges": [{"corners": [1, 7], "name": "x"}]}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "edges": [{"corners": [0, 1], "name": "x"}]}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "edges": [{"corners": [1, 2]}]}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "merge": -1}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "merge": 1}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1e9, 1e9, 1e9]}),
            json!({"body": "B", "kind": "hex8", "corners": cube(1e300), "divisions": [1, 1, 1]}),
            json!({"body": "", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1]}),
            json!({"body": "B", "kind": "hex8", "corners": cube(5.0), "divisions": [1, 1, 1], "colour": "red"}),
        ] {
            assert!(s.execute("mesh.block", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.journal.len(), journal);
    }
}
