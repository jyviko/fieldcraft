use std::borrow::Cow;

use modelcraft_geometry::SurfaceMesh;
use modelcraft_mesh::delaunay::Triangulation;
use modelcraft_mesh::quality::{self, QualityReport};
use modelcraft_mesh::surface::{BoundarySurface, Defect, FeatureKind, SurfaceOptions};
use modelcraft_model::{ElementKind, MAX_ELEMENTS};
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

/// The surface named by `body` (tessellated at `tolerance`) or `surface` (an imported index).
fn surface_input<'a>(s: &'a Session, p: &Value) -> Result<Cow<'a, SurfaceMesh>> {
    match (p.get("body"), p.get("surface")) {
        (Some(_), None) => Ok(Cow::Owned(super::geometry::body_mesh(s, p)?.1)),
        (None, Some(i)) => {
            let n = s.surfaces.len();
            i.as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| s.surfaces.get(i))
                .map(|t| Cow::Borrowed(&t.mesh))
                .ok_or_else(|| EngineError::Params(format!("`surface` must be the index of an imported surface (there are {n})")))
        }
        _ => Err(EngineError::Params("give either `body` (with `tolerance`) or `surface`".into())),
    }
}

fn defect_json(d: &Defect) -> Value {
    json!({"kind": d.kind(), "message": d.to_string(), "at": d.at()})
}

fn surface_check(s: &mut Session, p: &Value) -> Result<Value> {
    let mesh = surface_input(s, p)?;
    let defaults = SurfaceOptions::default();
    let options = SurfaceOptions {
        weld_tolerance: optional_number(p, "weld", defaults.weld_tolerance)?,
        weld_distance: optional_number(p, "weld_distance", mesh.weld_distance)?,
        feature_angle: optional_number(p, "feature_angle", defaults.feature_angle)?,
    };
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
}
