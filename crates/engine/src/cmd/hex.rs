//! Structured hexahedral meshing of the open design's bodies or of imported surfaces.

use modelcraft_geometry::{FaceSurface, Shape, SurfaceMesh};
use modelcraft_mesh::hex::{HexMesh, HexMethod, HexOptions, Surface, hex_mesh};
use modelcraft_mesh::size::UniformSize;
use modelcraft_mesh::surface::{BoundarySurface, SurfaceOptions};
use modelcraft_model::{GeometryRef, Model};
use serde_json::{Value, json};

use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const HEX_GENERATE: CommandSpec = CommandSpec {
    id: "mesh.hex.generate",
    label: "Generate Hex Mesh",
    params: r#"{"body": "Body1" (default: every body of the open design) or "surface": 0 (an imported surface), "size": 2.0 (element size, mm; default 1/25 of the bounding-box diagonal), "order": 1 (hex8) or 2 (hex20, mid-edge nodes on the geometry), "method": "auto" (default) | "swept" (any sweep) | "extrude" | "sweep" | "revolve" | "mapped", "tolerance": 0.05 (chordal tolerance of the tessellation, mm; default 1e-3 of the diagonal)} — replaces the mesh with structured hexahedra: multi-level sweeps along a direction or around an axis, one-to-one sweeps through linking faces, or mapped blocks. Boundary element faces are tagged with their B-rep faces (named as in geometry.bodies; `face<index>` when a face has no name), so sets by geometry resolve. Reports per body the method, counts and quality; a body no structured method fits is an error that says why"#,
    kind: CommandKind::Model,
    run: hex_generate,
};

/// Bodies at most a command meshes.
const MAX_BODIES: usize = 1000;
/// Smallest element size and chordal tolerance, relative to the bounding-box diagonal.
const MIN_RELATIVE_SIZE: f64 = 1e-4;
const MIN_RELATIVE_TOLERANCE: f64 = 1e-5;
/// Default element size and chordal tolerance, relative to the bounding-box diagonal.
const DEFAULT_RELATIVE_SIZE: f64 = 1.0 / 25.0;
const DEFAULT_RELATIVE_TOLERANCE: f64 = 1e-3;

/// One body to mesh.
struct Input {
    name: String,
    mesh: SurfaceMesh,
    surfaces: Vec<Option<Surface>>,
    face_names: Vec<String>,
    diagonal: f64,
}

fn analytic(s: FaceSurface) -> Surface {
    match s {
        FaceSurface::Plane { normal, point } => Surface::Plane { point, normal },
        FaceSurface::Cylinder { origin, axis, radius } => Surface::Cylinder { origin, axis, radius },
        FaceSurface::Cone { apex, axis, half_angle } => Surface::Cone { apex, axis, half_angle },
        FaceSurface::Sphere { center, radius } => Surface::Sphere { center, radius },
        FaceSurface::Torus { center, axis, major, minor } => Surface::Torus { center, axis, major, minor },
    }
}

fn number(p: &Value, key: &str) -> Result<Option<f64>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite() && *x > 0.0)
            .map(Some)
            .ok_or_else(|| EngineError::Params(format!("`{key}` must be a positive number (mm)"))),
    }
}

fn diagonal(lo: [f64; 3], hi: [f64; 3]) -> f64 {
    (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f64>().sqrt()
}

fn shape_input(shape: &Shape, tolerance: Option<f64>) -> Result<Input> {
    let diag = shape.size()?;
    let tol = tolerance.unwrap_or(DEFAULT_RELATIVE_TOLERANCE * diag);
    if tol < MIN_RELATIVE_TOLERANCE * diag {
        return Err(EngineError::Params(format!("`tolerance` must be at least {} mm for body `{}`", MIN_RELATIVE_TOLERANCE * diag, shape.name)));
    }
    let surfaces = shape.face_surfaces()?.into_iter().map(|f| f.map(analytic)).collect();
    let face_names = shape.faces().iter().map(|f| f.name.clone()).collect();
    Ok(Input { name: shape.name.clone(), mesh: shape.tessellate(tol)?, surfaces, face_names, diagonal: diag })
}

fn inputs(s: &Session, p: &Value) -> Result<Vec<Input>> {
    let tolerance = number(p, "tolerance")?;
    match (p.get("body"), p.get("surface")) {
        (Some(b), None) => {
            let name = b.as_str().ok_or_else(|| EngineError::Params("`body` must be a body name".into()))?;
            let shape = s
                .design
                .as_deref()
                .and_then(|d| d.body(name))
                .ok_or_else(|| EngineError::Params(format!("no body `{name}` (open a design first)")))?;
            Ok(vec![shape_input(shape, tolerance)?])
        }
        (None, Some(i)) => {
            let n = s.surfaces.len();
            let index = i.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < n);
            let index = index.ok_or_else(|| EngineError::Params(format!("`surface` must be the index of an imported surface (there are {n})")))?;
            let t = s.surfaces.get(index).ok_or_else(|| EngineError::Params("no such surface".into()))?;
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for q in &t.mesh.positions {
                for k in 0..3 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
            let name = if t.name.trim().is_empty() { format!("Surface{index}") } else { t.name.clone() };
            Ok(vec![Input { name, mesh: t.mesh.clone(), surfaces: Vec::new(), face_names: Vec::new(), diagonal: diagonal(lo, hi) }])
        }
        (None, None) => {
            let design = s.design.as_deref().ok_or_else(|| EngineError::Params("open a design first, or give `surface`".into()))?;
            if design.bodies.is_empty() || design.bodies.len() > MAX_BODIES {
                return Err(EngineError::Params(format!("the design has {} bodies; meshing takes 1 to {MAX_BODIES}", design.bodies.len())));
            }
            design.bodies.iter().map(|b| shape_input(b, tolerance)).collect()
        }
        _ => Err(EngineError::Params("give `body` or `surface`, not both".into())),
    }
}

pub const HEX_SWEEP: CommandSpec = CommandSpec {
    id: "mesh.hex.sweep",
    label: "Sweep Hex Mesh",
    params: r#"the parameters of mesh.hex.generate, with "method": "swept" (default: extrude, sweep or revolve, whichever fits first) | "extrude" | "sweep" | "revolve" — mesh.hex.generate limited to the sweeping methods"#,
    kind: CommandKind::Model,
    run: hex_sweep,
};

pub const HEX_MAPPED: CommandSpec = CommandSpec {
    id: "mesh.hex.mapped",
    label: "Mapped Hex Mesh",
    params: r#"the parameters of mesh.hex.generate without "method" — mesh.hex.generate with the mapped method: a six-sided block (six faces of four edges, three edges at each of eight corners)"#,
    kind: CommandKind::Model,
    run: hex_mapped,
};

fn hex_generate(s: &mut Session, p: &Value) -> Result<Value> {
    let all = [HexMethod::Auto, HexMethod::Swept, HexMethod::Extrude, HexMethod::Sweep, HexMethod::Revolve, HexMethod::Mapped];
    generate(s, p, HexMethod::Auto, &all)
}

fn hex_sweep(s: &mut Session, p: &Value) -> Result<Value> {
    generate(s, p, HexMethod::Swept, &[HexMethod::Swept, HexMethod::Extrude, HexMethod::Sweep, HexMethod::Revolve])
}

fn hex_mapped(s: &mut Session, p: &Value) -> Result<Value> {
    generate(s, p, HexMethod::Mapped, &[HexMethod::Mapped])
}

/// Mesh with `default` or the `method` parameter, which must be one of `allowed`.
fn generate(s: &mut Session, p: &Value, default: HexMethod, allowed: &[HexMethod]) -> Result<Value> {
    let size = number(p, "size")?;
    let quadratic = match p.get("order") {
        None => false,
        Some(v) => match v.as_u64() {
            Some(1) => false,
            Some(2) => true,
            _ => return Err(EngineError::Params("`order` must be 1 (hex8) or 2 (hex20)".into())),
        },
    };
    let method = match p.get("method") {
        None => default,
        Some(v) => v.as_str().and_then(HexMethod::from_name).filter(|m| allowed.contains(m)).ok_or_else(|| {
            let names: Vec<&str> = allowed.iter().map(|m| m.name()).collect();
            EngineError::Params(format!("`method` must be one of {}", names.join(", ")))
        })?,
    };
    let bodies = inputs(s, p)?;
    let mut meshes: Vec<(Input, HexMesh)> = Vec::with_capacity(bodies.len());
    for b in bodies {
        if !(b.diagonal > 0.0 && b.diagonal.is_finite()) {
            return Err(EngineError::Params(format!("body `{}` has no extent", b.name)));
        }
        let h = size.unwrap_or(DEFAULT_RELATIVE_SIZE * b.diagonal);
        if h < MIN_RELATIVE_SIZE * b.diagonal {
            return Err(EngineError::Params(format!("`size` must be at least {} mm for body `{}`", MIN_RELATIVE_SIZE * b.diagonal, b.name)));
        }
        let surface_options = SurfaceOptions { weld_distance: b.mesh.weld_distance, ..SurfaceOptions::default() };
        let surface = BoundarySurface::new(&b.mesh.positions, &b.mesh.triangles, &b.mesh.tri_face, &surface_options)
            .map_err(|e| EngineError::Params(format!("body `{}`: {e}", b.name)))?;
        let chord = if b.surfaces.is_empty() { 0.0 } else { number(p, "tolerance")?.unwrap_or(DEFAULT_RELATIVE_TOLERANCE * b.diagonal) };
        let options =
            HexOptions { method, quadratic, chordal_tolerance: chord, max_elements: modelcraft_model::MAX_ELEMENTS / 8, ..HexOptions::default() };
        let mesh = hex_mesh(&surface, &b.surfaces, &UniformSize(h), &options).map_err(|e| EngineError::Params(format!("body `{}`: {e}", b.name)))?;
        meshes.push((b, mesh));
    }
    // Replace the mesh: definitions stay, nodes, elements and geometry tags are new.
    let mut model = s.model.without_mesh();
    let mut report = Vec::with_capacity(meshes.len());
    let (mut node_label, mut element_label) = (1u32, 1u32);
    for (b, m) in &meshes {
        let first_node = node_label;
        for xyz in &m.nodes {
            model.set_node(node_label, *xyz)?;
            node_label = node_label.checked_add(1).ok_or_else(|| EngineError::Params("too many nodes".into()))?;
        }
        let body_tag = model.geometry_tag(&GeometryRef::body(b.name.as_str())?)?;
        let first_element = element_label;
        for e in 0..m.element_count() {
            let nodes: Vec<u32> = m.element(e).unwrap_or(&[]).iter().map(|&i| first_node + i).collect();
            model.set_element(element_label, m.kind, nodes)?;
            model.tag_element(element_label, body_tag)?;
            element_label = element_label.checked_add(1).ok_or_else(|| EngineError::Params("too many elements".into()))?;
        }
        tag_faces(&mut model, b, m, first_element)?;
        report.push(json!({
            "body": b.name,
            "method": m.report.method.name(),
            "description": m.report.description,
            "kind": m.kind.name(),
            "elements": m.element_count(),
            "nodes": m.nodes.len(),
            "boundary_faces": m.boundary.len(),
            "quality": {"min_scaled_jacobian": m.report.min_scaled_jacobian, "mean_scaled_jacobian": m.report.mean_scaled_jacobian},
            "volume": m.report.volume,
            "surface_volume": m.report.surface_volume,
        }));
    }
    s.model = model;
    Ok(json!({"bodies": report, "nodes": node_label - 1, "elements": element_label - 1}))
}

/// Tag every boundary element face with its B-rep face.
fn tag_faces(model: &mut Model, b: &Input, m: &HexMesh, first_element: u32) -> Result<()> {
    let mut tags: Vec<Option<u32>> = Vec::new();
    for f in &m.boundary {
        let i = f.brep_face as usize;
        if tags.len() <= i {
            tags.resize(i + 1, None);
        }
        let tag = match tags.get(i).copied().flatten() {
            Some(t) => t,
            None => {
                let name = b.face_names.get(i).cloned().unwrap_or_else(|| format!("face{i}"));
                let t = model.geometry_tag(&GeometryRef::face(b.name.as_str(), name)?)?;
                if let Some(slot) = tags.get_mut(i) {
                    *slot = Some(t);
                }
                t
            }
        };
        model.tag_element_face(first_element + f.element, f.face, tag)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelcraft_geometry::sample::Plate;

    fn opened() -> (crate::tests::TempDir, Session) {
        let (dir, path) = crate::tests::sample_design_file(&Plate::default());
        let mut s = Session::default();
        s.execute("geometry.open", &json!({"path": path})).unwrap();
        (dir, s)
    }

    #[test]
    fn plate_with_a_hole_meshes_with_hexahedra_and_face_sets_resolve() {
        let (_dir, mut s) = opened();
        let r = s.execute("mesh.hex.generate", &json!({"body": "Body1", "size": 2.5})).unwrap();
        let body = &r["bodies"][0];
        assert_eq!(body["method"], "extrude", "{r}");
        assert_eq!(body["kind"], "hex8");
        let n = body["elements"].as_u64().unwrap() as usize;
        assert!(n > 100, "{r}");
        assert!(body["quality"]["min_scaled_jacobian"].as_f64().unwrap() > 0.3, "{r}");
        assert_eq!(s.model.element_counts(), vec![(modelcraft_model::ElementKind::Hex8, n)]);
        // Volume: the plate minus the hole, to the tessellation's accuracy.
        let v = 40.0 * 30.0 * 10.0 - std::f64::consts::PI * 25.0 * 10.0;
        assert!((body["volume"].as_f64().unwrap() - v).abs() < 0.01 * v, "{r}");
        // A set on the top face resolves to element faces.
        let bodies = s.execute("geometry.bodies", &json!({})).unwrap();
        let faces: Vec<String> = bodies[0]["faces"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect();
        let top = faces.iter().find(|f| f.ends_with("top") || f.contains("end")).unwrap_or(&faces[0]).clone();
        s.execute("model.set.create", &json!({"name": "top", "geometry": [{"body": "Body1", "face": top}]})).unwrap();
        let set = s.execute("model.set.resolve", &json!({"name": "top"})).unwrap();
        assert!(set["faces"]["count"].as_u64().unwrap() > 0, "{set}");
        // Every boundary face is tagged: the skin of the body set.
        s.execute("model.set.create", &json!({"name": "skin", "geometry": [{"body": "Body1"}]})).unwrap();
        let skin = s.execute("model.set.resolve", &json!({"name": "skin"})).unwrap();
        assert_eq!(skin["faces"]["count"], body["boundary_faces"], "{skin}");
        assert_eq!(skin["elements"]["count"].as_u64().unwrap() as usize, n);
        // Hex20 with mid-edge nodes on the hole.
        let r2 = s.execute("mesh.hex.generate", &json!({"order": 2, "size": 2.5})).unwrap();
        assert_eq!(r2["bodies"][0]["kind"], "hex20");
        assert_eq!(s.model.element_counts(), vec![(modelcraft_model::ElementKind::Hex20, n)]);
        let v2 = r2["bodies"][0]["volume"].as_f64().unwrap();
        assert!((v2 - v).abs() < (body["volume"].as_f64().unwrap() - v).abs() + 1e-6 * v, "hex20 {v2} vs hex8 {} vs {v}", body["volume"]);
        // The set still resolves after the remesh.
        let set = s.execute("model.set.resolve", &json!({"name": "top"})).unwrap();
        assert!(set["faces"]["count"].as_u64().unwrap() > 0);
        // Undo returns the hex8 mesh.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.model.element_counts(), vec![(modelcraft_model::ElementKind::Hex8, n)]);
    }

    #[test]
    fn hostile_params_are_errors_and_change_nothing() {
        let (_dir, mut s) = opened();
        let before = s.model.clone();
        for p in [
            json!({"body": "Body1", "size": 0}),
            json!({"body": "Body1", "size": -1}),
            json!({"body": "Body1", "size": 1e-300}),
            json!({"body": "Body1", "size": "big"}),
            json!({"body": "Body1", "size": 1e300}),
            json!({"body": "Body1", "order": 3}),
            json!({"body": "Body1", "order": "2"}),
            json!({"body": "Body1", "method": "octree"}),
            json!({"body": "Body1", "method": null}),
            json!({"body": "Body1", "tolerance": 1e-300}),
            json!({"body": "Body1", "tolerance": -1}),
            json!({"body": "Nope"}),
            json!({"body": 7}),
            json!({"surface": 0}),
            json!({"surface": -1}),
            json!({"body": "Body1", "surface": 0}),
            json!({"body": "Body1", "method": "revolve"}),
            json!({"body": "Body1", "method": "mapped"}),
        ] {
            let r = s.execute("mesh.hex.generate", &p);
            if p.get("size") == Some(&json!(1e300)) {
                // One element through the whole body is still a valid request.
                if r.is_ok() {
                    s.model = before.clone();
                }
                continue;
            }
            assert!(r.is_err(), "{p}: {r:?}");
            assert_eq!(s.model, before, "{p}");
        }
        assert!(Session::default().execute("mesh.hex.generate", &json!({})).is_err());
        assert!(s.execute("mesh.hex.sweep", &json!({"body": "Body1", "method": "mapped"})).is_err());
        assert!(s.execute("mesh.hex.mapped", &json!({"body": "Body1", "method": "sweep"})).is_err());
        // The plate is not a six-sided block; it is a sweep.
        assert!(s.execute("mesh.hex.mapped", &json!({"body": "Body1"})).unwrap_err().to_string().contains("logical cube"));
        assert_eq!(s.model, before);
        let r = s.execute("mesh.hex.sweep", &json!({"body": "Body1", "size": 4})).unwrap();
        assert_eq!(r["bodies"][0]["method"], "extrude");
    }
}
