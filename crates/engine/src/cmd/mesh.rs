use std::borrow::Cow;
use std::collections::BTreeMap;

use modelcraft_geometry::SurfaceMesh;
use modelcraft_mesh::cdt::{CdtOptions, tetrahedralize};
use modelcraft_mesh::delaunay::Triangulation;
use modelcraft_mesh::quality::{self, QualityReport};
use modelcraft_mesh::surface::{BoundarySurface, Defect, FeatureKind, SurfaceOptions};
use modelcraft_model::{ElementKind, GeometryRef, MAX_ELEMENTS, MAX_LABEL};
use serde_json::{Value, json};

use super::{label, labels, listed, optional_number, points, xyz};
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const NODE_ADD: CommandSpec = CommandSpec {
    id: "mesh.node.add",
    label: "Add Node",
    params: r#"{"label": 1, "xyz": [x, y, z]} — adds the node, or moves it if the label exists"#,
    kind: CommandKind::Model,
    run: node_add,
};

pub const NODE_REMOVE: CommandSpec = CommandSpec {
    id: "mesh.node.remove",
    label: "Remove Node",
    params: r#"{"label": 1} — only a node no element uses"#,
    kind: CommandKind::Model,
    run: node_remove,
};

pub const ELEMENT_ADD: CommandSpec = CommandSpec {
    id: "mesh.element.add",
    label: "Add Element",
    params: r#"{"label": 1, "kind": "rod2|beam2|tri3|tri6|quad4|quad8|tet4|tet10|wedge6|hex8|hex20", "nodes": [labels]} — adds or replaces"#,
    kind: CommandKind::Model,
    run: element_add,
};

pub const ELEMENT_REMOVE: CommandSpec =
    CommandSpec { id: "mesh.element.remove", label: "Remove Element", params: r#"{"label": 1}"#, kind: CommandKind::Model, run: element_remove };

pub const QUALITY: CommandSpec = CommandSpec {
    id: "mesh.quality",
    label: "Mesh Quality",
    params: r#"{"elements": [labels] (optional, default all), "bins": 10 (1..=1000)} — certified validity (det J > 0 everywhere), volume, and min/max/mean/worst/histogram per metric (angles in degrees; histograms over each metric's fixed range, out-of-range values in the end bins)"#,
    kind: CommandKind::Model,
    run: quality_report,
};

pub const DELAUNAY_POINTS: CommandSpec = CommandSpec {
    id: "mesh.delaunay.points",
    label: "Delaunay of Points",
    params: r#"{"points": [[x, y, z], ...] (at most 20000)} — the exact Delaunay tetrahedralization (a debugging report; the model is not changed): {"dimension", "vertices", "duplicates": [[point, kept point], ...], "tets": [[i, j, k, l], ...]} with 0-based point indices in tet4 order; collinear or coplanar points give no tets"#,
    kind: CommandKind::Model,
    run: delaunay_points,
};

/// The most points `mesh.delaunay.points` takes (its report lists every tetrahedron).
const DELAUNAY_MAX_POINTS: usize = 20_000;

fn delaunay_points(_: &mut Session, p: &Value) -> Result<Value> {
    let points = points(p, "points", DELAUNAY_MAX_POINTS)?;
    let tri = Triangulation::new(&points)?;
    tri.verify()?;
    let duplicates: Vec<[u32; 2]> = (0..points.len() as u32).filter_map(|v| tri.duplicate_of(v).map(|d| [v, d])).collect();
    let tets: Vec<[u32; 4]> = tri.finite_tets().map(|(_, vs)| vs).collect();
    Ok(json!({
        "dimension": tri.dimension(),
        "vertices": points.len() - duplicates.len(),
        "duplicates": duplicates,
        "tets": tets,
    }))
}

pub const SURFACE_CHECK: CommandSpec = CommandSpec {
    id: "mesh.surface.check",
    label: "Check Boundary Surface",
    params: r#"{"body": "Body1", "tolerance": 0.05} or {"surface": 0}, optional "weld": 1e-9 (0 or 1e-15..=1e-3 of the bounding-box diagonal), "weld_distance": mm (default: the surface's own, twice SolidCraft's linear tolerance for a tessellated body, 0 for an imported mesh), "feature_angle": 45 (degrees, (0, 180]) — welds the tessellated body or imported surface and checks it can bound a volume mesh: closed, manifold, outward, no self-intersections; reports shells, genus, volume and feature edges, or the located defects"#,
    kind: CommandKind::Model,
    run: surface_check,
};

pub const TET_GENERATE: CommandSpec = CommandSpec {
    id: "mesh.tet.generate",
    label: "Generate Tet Mesh",
    params: r#"{"body": "Body1", "tolerance": 0.05} or {"surface": 0}, the boundary options of mesh.surface.check ("weld", "weld_distance", "feature_angle"), optional "max_steiner": 50000 (0..=10000000), "max_tets": 20000000 (1..=50000000) — the constrained Delaunay tetrahedralization of the welded boundary: every boundary triangle is a union of tet faces, Steiner points only on boundary edges, no quality refinement yet. Adds tet4 elements and their nodes (labels after the largest existing ones), tags the elements with the body and each boundary element face with its geometry face, and fails, changing nothing, if the surface is invalid or the mesh rounded to doubles does not pass the validity gate"#,
    kind: CommandKind::Model,
    run: tet_generate,
};

/// The first node and element labels of a mesh of `nodes` nodes and `elements` elements added
/// after the model's largest labels, or an error when the labels would pass [`MAX_LABEL`].
fn next_labels(s: &Session, nodes: usize, elements: usize) -> Result<(u32, u32)> {
    let next = |last: Option<u32>| last.map_or(Some(1), |l| l.checked_add(1));
    let fits = |first: Option<u32>, count: usize| first.filter(|&f| u64::from(f) + count as u64 - 1 <= u64::from(MAX_LABEL));
    match (fits(next(s.model.nodes().keys().next_back().copied()), nodes), fits(next(s.model.elements().keys().next_back().copied()), elements)) {
        (Some(n), Some(e)) => Ok((n, e)),
        _ => Err(EngineError::Params(format!("the mesh needs labels beyond {MAX_LABEL}"))),
    }
}

/// The most Steiner points and tetrahedra `mesh.tet.generate` may be asked for.
const TET_MAX_STEINER: u64 = 10_000_000;
const TET_MAX_TETS: u64 = 50_000_000;

/// An optional whole-number parameter in `range`.
fn optional_count(p: &Value, key: &str, default: u64, range: std::ops::RangeInclusive<u64>) -> Result<usize> {
    let v = match p.get(key) {
        None => default,
        Some(v) => v
            .as_u64()
            .filter(|n| range.contains(n))
            .ok_or_else(|| EngineError::Params(format!("`{key}` must be a whole number in {}..={}", range.start(), range.end())))?,
    };
    usize::try_from(v).map_err(|_| EngineError::Params(format!("`{key}` is too large")))
}

/// The boundary options of `mesh.surface.check` and `mesh.tet.generate`.
fn surface_options(p: &Value, mesh: &SurfaceMesh) -> Result<SurfaceOptions> {
    let defaults = SurfaceOptions::default();
    Ok(SurfaceOptions {
        weld_tolerance: optional_number(p, "weld", defaults.weld_tolerance)?,
        weld_distance: optional_number(p, "weld_distance", mesh.weld_distance)?,
        feature_angle: optional_number(p, "feature_angle", defaults.feature_angle)?,
    })
}

fn tet_generate(s: &mut Session, p: &Value) -> Result<Value> {
    let defaults = CdtOptions::default();
    let cdt_options = CdtOptions {
        max_steiner: optional_count(p, "max_steiner", defaults.max_steiner as u64, 0..=TET_MAX_STEINER)?,
        max_tets: optional_count(p, "max_tets", defaults.max_tets as u64, 1..=TET_MAX_TETS)?,
    };
    let (surface, body, face_names) = {
        let Input { mesh, body, faces } = surface_input(s, p)?;
        let options = surface_options(p, &mesh)?;
        (BoundarySurface::new(&mesh.positions, &mesh.triangles, &mesh.tri_face, &options)?, body, faces)
    };
    let cdt = tetrahedralize(&surface, &cdt_options)?;
    let gate = cdt.gate(surface.volume())?.into_result()?;

    let (first_node, first_element) = next_labels(s, cdt.points.len(), cdt.tets.len())?;
    for (i, xyz) in cdt.points.iter().enumerate() {
        s.model.set_node(first_node + i as u32, *xyz)?;
    }
    let body_tag = s.model.geometry_tag(&GeometryRef::body(body.clone())?)?;
    for (i, t) in cdt.tets.iter().enumerate() {
        let label = first_element + i as u32;
        s.model.set_element(label, ElementKind::Tet4, t.iter().map(|&v| first_node + v).collect())?;
        s.model.tag_element(label, body_tag)?;
    }
    let mut face_tags: Vec<Option<u32>> = vec![None; face_names.len()];
    for b in &cdt.boundary {
        let face = surface.tri_face().get(b.triangle as usize).copied().unwrap_or(u32::MAX) as usize;
        let tag = match face_tags.get(face).copied().flatten() {
            Some(t) => t,
            None => {
                let name =
                    face_names.get(face).ok_or_else(|| EngineError::Params(format!("boundary triangle {} has no geometry face", b.triangle)))?;
                let t = s.model.geometry_tag(&GeometryRef::face(body.clone(), name.clone())?)?;
                if let Some(slot) = face_tags.get_mut(face) {
                    *slot = Some(t);
                }
                t
            }
        };
        let number = b.tet4_face().ok_or_else(|| EngineError::Params(format!("boundary face {} has no tet4 face number", b.opposite)))?;
        s.model.tag_element_face(first_element + b.tet, number, tag)?;
    }
    Ok(json!({
        "body": body,
        "nodes": cdt.points.len(),
        "elements": cdt.tets.len(),
        "first_node": first_node,
        "first_element": first_element,
        "steiner": cdt.steiner.len(),
        "cavities": cdt.stats.cavities,
        "boundary_faces": cdt.boundary.len(),
        "volume": gate.volume,
        "boundary_volume": surface.volume(),
    }))
}

pub const PLANAR: CommandSpec = CommandSpec {
    id: "mesh.planar",
    label: "2D Mesh",
    params: r#"{"size": 1.0, "kind": "quad4|quad8|tri3|tri6" (default quad4), and either "polygon": [[x, y], ...] with optional "holes": [[[x, y], ...], ...] and "name": "Plane" (the body name its tags use), or "body": "Body1", "face": "F2:top", "tolerance": 0.05 (a planar face of an open design, parallel to XY); optional "max_elements": 1000000 (1..=4000000)} — meshes the planar region with 2D elements in the XY plane (a face at z = c is meshed at z = 0) for plane stress, plane strain or axisymmetric sections (x the radius, Y the axis): free quadrilaterals (or constrained Delaunay triangles) of about `size`, mid-side nodes on the boundary for quadratic kinds. Adds nodes and elements after the largest existing labels and tags them: the elements with the face (polygon: face `area` of body `name`), the boundary element edges with edges named after the neighbouring face of the body (polygon: `e1`, `e2`, … side by side, the outer loop then each hole, and vertices `v1`, … at the corners), so sets on those edges carry loads and boundary conditions"#,
    kind: CommandKind::Model,
    run: planar,
};

/// Most points of a `mesh.planar` polygon or hole, and most holes.
const PLANAR_MAX_POINTS: usize = 100_000;
const PLANAR_MAX_HOLES: usize = 10_000;
const PLANAR_MAX_ELEMENTS: u64 = 4_000_000;

/// A list of `[x, y]` points.
fn polygon(v: &Value, what: &str) -> Result<Vec<[f64; 2]>> {
    let bad = || EngineError::Params(format!("`{what}` must be a list of 3 to {PLANAR_MAX_POINTS} [x, y] points"));
    let raw = v.as_array().filter(|a| (3..=PLANAR_MAX_POINTS).contains(&a.len())).ok_or_else(bad)?;
    raw.iter()
        .map(|p| match p.as_array().map(Vec::as_slice) {
            Some([x, y]) => x.as_f64().zip(y.as_f64()).map(|(x, y)| [x, y]).filter(|c| c.iter().all(|v| v.is_finite())),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(bad)
}

fn planar(s: &mut Session, p: &Value) -> Result<Value> {
    use modelcraft_mesh::planar::{Curve, face_boundary, mesh_region};
    use modelcraft_model::Entity;

    let size = p
        .get("size")
        .and_then(Value::as_f64)
        .filter(|v| *v > 0.0 && v.is_finite())
        .ok_or_else(|| EngineError::Params("`size` must be a positive number (the element size)".into()))?;
    let kind = match p.get("kind") {
        None => ElementKind::Quad4,
        Some(k) => k
            .as_str()
            .and_then(ElementKind::from_name)
            .filter(|k| k.dimension() == 2)
            .ok_or_else(|| EngineError::Params("`kind` must be quad4, quad8, tri3 or tri6".into()))?,
    };
    let opts = modelcraft_mesh::quad::QuadOptions {
        max_elements: optional_count(p, "max_elements", 1_000_000, 1..=PLANAR_MAX_ELEMENTS)?,
        ..Default::default()
    };
    // The region's loops of curves, the body name, the face its elements fill, and each curve
    // id's edge name.
    let (loops, body, face, names): (Vec<Vec<Curve>>, String, String, BTreeMap<u32, String>) = match (p.get("polygon"), p.get("body")) {
        (Some(outer), None) => {
            let mut rings = vec![polygon(outer, "polygon")?];
            if let Some(h) = p.get("holes") {
                let list = h
                    .as_array()
                    .filter(|a| a.len() <= PLANAR_MAX_HOLES)
                    .ok_or_else(|| EngineError::Params(format!("`holes` must be a list of at most {PLANAR_MAX_HOLES} polygons")))?;
                for hole in list {
                    rings.push(polygon(hole, "holes")?);
                }
            }
            let name = match p.get("name") {
                None => "Plane".to_string(),
                Some(n) => n.as_str().map(str::to_string).ok_or_else(|| EngineError::Params("`name` must be a body name".into()))?,
            };
            GeometryRef::body(name.clone())?;
            let mut id = 0u32;
            let mut names = BTreeMap::new();
            let loops = rings
                .into_iter()
                .map(|ring| {
                    ring.into_iter()
                        .map(|pt| {
                            id += 1;
                            names.insert(id, format!("e{id}"));
                            Curve { id: Some(id), points: vec![pt] }
                        })
                        .collect()
                })
                .collect();
            (loops, name, "area".to_string(), names)
        }
        (None, Some(_)) => {
            let face_name = p.get("face").and_then(Value::as_str).ok_or_else(|| EngineError::Params("`face` must be a face name".into()))?;
            let (shape, mesh) = super::geometry::body_mesh(s, p)?;
            let face = shape.face_index(face_name).ok_or_else(|| EngineError::Params(format!("body `{}` has no face `{face_name}`", shape.name)))?;
            let b = face_boundary(&mesh.positions, &mesh.triangles, &mesh.tri_face, face, mesh.weld_distance)?;
            let names = shape.faces().iter().enumerate().map(|(i, f)| (i as u32, f.name.clone())).collect();
            (b.loops, shape.name.clone(), face_name.to_string(), names)
        }
        _ => return Err(EngineError::Params("give either `polygon` (and `holes`) or `body` with `face` and `tolerance`".into())),
    };
    let m = mesh_region(&loops, kind, size, &opts)?;

    let (first_node, first_element) = next_labels(s, m.nodes.len(), m.elements.len())?;
    for (i, xy) in m.nodes.iter().enumerate() {
        s.model.set_node(first_node + i as u32, [xy[0], xy[1], 0.0])?;
    }
    let region = s.model.geometry_tag(&GeometryRef::face(body.clone(), face.clone())?)?;
    for (i, nodes) in m.elements.iter().enumerate() {
        let label = first_element + i as u32;
        s.model.set_element(label, kind, nodes.iter().map(|v| first_node + v).collect())?;
        s.model.tag_element(label, region)?;
    }
    let mut tags: BTreeMap<u32, u32> = BTreeMap::new();
    for (e, edge, id) in &m.edges {
        let Some(name) = names.get(id) else { continue };
        let tag = match tags.get(id) {
            Some(t) => *t,
            None => {
                let t = s.model.geometry_tag(&GeometryRef::new(body.clone(), Entity::Edge(name.clone()))?)?;
                tags.insert(*id, t);
                t
            }
        };
        s.model.tag_element_edge(first_element + *e as u32, *edge, tag)?;
    }
    let polygon_input = p.get("polygon").is_some();
    if polygon_input {
        for (node, id) in &m.corners {
            let t = s.model.geometry_tag(&GeometryRef::new(body.clone(), Entity::Vertex(format!("v{id}")))?)?;
            s.model.tag_node(first_node + node, t)?;
        }
    }
    let edges: Vec<&String> = tags.keys().filter_map(|id| names.get(id)).collect();
    Ok(json!({
        "body": body,
        "face": face,
        "kind": kind.name(),
        "nodes": m.nodes.len(),
        "elements": m.elements.len(),
        "first_node": first_node,
        "first_element": first_element,
        "edges": edges,
    }))
}

fn node_add(s: &mut Session, p: &Value) -> Result<Value> {
    let l = label(p, "label")?;
    s.model.set_node(l, xyz(p, "xyz")?)?;
    Ok(json!({"label": l}))
}

fn node_remove(s: &mut Session, p: &Value) -> Result<Value> {
    s.model.remove_node(label(p, "label")?)?;
    Ok(Value::Null)
}

fn element_add(s: &mut Session, p: &Value) -> Result<Value> {
    let l = label(p, "label")?;
    let name = p.get("kind").and_then(Value::as_str).unwrap_or_default();
    let kind = ElementKind::from_name(name).ok_or_else(|| EngineError::Params(format!("unknown element kind `{name}`")))?;
    let nodes = labels(p, "nodes", ElementKind::Hex20.node_count())?;
    if nodes.len() != kind.node_count() {
        return Err(EngineError::Params(format!("{name} needs {} nodes, got {}", kind.node_count(), nodes.len())));
    }
    s.model.set_element(l, kind, nodes)?;
    Ok(json!({"label": l}))
}

fn element_remove(s: &mut Session, p: &Value) -> Result<Value> {
    s.model.remove_element(label(p, "label")?)?;
    Ok(Value::Null)
}

/// A JSON number, or a string for ±∞ (JSON has no infinity).
fn number(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { json!(if v > 0.0 { "inf" } else { "-inf" }) }
}

fn quality_report(s: &mut Session, p: &Value) -> Result<Value> {
    let bins = match p.get("bins") {
        None => 10,
        Some(v) => v.as_u64().and_then(|b| usize::try_from(b).ok()).ok_or_else(|| EngineError::Params("`bins` must be a whole number".into()))?,
    };
    let selection = match p.get("elements") {
        None => None,
        Some(_) => Some(labels(p, "elements", MAX_ELEMENTS)?),
    };
    let r: QualityReport = quality::quality_report(&s.model, selection.as_deref(), bins)?;
    let skipped: serde_json::Map<String, Value> = r.skipped.iter().map(|(k, n)| (k.name().to_string(), json!(n))).collect();
    let metrics: serde_json::Map<String, Value> = r
        .metrics
        .iter()
        .map(|m| {
            let (lo, hi) = m.metric.range();
            let stats = json!({
                "count": m.count,
                "min": number(m.min),
                "max": number(m.max),
                "mean": m.mean.map(number),
                "worst": m.worst,
                "histogram": {"range": [lo, hi], "counts": m.histogram},
            });
            (m.metric.name().to_string(), stats)
        })
        .collect();
    Ok(json!({
        "measured": r.measured,
        "skipped": skipped,
        "valid": r.valid,
        "invalid": listed(&r.invalid),
        "undetermined": listed(&r.undetermined),
        "volume": r.volume,
        "area": r.area,
        "metrics": metrics,
    }))
}

/// A boundary to mesh: the surface, with the body name and the geometry name of each face index
/// for tagging.
struct Input<'a> {
    mesh: Cow<'a, SurfaceMesh>,
    body: String,
    faces: Vec<String>,
}

/// The surface named by `body` (tessellated at `tolerance`) or `surface` (an imported index).
fn surface_input<'a>(s: &'a Session, p: &Value) -> Result<Input<'a>> {
    match (p.get("body"), p.get("surface")) {
        (Some(_), None) => {
            let (shape, mesh) = super::geometry::body_mesh(s, p)?;
            Ok(Input { mesh: Cow::Owned(mesh), body: shape.name.clone(), faces: shape.faces().iter().map(|f| f.name.clone()).collect() })
        }
        (None, Some(i)) => {
            let n = s.surfaces.len();
            let t = i
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| s.surfaces.get(i))
                .ok_or_else(|| EngineError::Params(format!("`surface` must be the index of an imported surface (there are {n})")))?;
            Ok(Input { mesh: Cow::Borrowed(&t.mesh), body: t.name.clone(), faces: (0..t.faces).map(|f| f.to_string()).collect() })
        }
        _ => Err(EngineError::Params("give either `body` (with `tolerance`) or `surface`".into())),
    }
}

fn defect_json(d: &Defect) -> Value {
    json!({"kind": d.kind(), "message": d.to_string(), "at": d.at()})
}

fn surface_check(s: &mut Session, p: &Value) -> Result<Value> {
    let mesh = surface_input(s, p)?.mesh;
    let options = surface_options(p, &mesh)?;
    let surface = match BoundarySurface::new(&mesh.positions, &mesh.triangles, &mesh.tri_face, &options) {
        Ok(surface) => surface,
        Err(modelcraft_mesh::Error::Surface(d)) => {
            return Ok(json!({"valid": false, "defects": {"count": d.total, "listed": d.listed.iter().map(defect_json).collect::<Vec<_>>()}}));
        }
        Err(e) => return Err(e.into()),
    };
    let mut faces: Vec<u32> = surface.tri_face().to_vec();
    faces.sort_unstable();
    faces.dedup();
    let count = |k: FeatureKind| surface.feature_edges().iter().filter(|e| e.kind == k).count();
    let shells: Vec<Value> = surface
        .shells()
        .iter()
        .map(|sh| json!({"triangles": sh.triangles, "vertices": sh.vertices, "genus": sh.genus, "volume": sh.volume, "void": sh.void}))
        .collect();
    Ok(json!({
        "valid": true,
        "vertices": surface.points().len(),
        "triangles": surface.triangles().len(),
        "input": {"vertices": mesh.positions.len(), "triangles": mesh.triangles.len()},
        "faces": faces.len(),
        "volume": surface.volume(),
        "shells": shells,
        "feature_edges": {"face_boundary": count(FeatureKind::FaceBoundary), "sharp": count(FeatureKind::Sharp)},
        "feature_corners": surface.feature_corners().len(),
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn hex(s: &mut Session, label: u32, first_node: u32, mirror: bool) {
        let corners = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]];
        for (i, c) in corners.iter().enumerate() {
            let x = if mirror { -c[0] - 2 } else { c[0] };
            s.execute("mesh.node.add", &json!({"label": first_node + i as u32, "xyz": [x, c[1], c[2]]})).unwrap();
        }
        let nodes: Vec<u32> = (first_node..first_node + 8).collect();
        s.execute("mesh.element.add", &json!({"label": label, "kind": "hex8", "nodes": nodes})).unwrap();
    }

    #[test]
    fn delaunay_of_points_reports_tets_and_duplicates() {
        let mut s = Session::default();
        let cube = json!({"points": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1], [1, 1, 1]]});
        let r = s.execute("mesh.delaunay.points", &cube).unwrap();
        assert_eq!(r["dimension"], 3);
        assert_eq!(r["vertices"], 8);
        assert_eq!(r["duplicates"], json!([[8, 7]]));
        let tets = r["tets"].as_array().unwrap();
        assert!((5..=6).contains(&tets.len()), "{tets:?}");
        assert!(s.journal.is_empty(), "a report does not change the model");

        let flat = s.execute("mesh.delaunay.points", &json!({"points": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0]]})).unwrap();
        assert_eq!((flat["dimension"].clone(), flat["tets"].clone()), (json!(2), json!([])));
        for bad in [
            json!({}),
            json!({"points": [[0, 0]]}),
            json!({"points": [[0, 0, "x"]]}),
            json!({"points": [[f64::MAX, 0, 0]]}),
            json!({"points": [[1e-300, 0, 0]]}),
            json!({"points": vec![[0, 0, 0]; 20_001]}),
        ] {
            assert!(s.execute("mesh.delaunay.points", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reports_quality_and_inverted_elements() {
        let mut s = Session::default();
        hex(&mut s, 1, 1, false);
        hex(&mut s, 2, 11, true);
        let journal = s.journal.len();
        let r = s.execute("mesh.quality", &json!({"bins": 2})).unwrap();
        assert_eq!(r["measured"], 2);
        assert_eq!(r["valid"], 1);
        assert_eq!(r["invalid"], json!({"count": 1, "labels": [2]}));
        assert_eq!(r["volume"], 0.0);
        let sj = &r["metrics"]["scaled_jacobian"];
        assert_eq!((sj["min"].clone(), sj["max"].clone(), sj["worst"].clone()), (json!(-1.0), json!(1.0), json!(2)));
        assert_eq!(sj["histogram"]["counts"], json!([1, 1]));
        assert_eq!(s.journal.len(), journal, "a report does not change the model");

        let one = s.execute("mesh.quality", &json!({"elements": [1, 1]})).unwrap();
        assert_eq!((one["measured"].clone(), one["volume"].clone()), (json!(1), json!(1.0)));
        assert!(s.execute("mesh.quality", &json!({"elements": [3]})).is_err());
        assert!(s.execute("mesh.quality", &json!({"elements": [0]})).is_err());
        assert!(s.execute("mesh.quality", &json!({"bins": 0})).is_err());
        assert!(s.execute("mesh.quality", &json!({"bins": 1e9})).is_err());
        assert!(s.execute("mesh.quality", &json!({"elements": "all"})).is_err());
    }

    #[test]
    fn a_tessellated_block_is_a_valid_boundary() {
        use modelcraft_mesh::surface::{BoundarySurface, FeatureKind, SurfaceOptions};
        let mesh = modelcraft_geometry::block([0.0; 3], [2.0, 3.0, 4.0]).unwrap().tessellate(0.01).unwrap();
        let options = SurfaceOptions { weld_distance: mesh.weld_distance, ..SurfaceOptions::default() };
        let s = BoundarySurface::new(&mesh.positions, &mesh.triangles, &mesh.tri_face, &options).unwrap();
        assert!(s.points().len() < mesh.positions.len(), "the tessellation is a soup that welding joins");
        assert_eq!((s.shells().len(), s.shells()[0].genus), (1, 0));
        assert!((s.volume() - 24.0).abs() < 1e-9, "{}", s.volume());
        assert!(s.feature_edges().iter().all(|e| e.kind == FeatureKind::FaceBoundary));
        assert_eq!(s.feature_corners().len(), 8);
        let length: f64 = s
            .feature_edges()
            .iter()
            .map(|e| {
                let [a, b] = e.vertices.map(|v| s.points()[v as usize]);
                ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
            })
            .sum();
        assert!((length - 4.0 * (2.0 + 3.0 + 4.0)).abs() < 1e-9, "the twelve edges: {length}");
    }

    #[test]
    fn surface_check_on_an_open_design_and_on_imported_surfaces() {
        use std::sync::Arc;

        use modelcraft_geometry::sample::{Plate, plate_design};
        use modelcraft_geometry::tagged_mesh::TaggedSurface;
        use modelcraft_geometry::{SurfaceMesh, open_design};

        let plate = Plate::default();
        let mut s = Session { design: Some(Arc::new(open_design(&plate_design(&plate).unwrap()).unwrap())), ..Session::default() };
        let r = s.execute("mesh.surface.check", &json!({"body": "Body1", "tolerance": 0.05})).unwrap();
        assert_eq!(r["valid"], true, "{r}");
        assert_eq!(r["faces"], 8);
        assert_eq!(r["shells"].as_array().unwrap().len(), 1);
        assert_eq!(r["shells"][0]["genus"], 1, "the hole is a handle");
        let exact = (plate.width * plate.depth - std::f64::consts::PI * (plate.hole_diameter / 2.0).powi(2)) * plate.thickness;
        let volume = r["volume"].as_f64().unwrap();
        assert!(volume >= exact && volume < exact * 1.01, "a polygonal hole inscribed in the circle: {volume} vs {exact}");
        assert!(r["feature_edges"]["face_boundary"].as_u64().unwrap() > 12);
        assert!(r["feature_corners"].as_u64().unwrap() >= 8);
        assert!(s.journal.is_empty(), "a check changes nothing");

        // A tagged tetrahedron, and the same with a face missing.
        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let t = vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]];
        let tet = SurfaceMesh { positions: p.clone(), triangles: t.clone(), tri_face: vec![0, 1, 1, 1], weld_distance: 0.0 };
        let open = SurfaceMesh { positions: p, triangles: t[..3].to_vec(), tri_face: vec![0, 1, 1], weld_distance: 0.0 };
        for mesh in [tet, open] {
            s.surfaces.push(Arc::new(TaggedSurface { name: "Tet".into(), mesh, faces: 2 }));
        }
        let r = s.execute("mesh.surface.check", &json!({"surface": 0, "feature_angle": 180})).unwrap();
        assert_eq!((r["valid"].clone(), r["feature_edges"].clone()), (json!(true), json!({"face_boundary": 3, "sharp": 0})));
        let r = s.execute("mesh.surface.check", &json!({"surface": 1})).unwrap();
        assert_eq!(r["valid"], false);
        assert_eq!(r["defects"]["count"], 3);
        assert_eq!(r["defects"]["listed"][0]["kind"], "open_edge");
        assert!(r["defects"]["listed"][0]["at"].is_array());

        for bad in [
            json!({"surface": 2}),
            json!({"surface": -1}),
            json!({"surface": 0, "body": "Body1"}),
            json!({}),
            json!({"surface": 0, "weld": 1.0}),
            json!({"surface": 0, "weld": "fine"}),
            json!({"surface": 0, "weld_distance": -1}),
            json!({"surface": 0, "weld_distance": 1}),
            json!({"surface": 0, "feature_angle": 0}),
            json!({"body": "Body1"}),
            json!({"body": "Nope", "tolerance": 0.05}),
        ] {
            assert!(s.execute("mesh.surface.check", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn tet_generate_meshes_a_body_with_tags_and_appends() {
        use std::sync::Arc;

        use modelcraft_geometry::open_design;
        use modelcraft_geometry::sample::{Plate, plate_design};

        let plate = Plate::default();
        let mut s = Session { design: Some(Arc::new(open_design(&plate_design(&plate).unwrap()).unwrap())), ..Session::default() };
        let r = s.execute("mesh.tet.generate", &json!({"body": "Body1", "tolerance": 0.5})).unwrap();
        let elements = r["elements"].as_u64().unwrap() as usize;
        assert!(elements > 0, "{r}");
        assert_eq!(s.model.elements().len(), elements);
        assert!(s.model.elements().values().all(|e| e.kind == modelcraft_model::ElementKind::Tet4));
        let (volume, boundary) = (r["volume"].as_f64().unwrap(), r["boundary_volume"].as_f64().unwrap());
        assert!((volume - boundary).abs() <= 1e-9 * boundary, "{r}");
        // Every element is on the body; boundary faces carry the 8 face names.
        let c = s.model.classification();
        assert_eq!(c.elements().len(), elements);
        let faces: std::collections::BTreeSet<u32> = c.elements().values().flat_map(|t| t.faces.values().copied()).collect();
        assert_eq!(faces.len(), 8);
        let tagged: usize = c.elements().values().map(|t| t.faces.len()).sum();
        assert_eq!(tagged as u64, r["boundary_faces"].as_u64().unwrap());
        // The certified quality report agrees.
        let q = s.execute("mesh.quality", &json!({})).unwrap();
        assert_eq!(q["valid"].as_u64().unwrap() as usize, elements);
        // A second run appends after the existing labels.
        let again = s.execute("mesh.tet.generate", &json!({"body": "Body1", "tolerance": 0.5})).unwrap();
        assert_eq!(again["first_element"].as_u64().unwrap() as usize, elements + 1);
        assert_eq!(s.model.elements().len(), 2 * elements);
        assert_eq!(s.journal.len(), 2);

        let journal = s.journal.len();
        let model = s.model.clone();
        for bad in [
            json!({"body": "Body1", "tolerance": 0.5, "max_steiner": -1}),
            json!({"body": "Body1", "tolerance": 0.5, "max_steiner": 1e300}),
            json!({"body": "Body1", "tolerance": 0.5, "max_tets": 0}),
            json!({"body": "Body1", "tolerance": 0.5, "max_tets": 10}),
            json!({"body": "Body1", "tolerance": 0.5, "max_tets": "many"}),
            json!({"body": "Body1"}),
            json!({"body": "Nope", "tolerance": 0.5}),
            json!({"surface": 0}),
            json!({}),
        ] {
            assert!(s.execute("mesh.tet.generate", &bad).is_err(), "{bad}");
        }
        assert_eq!((s.journal.len(), &s.model), (journal, &model), "failures change nothing");
    }

    #[test]
    fn tet_generate_on_an_imported_surface_and_an_open_one() {
        use std::sync::Arc;

        use modelcraft_geometry::SurfaceMesh;
        use modelcraft_geometry::tagged_mesh::TaggedSurface;

        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let t = vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]];
        let tet = SurfaceMesh { positions: p.clone(), triangles: t.clone(), tri_face: vec![0, 1, 1, 1], weld_distance: 0.0 };
        let open = SurfaceMesh { positions: p, triangles: t[..3].to_vec(), tri_face: vec![0, 1, 1], weld_distance: 0.0 };
        let mut s = Session::default();
        for mesh in [tet, open] {
            s.surfaces.push(Arc::new(TaggedSurface { name: "Tet".into(), mesh, faces: 2 }));
        }
        let r = s.execute("mesh.tet.generate", &json!({"surface": 0})).unwrap();
        assert_eq!((r["elements"].clone(), r["nodes"].clone(), r["boundary_faces"].clone()), (json!(1), json!(4), json!(4)));
        let element = s.model.element(1).unwrap();
        assert_eq!(element.nodes.len(), 4);
        assert!(s.execute("mesh.tet.generate", &json!({"surface": 1})).is_err(), "an open surface bounds no volume");
        assert_eq!(s.model.elements().len(), 1);
    }

    #[test]
    fn planar_meshes_a_polygon_that_solves_as_a_2d_model() {
        let mut s = Session::default();
        let hole: Vec<[f64; 2]> = (0..12).map(|k| std::f64::consts::TAU * k as f64 / 12.0).map(|t| [5.0 + t.cos(), 2.0 + t.sin()]).collect();
        let r = s
            .execute(
                "mesh.planar",
                &json!({"polygon": [[0, 0], [10, 0], [10, 4], [0, 4]], "holes": [hole], "size": 0.5, "kind": "quad8", "name": "Strip"}),
            )
            .unwrap();
        assert_eq!(r["kind"], "quad8");
        let elements = r["elements"].as_u64().unwrap() as usize;
        assert!(elements > 20, "{r}");
        assert!(s.model.elements().values().all(|e| e.kind == modelcraft_model::ElementKind::Quad8));
        assert!(s.model.nodes().values().all(|n| n.xyz[2] == 0.0));
        // Sets on the tagged edges carry the boundary condition and the load.
        for (cmd, params) in [
            ("model.set.create", json!({"name": "all", "geometry": [{"body": "Strip", "face": "area"}]})),
            ("model.set.create", json!({"name": "left", "geometry": [{"body": "Strip", "edge": "e4"}]})),
            ("model.set.create", json!({"name": "right", "geometry": [{"body": "Strip", "edge": "e2"}]})),
            ("model.material.create", json!({"name": "Steel", "elastic": {"type": "isotropic", "young": 210000.0, "poisson": 0.3}})),
            ("model.section.create", json!({"name": "plate", "set": "all", "material": "Steel", "kind": {"type": "plane_stress", "thickness": 2.0}})),
            ("model.boundary_condition.create", json!({"name": "hold", "type": "fixed", "target": {"set": "left"}})),
            ("model.load.create", json!({"name": "pull", "type": "traction", "set": "right", "vector": [10.0, 0.0, 0.0]})),
            ("model.step.create", json!({"name": "Pull", "procedure": {"type": "static"}, "boundary_conditions": ["hold"], "loads": ["pull"]})),
        ] {
            s.execute(cmd, &params).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        }
        let r = s.execute("solve.static", &json!({})).unwrap();
        let total: f64 = r["reaction_total"][0].as_f64().unwrap();
        // The traction over the right edge: 10 × 4 × 2.
        assert!((total + 80.0).abs() < 1e-9, "{r}");
        // A second mesh appends; failures change nothing.
        let journal = s.journal.len();
        let model = s.model.clone();
        for bad in [
            json!({"polygon": [[0, 0], [1, 0], [0, 1]], "size": 0}),
            json!({"polygon": [[0, 0], [1, 0], [0, 1]], "size": 1e-9}),
            json!({"polygon": [[0, 0], [1, 0], [0, 1]], "size": 0.1, "kind": "hex8"}),
            json!({"polygon": [[0, 0], [1, 0]], "size": 0.1}),
            json!({"polygon": [[0, 0], [1, 0], [1, 1], [0, 1]], "holes": [[[2, 2], [3, 2], [3, 3]]], "size": 0.1}),
            json!({"polygon": [[0, 0], [1, 1], [1, 0], [0, 1]], "size": 0.1}),
            json!({"polygon": [[0, 0], [1, 0], [0, 1]], "size": 0.1, "max_elements": 0}),
            json!({"polygon": [[0, 0], [1, 0], [0, 1]], "size": 0.1, "name": ""}),
            json!({"polygon": [[0, 0], [1, 0], [0, 1]], "size": 0.1, "body": "Body1", "face": "x"}),
            json!({"body": "Body1", "face": "x", "tolerance": 0.1, "size": 1}),
            json!({"size": 1}),
        ] {
            assert!(s.execute("mesh.planar", &bad).is_err(), "{bad}");
        }
        assert_eq!((s.journal.len(), &s.model), (journal, &model), "failures change nothing");
    }

    #[test]
    fn planar_meshes_a_flat_face_of_a_design_with_edges_named_by_their_neighbours() {
        use std::sync::Arc;

        use modelcraft_geometry::open_design;
        use modelcraft_geometry::sample::{Plate, plate_design};

        let design = open_design(&plate_design(&Plate::default()).unwrap()).unwrap();
        let names: Vec<String> = design.bodies[0].faces().iter().map(|f| f.name.clone()).collect();
        let mut s = Session { design: Some(Arc::new(design)), ..Session::default() };
        let mut meshed = 0;
        for face in &names {
            let Ok(r) = s.execute("mesh.planar", &json!({"body": "Body1", "face": face, "tolerance": 0.2, "size": 4.0, "kind": "tri6"})) else {
                continue;
            };
            meshed += 1;
            let edges: Vec<&str> = r["edges"].as_array().unwrap().iter().map(|e| e.as_str().unwrap()).collect();
            // The plate's top and bottom faces border its four sides and the hole.
            assert!(edges.len() >= 5 && edges.iter().all(|e| names.iter().any(|n| n == e)), "{face}: {edges:?}");
        }
        assert_eq!(meshed, 2, "the top and bottom faces are flat and parallel to XY; the sides and hole are not");
    }
}
