use crate::schema::ProblemKind;
use std::collections::BTreeSet;
use std::sync::Arc;

use fieldcraft_geometry::tagged_mesh;
use fieldcraft_geometry::{Design, MAX_DESIGN_BYTES, Shape, SurfaceMesh, open_design};
use fieldcraft_io::SourceFile;
use fieldcraft_io::roots::Access;
use fieldcraft_model::Model;
use fieldcraft_model::geometry::{Entity, GeometryRef};
use serde_json::{Value, json};

use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const OPEN: CommandSpec = CommandSpec {
    id: "geometry.open",
    label: "Open Design",
    params: r#"{"path": "part.solidcraft", "model"?: "keep"} — rebuilds the SolidCraft design and makes its bodies the geometry. `model`: "keep" (default) keeps the mesh and every definition, so opening the same design again after an edit picks up the change and sets follow their persistent face names; "new" starts a new model with the design (no mesh, sets, materials, sections, BCs, loads or steps; the unit system stays; the model is untitled until saved, and undo restores the previous file). The reply's `related` is false when the model (before the open) refers to bodies (mesh tags, set geometry) and the design has none of them, and `stale` lists what no longer resolves: `mesh_bodies` (bodies the mesh is tagged with that the design lacks) `sets` (sets naming a body or face it lacks) and `seeds` (edge seeds on edges it lacks). `errors` lists the features that failed to rebuild with the `bodies` each may change, and `warnings` says so in one line each. One undo step"#,
    form: Some(super::forms::OPEN_DESIGN),
    kind: CommandKind::Model,
    requires: &[],
    run: open,
};

pub const BODIES: CommandSpec = CommandSpec {
    id: "geometry.bodies",
    label: "Bodies",
    params: "{} — the open design's bodies: measures, persistent face names (by face index) and edges (persistent name `<face>|<face>`, `~k` when several separate the same faces, and length) and vertices (persistent name `<face>|<face>|<face>` after the faces that meet there, `~k` when several touch the same faces, and point)",
    form: Some(&[]),
    kind: CommandKind::Model,
    requires: &[], // No design is an answer too: no bodies.
    run: bodies,
};

pub const FEATURES: CommandSpec = CommandSpec {
    id: "geometry.features",
    label: "Feature Graph",
    params: r#"{"body"?: "Base" (default: every body)} — the features of the SolidCraft timeline that built each body of the open design, in order (read-only): each feature's `id` (`F<id>` in persistent names), `name`, `type` (extrude, box, cylinder, hole, shell, fillet, combine, pattern, mirror, split, …), `operation` (new_body, join, cut, intersect), `values` (its numeric inputs evaluated: mm, radians, counts), `placement` (a primitive's corner, base or centre), `direction` (an extrude's along the sketch normal, a cylinder's axis, a hole's drilling direction), `two_sided`, `sketch` (plane origin, axes and normal, and the profiles used: lines, arcs and circles in sketch coordinates, holes, curve ids), `transforms` (pattern and mirror copies, 4×4 row-major), `targets` and `tools`, the stored `definition`, the persistent `faces` and `edges` of the body it made, and `error` when it failed to rebuild. A partition cell is described by the body it was cut from"#,
    form: Some(super::forms::FEATURES),
    kind: CommandKind::Model,
    requires: &[Requirement::Design],
    run: features,
};

pub const CENTRE: CommandSpec = CommandSpec {
    id: "geometry.centre",
    label: "Centre of Selection",
    params: r#"{"geometry": [{"body": "Body1", "face": "F3:hole"} | {"body", "edge"} | {"body", "vertex"}], "nodes": [labels]} — the point a reference point or datum point goes at for what is picked: a vertex's point; the mean of nodes; the centre of a hole (cylindrical faces on one axis, or circular edges on one circle: the axis point at their mid-depth, the circle's centre); otherwise the area centroid of the faces and the length centroid of the edges. Replies `point` and `from` (`vertex`, `nodes`, `circle`, `centroid`)"#,
    form: Some(super::forms::CENTRE),
    kind: CommandKind::Model,
    requires: &[],
    run: centre,
};

pub const TESSELLATE: CommandSpec = CommandSpec {
    id: "geometry.tessellate",
    label: "Tessellate Body",
    params: r#"{"body": "Body1", "tolerance": 0.05} — triangulates the body's boundary within the chordal tolerance (mm); reports triangles per face"#,
    form: Some(super::forms::TESSELLATE),
    kind: CommandKind::Model,
    requires: &[Requirement::Design],
    run: tessellate,
};

pub const IMPORT_MESH: CommandSpec = CommandSpec {
    id: "geometry.import.mesh",
    label: "Import Tagged Mesh",
    params: r#"{"path": "part.3mf"} — adds each build item of a 3MF package as a surface; triangles keep their B-rep face tags (docs/formats/tagged-mesh.md)"#,
    form: Some(super::forms::IMPORT_MESH),
    kind: CommandKind::Model,
    requires: &[],
    run: import_mesh,
};

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let new_model = match p.get("model") {
        None | Some(Value::Null) => false,
        Some(v) => match v.as_str() {
            Some("keep") => false,
            Some("new") => true,
            _ => return Err(EngineError::Params(r#"`model` must be "keep" or "new""#.into())),
        },
    };
    let path = super::file_param(s, p, "path", Access::Read)?;
    let bytes = super::file::read_file(&path, MAX_DESIGN_BYTES as u64)?;
    let source = SourceFile::of(super::file::absolute(&path), &bytes)?;
    let opened = open_design(&bytes)?;
    // The model's partitions cut the design again (a new model has none).
    let design = if new_model { opened } else { super::datum::partitioned(&opened, &s.model)? };
    let mut fit = fit(s, &design);
    if new_model {
        let mut model = Model::default();
        model.set_units(s.model.units());
        s.model = model;
        s.surfaces.clear();
        s.sources.meshes.clear();
        s.results_view = None;
        // A new model is untitled: saving asks for a file instead of overwriting the old model's
        // (undo brings the old file back with the old model).
        s.set_document(Default::default());
        fit.mesh_bodies.clear();
        fit.sets.clear();
        fit.seeds.clear();
    }
    let out = json!({
        "name": design.name,
        "bodies": design.bodies.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        "errors": design.errors.iter().map(|e| json!({"feature": e.feature, "message": e.message, "bodies": e.bodies})).collect::<Vec<_>>(),
        // Shown with the open: a body that differs from the design is never silent.
        "warnings": design.errors.iter().map(|e| e.warning()).collect::<Vec<_>>(),
        "model": if new_model { "new" } else { "kept" },
        "related": fit.related,
        "stale": {"mesh_bodies": fit.mesh_bodies, "sets": fit.sets, "seeds": fit.seeds},
    });
    // Another design file (or a new model) is another document; the same file opened again after
    // an edit is the same one.
    if new_model || s.sources.design.as_ref().is_none_or(|d| d.path() != source.path()) {
        s.mark_opened();
    }
    s.design = Some(Arc::new(design));
    s.sources.design = Some(source);
    Ok(out)
}

/// How the model's geometry references fit a design about to become the geometry.
struct Fit {
    /// Whether the design (or an imported surface) has a body the model refers to, or the model
    /// refers to none: `false` means the model is of another part.
    related: bool,
    /// Bodies the mesh is tagged with that neither the design nor a surface has.
    mesh_bodies: Vec<String>,
    /// Sets naming a body or a face that is not there.
    sets: Vec<String>,
    /// Edge seeds on an edge the design lacks.
    seeds: Vec<String>,
}

fn fit(s: &Session, design: &Design) -> Fit {
    let has = |body: &str| design.body(body).is_some() || s.surfaces.iter().any(|t| t.name == body);
    let resolves = |r: &GeometryRef| match (&r.entity, design.body(&r.body)) {
        (Entity::Face(face), Some(b)) => b.faces().iter().any(|f| &f.name == face),
        _ => has(&r.body),
    };
    let tagged: BTreeSet<&str> = s.model.classification().entities().iter().map(|r| r.body.as_str()).collect();
    let mut referenced = tagged.clone();
    referenced.extend(s.model.sets().values().flat_map(|set| set.geometry.iter().map(|r| r.body.as_str())));
    Fit {
        related: referenced.is_empty() || referenced.iter().any(|b| has(b)),
        mesh_bodies: tagged.into_iter().filter(|b| !has(b)).map(str::to_string).collect(),
        sets: s.model.sets().iter().filter(|(_, set)| !set.geometry.iter().all(resolves)).map(|(n, _)| n.clone()).collect(),
        seeds: s
            .model
            .seeds()
            .iter()
            .filter(|seed| super::seeds::seeded_edges(design, &seed.edge).map_or(true, |e| e.is_empty()))
            .map(|seed| seed.edge.to_string())
            .collect(),
    }
}

fn bodies(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(design) = &s.design else { return Ok(json!([])) };
    let mut out = Vec::with_capacity(design.bodies.len());
    for b in &design.bodies {
        let m = b.measure()?;
        let edges: Vec<Value> = b.edges()?.iter().map(|e| json!({"name": e.name, "length": e.length()})).collect();
        let vertices: Vec<Value> = b.vertices()?.iter().map(|v| json!({"name": v.name, "point": v.point})).collect();
        out.push(json!({
            "name": b.name,
            "volume": m.volume,
            "area": m.area,
            "centroid": m.centroid,
            "bbox": [m.bbox_min, m.bbox_max],
            "faces": b.faces().iter().map(|f| json!({"name": f.name, "exact": f.exact})).collect::<Vec<_>>(),
            "edges": edges,
            "vertices": vertices,
        }));
    }
    Ok(Value::Array(out))
}

fn features(s: &mut Session, p: &Value) -> Result<Value> {
    let body = match p.get("body") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| EngineError::param("body", ProblemKind::NotAName))?),
    };
    let design = s.design.as_deref().ok_or_else(|| EngineError::Params("open a design first".into()))?;
    let graph = design.features(body)?;
    Ok(json!({"bodies": graph}))
}

/// Most geometry items and nodes `geometry.centre` takes.
const MAX_CENTRE_ITEMS: usize = 10_000;

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(a: [f64; 3], k: f64) -> [f64; 3] {
    a.map(|v| v * k)
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The centre of the circle through three points, `None` when they lie on a line.
fn circumcentre(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<[f64; 3]> {
    let (u, v) = (sub3(b, a), sub3(c, a));
    let n = cross3(u, v);
    let nn = dot3(n, n);
    if !(nn > 1e-24 * dot3(u, u) * dot3(v, v)) {
        return None;
    }
    let k = add3(scale3(cross3(n, u), dot3(v, v)), scale3(cross3(v, n), dot3(u, u)));
    Some(add3(a, scale3(k, 0.5 / nn)))
}

/// The centre of an edge's polyline when it is an arc or circle (every point at one distance
/// from it, within a thousandth), else `None`.
fn circle_centre(points: &[[f64; 3]]) -> Option<[f64; 3]> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let closed = points.first().zip(points.last()).is_some_and(|(a, b)| dot3(sub3(*a, *b), sub3(*a, *b)) < 1e-18);
    let pick = |f: f64| points.get(((n - 1) as f64 * f).round() as usize).copied();
    let (a, b, c) = if closed { (pick(0.0)?, pick(1.0 / 3.0)?, pick(2.0 / 3.0)?) } else { (pick(0.0)?, pick(0.5)?, pick(1.0)?) };
    let centre = circumcentre(a, b, c)?;
    let r = dot3(sub3(a, centre), sub3(a, centre)).sqrt();
    let on = points.iter().all(|p| (dot3(sub3(*p, centre), sub3(*p, centre)).sqrt() - r).abs() <= 1e-3 * r);
    on.then_some(centre)
}

/// The length-weighted centre of a polyline and its length.
fn polyline_centroid(points: &[[f64; 3]]) -> ([f64; 3], f64) {
    let (mut sum, mut length) = ([0.0; 3], 0.0);
    for w in points.windows(2) {
        let l = dot3(sub3(w[1], w[0]), sub3(w[1], w[0])).sqrt();
        sum = add3(sum, scale3(add3(w[0], w[1]), 0.5 * l));
        length += l;
    }
    (if length > 0.0 { scale3(sum, 1.0 / length) } else { points.first().copied().unwrap_or_default() }, length)
}

fn centre(s: &mut Session, p: &Value) -> Result<Value> {
    let geometry: Vec<GeometryRef> = super::opt(p, "geometry")?.unwrap_or_default();
    let nodes: Vec<u32> = super::opt(p, "nodes")?.unwrap_or_default();
    if geometry.len() + nodes.len() > MAX_CENTRE_ITEMS {
        return Err(EngineError::Params(format!("geometry.centre takes at most {MAX_CENTRE_ITEMS} items")));
    }
    if geometry.is_empty() && nodes.is_empty() {
        return Err(EngineError::Params("give one of `geometry` or `nodes`: what to find the centre of".into()));
    }
    if !geometry.is_empty() && !nodes.is_empty() {
        return Err(EngineError::Params("give either `geometry` or `nodes`, not both".into()));
    }
    if !nodes.is_empty() {
        let mut sum = [0.0; 3];
        for n in &nodes {
            let node = s.model.node(*n).ok_or_else(|| EngineError::Params(format!("no node {n}")))?;
            sum = add3(sum, node.xyz);
        }
        return Ok(json!({"point": scale3(sum, 1.0 / nodes.len() as f64), "from": "nodes"}));
    }
    let design = s.design.as_deref().ok_or_else(|| EngineError::Params("open a design first".into()))?;
    let shape = |b: &str| design.body(b).ok_or_else(|| EngineError::Params(format!("the design has no body `{b}`")));
    if let [g] = geometry.as_slice()
        && let Entity::Vertex(v) = &g.entity
    {
        let vertex = shape(&g.body)?.vertices()?.into_iter().find(|x| &x.name == v);
        let vertex = vertex.ok_or_else(|| EngineError::Params(format!("body `{}` has no vertex `{v}`", g.body)))?;
        return Ok(json!({"point": vertex.point, "from": "vertex"}));
    }
    // Hole centres: every face a cylinder about one axis, or every edge an arc of one circle.
    let mut circles: Vec<[f64; 3]> = Vec::new();
    let mut axis: Option<([f64; 3], [f64; 3])> = None;
    let mut round = true;
    // Centroids, weighted by area (faces) or length (edges).
    let (mut faces_sum, mut faces_area, mut edges_sum, mut edges_length) = ([0.0; 3], 0.0, [0.0; 3], 0.0);
    for g in &geometry {
        let body = shape(&g.body)?;
        match &g.entity {
            Entity::Face(f) => {
                let i = body.face_index(f).ok_or_else(|| EngineError::Params(format!("body `{}` has no face `{f}`", g.body)))?;
                let c = body.face_centre(f)?;
                let mesh = body.tessellate((body.size()? * fieldcraft_geometry::EDGE_TOLERANCE).max(1e-4))?;
                let area: f64 = mesh
                    .triangles
                    .iter()
                    .zip(&mesh.tri_face)
                    .filter(|(_, t)| **t == i)
                    .filter_map(|(t, _)| {
                        let q = |k: usize| mesh.positions.get(t[k] as usize).copied();
                        let (a, b, c) = (q(0)?, q(1)?, q(2)?);
                        let n = cross3(sub3(b, a), sub3(c, a));
                        Some(0.5 * dot3(n, n).sqrt())
                    })
                    .sum();
                faces_sum = add3(faces_sum, scale3(c, area));
                faces_area += area;
                match body.face_surfaces()?.get(i as usize).copied().flatten() {
                    Some(fieldcraft_geometry::FaceSurface::Cylinder { origin, axis: d, .. }) => {
                        let len = dot3(d, d).sqrt();
                        let d = scale3(d, 1.0 / len.max(1e-300));
                        // The face centre's foot on the axis.
                        let foot = add3(origin, scale3(d, dot3(sub3(c, origin), d)));
                        let same = axis.is_none_or(|(o, a)| {
                            let off = sub3(foot, o);
                            let across = sub3(off, scale3(a, dot3(off, a)));
                            dot3(cross3(a, d), cross3(a, d)) < 1e-12 && dot3(across, across).sqrt() <= 1e-6 * body.size().unwrap_or(1.0).max(1e-9)
                        });
                        round &= same;
                        axis.get_or_insert((foot, d));
                        circles.push(foot);
                    }
                    _ => round = false,
                }
            }
            Entity::Edge(e) => {
                let edges = body.edges()?;
                let edge = edges.iter().find(|x| &x.name == e).ok_or_else(|| EngineError::Params(format!("body `{}` has no edge `{e}`", g.body)))?;
                let (c, l) = polyline_centroid(&edge.points);
                edges_sum = add3(edges_sum, scale3(c, l));
                edges_length += l;
                match circle_centre(&edge.points) {
                    Some(c) => circles.push(c),
                    None => round = false,
                }
            }
            Entity::Vertex(_) | Entity::Body => {
                return Err(EngineError::Params("geometry.centre takes one vertex, or faces and edges".into()));
            }
        }
    }
    // Arcs of one circle share a centre; faces of one axis were checked as they came.
    let spread = circles.iter().map(|c| circles.first().map_or(0.0, |f| dot3(sub3(*c, *f), sub3(*c, *f)).sqrt())).fold(0.0f64, f64::max);
    let size = geometry.first().and_then(|g| design.body(&g.body)).and_then(|b| b.size().ok()).unwrap_or(1.0).max(1e-9);
    let edges_only = geometry.iter().all(|g| matches!(g.entity, Entity::Edge(_)));
    if round && !circles.is_empty() && (!edges_only || spread <= 1e-6 * size) {
        let point = match axis {
            // Cylinder faces: the axis point at the mean of their centres' feet.
            Some((_, d)) => {
                let mean = scale3(circles.iter().fold([0.0; 3], |a, c| add3(a, *c)), 1.0 / circles.len() as f64);
                let o = circles.first().copied().unwrap_or(mean);
                add3(o, scale3(d, dot3(sub3(mean, o), d)))
            }
            None => circles.first().copied().unwrap_or_default(),
        };
        return Ok(json!({"point": point, "from": "circle"}));
    }
    let point = if faces_area > 0.0 {
        scale3(faces_sum, 1.0 / faces_area)
    } else if edges_length > 0.0 {
        scale3(edges_sum, 1.0 / edges_length)
    } else {
        return Err(EngineError::Params("what is picked has no area or length to find a centre of".into()));
    };
    if !point.iter().all(|v| v.is_finite()) {
        return Err(EngineError::Params("the centre is not a finite point".into()));
    }
    Ok(json!({"point": point, "from": "centroid"}))
}

fn body<'a>(s: &'a Session, p: &Value) -> Result<&'a Shape> {
    let name = p.get("body").and_then(Value::as_str).ok_or_else(|| EngineError::param("body", ProblemKind::NotAName))?;
    s.design.as_deref().and_then(|d| d.body(name)).ok_or_else(|| EngineError::Params(format!("no body `{name}` (open a design first)")))
}

/// The body named by `body`, tessellated within `tolerance`.
pub(crate) fn body_mesh<'a>(s: &'a Session, p: &Value) -> Result<(&'a Shape, SurfaceMesh)> {
    let shape = body(s, p)?;
    let tolerance = p.get("tolerance").and_then(Value::as_f64).ok_or_else(|| EngineError::param("tolerance", ProblemKind::NotANumber))?;
    Ok((shape, shape.tessellate(tolerance)?))
}

fn tessellate(s: &mut Session, p: &Value) -> Result<Value> {
    let (shape, mesh) = body_mesh(s, p)?;
    let mut per_face = vec![0usize; shape.faces().len()];
    for &f in &mesh.tri_face {
        if let Some(n) = per_face.get_mut(f as usize) {
            *n += 1;
        }
    }
    let faces: Vec<Value> = shape.faces().iter().zip(per_face).map(|(f, n)| json!({"name": f.name, "triangles": n})).collect();
    Ok(json!({"body": shape.name, "vertices": mesh.positions.len(), "triangles": mesh.triangles.len(), "faces": faces}))
}

fn import_mesh(s: &mut Session, p: &Value) -> Result<Value> {
    let path = super::file_param(s, p, "path", Access::Read)?;
    if s.sources.meshes.len() >= fieldcraft_io::MAX_MESH_SOURCES {
        return Err(EngineError::Params(format!("a model imports at most {} meshes", fieldcraft_io::MAX_MESH_SOURCES)));
    }
    let bytes = super::file::read_file(&path, tagged_mesh::MAX_FILE_BYTES as u64)?;
    let source = SourceFile::of(super::file::absolute(&path), &bytes)?;
    let surfaces = tagged_mesh::read_3mf(&bytes)?;
    let first = s.surfaces.len();
    let added: Vec<Value> = surfaces
        .iter()
        .enumerate()
        .map(|(k, t)| json!({"index": first + k, "name": t.name, "vertices": t.mesh.positions.len(), "triangles": t.mesh.triangles.len(), "faces": t.faces}))
        .collect();
    s.surfaces.extend(surfaces.into_iter().map(Arc::new));
    s.sources.meshes.push(source);
    Ok(json!({"path": path, "surfaces": added}))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    fn package(model: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("3D/3dmodel.model", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(model.as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    }

    /// The plate's hole (Ø10 at x = y = 15 through z 0..10): its faces give the axis point at
    /// mid-depth, a rim arc its circle's centre, a corner vertex its point, a flat face its
    /// centroid; datum points take the design's vertices by name.
    #[test]
    fn centres_of_holes_vertices_and_faces() {
        use fieldcraft_geometry::FaceSurface;
        use fieldcraft_geometry::sample::{Plate, plate_design};
        let design = Arc::new(open_design(&plate_design(&Plate::default()).unwrap()).unwrap());
        let mut s = Session { design: Some(design.clone()), ..Session::default() };
        let body = &design.bodies[0];
        let near = |v: &Value, p: [f64; 3]| {
            let q: [f64; 3] = serde_json::from_value(v["point"].clone()).unwrap();
            assert!((0..3).all(|k| (q[k] - p[k]).abs() < 1e-6), "{v} vs {p:?}");
        };
        let surfaces = body.face_surfaces().unwrap();
        let hole: Vec<Value> = body
            .faces()
            .iter()
            .zip(&surfaces)
            .filter(|(_, s)| matches!(s, Some(FaceSurface::Cylinder { .. })))
            .map(|(f, _)| json!({"body": body.name, "face": f.name}))
            .collect();
        assert_eq!(hole.len(), 2);
        let r = s.execute("geometry.centre", &json!({"geometry": hole})).unwrap();
        near(&r, [15.0, 15.0, 5.0]);
        assert_eq!(r["from"], "circle");
        let r = s.execute("geometry.centre", &json!({"geometry": [hole[0]]})).unwrap();
        near(&r, [15.0, 15.0, 5.0]);
        // A rim arc on top.
        let edges = body.edges().unwrap();
        let arc = edges.iter().find(|e| e.points.iter().all(|p| (p[2] - 10.0).abs() < 1e-9) && e.points.len() > 3).unwrap();
        let r = s.execute("geometry.centre", &json!({"geometry": [{"body": body.name, "edge": arc.name}]})).unwrap();
        near(&r, [15.0, 15.0, 10.0]);
        assert_eq!(r["from"], "circle");
        // The corner at the origin, and a datum point on it by name.
        let corner = body.vertices().unwrap().into_iter().find(|v| v.point == [0.0; 3]).unwrap();
        let r = s.execute("geometry.centre", &json!({"geometry": [{"body": body.name, "vertex": corner.name}]})).unwrap();
        near(&r, [0.0; 3]);
        s.execute("geometry.datum.point", &json!({"name": "P", "by": {"type": "vertex", "body": body.name, "vertex": corner.name}})).unwrap();
        assert_eq!(s.model.datum("P").and_then(|d| d.point()), Some([0.0; 3]));
        let listed = s.execute("geometry.bodies", &json!({})).unwrap();
        assert!(listed[0]["vertices"].as_array().unwrap().iter().any(|v| v["name"] == corner.name.as_str()), "{listed}");
        // The bottom face (z = 0): its centroid, off the hole.
        let bottom = body
            .faces()
            .iter()
            .zip(&surfaces)
            .find(|(_, s)| matches!(s, Some(FaceSurface::Plane { normal, point }) if normal[2].abs() > 0.99 && point[2].abs() < 1e-9))
            .unwrap()
            .0;
        let r = s.execute("geometry.centre", &json!({"geometry": [{"body": body.name, "face": bottom.name}]})).unwrap();
        assert_eq!(r["from"], "centroid");
        // Nodes, and what is refused.
        s.model.set_node(1, [1.0, 2.0, 3.0]).unwrap();
        s.model.set_node(2, [3.0, 2.0, 1.0]).unwrap();
        near(&s.execute("geometry.centre", &json!({"nodes": [1, 2]})).unwrap(), [2.0, 2.0, 2.0]);
        for bad in [
            json!({}),
            json!({"nodes": [99]}),
            json!({"geometry": [{"body": "nope"}]}),
            json!({"geometry": [{"body": body.name}]}),
            json!({"nodes": "x"}),
        ] {
            assert!(s.execute("geometry.centre", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn import_adds_tagged_surfaces_and_failures_change_nothing() {
        let dir = std::env::temp_dir().join(format!("fieldcraft-import-mesh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = format!(
            r#"<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:ft="{}"><resources>
<object id="1" name="Tet" ft:faces="2"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/></vertices>
<triangles><triangle v1="0" v2="2" v3="1" ft:face="0"/><triangle v1="0" v2="1" v3="3" ft:face="1"/><triangle v1="1" v2="2" v3="3" ft:face="1"/><triangle v1="0" v2="3" v3="2" ft:face="1"/></triangles></mesh></object>
</resources><build><item objectid="1"/></build></model>"#,
            tagged_mesh::FACE_TAGS_NS
        );
        let good = dir.join("tet.3mf");
        std::fs::write(&good, package(&model)).unwrap();
        let junk = dir.join("junk.3mf");
        std::fs::write(&junk, b"PK\x03\x04 not really").unwrap();

        let mut s = Session::default();
        let r = s.execute("geometry.import.mesh", &json!({"path": good.to_str().unwrap()})).unwrap();
        assert_eq!(r["surfaces"][0], json!({"index": 0, "name": "Tet", "vertices": 4, "triangles": 4, "faces": 2}));
        assert_eq!(s.surfaces[0].mesh.tri_face, vec![0, 1, 1, 1]);
        assert_eq!(s.journal.len(), 1);

        let hostile = [
            json!({"path": junk.to_str().unwrap()}),
            json!({"path": dir.join("missing.3mf").to_str().unwrap()}),
            json!({"path": dir.to_str().unwrap()}),
            json!({"path": "/dev/zero"}),
            json!({"path": ""}),
            json!({"path": "x".repeat(5000)}),
        ];
        for p in hostile {
            assert!(s.execute("geometry.import.mesh", &p).is_err(), "{p}");
        }
        assert_eq!(s.surfaces.len(), 1);
        assert_eq!(s.journal.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
