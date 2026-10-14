//! Unstructured all-hexahedral meshing (`mesh.hex.octree`) of the open design's bodies or of
//! imported surfaces.

use fieldcraft_mesh::octree::{OctreeHexMesh, OctreeOptions, octree_hex_mesh};
use fieldcraft_mesh::size::UniformSize;
use fieldcraft_mesh::surface::BoundarySurface;
use fieldcraft_model::{ElementKind, GeometryRef, MAX_ELEMENTS};
use serde_json::{Value, json};

use super::hex::{Input, inputs, number, tag_faces};
use super::mesh::optional_count;
use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const HEX_OCTREE: CommandSpec = CommandSpec {
    id: "mesh.hex.octree",
    label: "Octree Hex Mesh",
    params: r#"{"body": "Body1" (default: every body of the open design) or "surface": 0 (an imported surface), "size": 2.0 (element size in the interior, mm; default 1/25 of the bounding-box diagonal), "tolerance": 0.05 (chordal tolerance of the tessellation, mm; default 1e-3 of the diagonal), "max_depth": 3 (refinement levels below the size, each a third, 0..=5), "max_elements": 2000000 (1..=10000000; default the session limit), "attempts": 4 (builds, refining where one failed, 1..=8), "max_sweeps": 300 (optimiser sweeps, 0..=10000), and the boundary options of mesh.tet.generate ("weld", "weld_distance", "feature_angle")} — replaces the mesh with an unstructured all-hex8 mesh of any part: an adaptive 3-refinement grid with conforming transition templates, a core inside the part, two boundary layers that capture every B-rep face boundary and sharp edge, and untangling. Every element is proven valid; boundary element faces are tagged with their B-rep faces (named as in geometry.bodies; by index when a face has no name), so sets by geometry resolve. Reports per body the elements, quality, volume and grid statistics"#,
    form: Some(super::forms::HEX_OCTREE),
    kind: CommandKind::Model,
    requires: &[Requirement::Geometry],
    run: hex_octree,
};

/// Smallest element size, relative to the bounding-box diagonal (the grid's cell count grows
/// with the cube of its inverse).
const MIN_RELATIVE_SIZE: f64 = 1e-3;
/// Default element size, relative to the bounding-box diagonal.
const DEFAULT_RELATIVE_SIZE: f64 = 1.0 / 25.0;
/// Caps on the options.
const MAX_DEPTH: u64 = 5;
const MAX_ATTEMPTS: u64 = 8;
const MAX_SWEEPS: u64 = 10_000;
const MAX_HEX_ELEMENTS: u64 = 10_000_000;
/// The session default of `max_elements` ([`crate::Limits::max_hex_elements`]).
pub(crate) const DEFAULT_MAX_ELEMENTS: usize = 2_000_000;

fn hex_octree(s: &mut Session, p: &Value) -> Result<Value> {
    let size = number(p, "size")?;
    let defaults = OctreeOptions::default();
    let options = OctreeOptions {
        max_depth: u8::try_from(optional_count(p, "max_depth", u64::from(defaults.max_depth), 0..=MAX_DEPTH)?).unwrap_or(defaults.max_depth),
        max_elements: optional_count(p, "max_elements", s.limits.max_hex_elements as u64, 1..=MAX_HEX_ELEMENTS)?.min(MAX_ELEMENTS),
        attempts: optional_count(p, "attempts", defaults.attempts as u64, 1..=MAX_ATTEMPTS)?,
        max_sweeps: optional_count(p, "max_sweeps", defaults.max_sweeps as u64, 0..=MAX_SWEEPS)?,
        ..defaults
    };
    let bodies = inputs(s, p)?;
    let mut meshes: Vec<(Input, OctreeHexMesh)> = Vec::with_capacity(bodies.len());
    let mut total = 0usize;
    for b in bodies {
        if !(b.diagonal > 0.0 && b.diagonal.is_finite()) {
            return Err(EngineError::Params(format!("body `{}` has no extent", b.name)));
        }
        let h = size.unwrap_or(DEFAULT_RELATIVE_SIZE * b.diagonal);
        if h < MIN_RELATIVE_SIZE * b.diagonal {
            return Err(EngineError::Params(format!("`size` must be at least {} mm for body `{}`", MIN_RELATIVE_SIZE * b.diagonal, b.name)));
        }
        let surface_options = super::mesh::surface_options(p, &b.mesh)?;
        let surface = BoundarySurface::new(&b.mesh.positions, &b.mesh.triangles, &b.mesh.tri_face, &surface_options)
            .map_err(|e| EngineError::Params(format!("body `{}`: {e}", b.name)))?;
        let left = OctreeOptions { max_elements: options.max_elements.saturating_sub(total).max(1), ..options.clone() };
        let mesh = octree_hex_mesh(&surface, &UniformSize(h), &left).map_err(|e| EngineError::Params(format!("body `{}`: {e}", b.name)))?;
        total += mesh.hexes.len();
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
        for hx in &m.hexes {
            model.set_element(element_label, ElementKind::Hex8, hx.iter().map(|&i| first_node + i).collect())?;
            model.tag_element(element_label, body_tag)?;
            element_label = element_label.checked_add(1).ok_or_else(|| EngineError::Params("too many elements".into()))?;
        }
        tag_faces(&mut model, std::slice::from_ref(b), |_| 0, &m.boundary, first_element)?;
        let r = &m.report;
        report.push(json!({
            "body": b.name,
            "kind": "hex8",
            "elements": m.hexes.len(),
            "nodes": m.nodes.len(),
            "boundary_faces": m.boundary.len(),
            "quality": {"min_scaled_jacobian": r.min_scaled_jacobian, "mean_scaled_jacobian": r.mean_scaled_jacobian},
            "volume": r.volume,
            "surface_volume": r.surface_volume,
            "grid": {"cells": r.grid_cells, "core_elements": r.core_elements, "layer_elements": r.layer_elements, "frame": r.frame},
            "features": {"corners": r.corners, "curves": r.curves, "patches": r.patches},
            "attempts": r.attempts,
            "retries": r.retries,
            "sweeps": r.sweeps,
        }));
    }
    s.model = model;
    Ok(json!({"bodies": report, "nodes": node_label - 1, "elements": element_label - 1}))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fieldcraft_geometry::open_design;
    use fieldcraft_geometry::sample::{Plate, plate_design};
    use serde_json::json;

    use crate::Session;

    fn opened() -> Session {
        let plate = Plate::default();
        Session { design: Some(Arc::new(open_design(&plate_design(&plate).unwrap()).unwrap())), ..Session::default() }
    }

    #[test]
    fn plate_with_a_hole_meshes_all_hex_and_face_sets_resolve() {
        let mut s = opened();
        let r = s.execute("mesh.hex.octree", &json!({"body": "Body1", "size": 5.0, "max_depth": 2})).unwrap();
        let body = &r["bodies"][0];
        let n = body["elements"].as_u64().unwrap() as usize;
        assert!(n > 50, "{r}");
        assert!(body["quality"]["min_scaled_jacobian"].as_f64().unwrap() > 0.0, "{r}");
        assert_eq!(s.model.element_counts(), vec![(fieldcraft_model::ElementKind::Hex8, n)]);
        let (v, sv) = (body["volume"].as_f64().unwrap(), body["surface_volume"].as_f64().unwrap());
        assert!((v - sv).abs() < 0.02 * sv, "{r}");
        // The certified quality report agrees.
        let q = s.execute("mesh.quality", &json!({})).unwrap();
        assert_eq!(q["valid"].as_u64().unwrap() as usize, n);
        // Every boundary face is tagged: the skin of the body set; each named face resolves.
        s.execute("model.set.create", &json!({"name": "skin", "geometry": [{"body": "Body1"}]})).unwrap();
        let skin = s.execute("model.set.resolve", &json!({"name": "skin"})).unwrap();
        assert_eq!(skin["faces"]["count"], body["boundary_faces"], "{skin}");
        let bodies = s.execute("geometry.bodies", &json!({})).unwrap();
        for f in bodies[0]["faces"].as_array().unwrap() {
            let name = f["name"].as_str().unwrap();
            s.execute("model.set.create", &json!({"name": "f", "geometry": [{"body": "Body1", "face": name}]})).unwrap();
            let set = s.execute("model.set.resolve", &json!({"name": "f"})).unwrap();
            assert!(set["faces"]["count"].as_u64().unwrap() > 0, "face {name}: {set}");
            s.execute("model.set.delete", &json!({"name": "f"})).unwrap();
        }
    }

    #[test]
    fn hostile_params_are_errors_and_change_nothing() {
        let mut s = opened();
        let before = s.model.clone();
        for p in [
            json!({"body": "Body1", "size": 0}),
            json!({"body": "Body1", "size": -1}),
            json!({"body": "Body1", "size": 1e-300}),
            json!({"body": "Body1", "size": "big"}),
            json!({"body": "Body1", "tolerance": 1e-300}),
            json!({"body": "Body1", "tolerance": -1}),
            json!({"body": "Body1", "max_depth": 9}),
            json!({"body": "Body1", "max_depth": -1}),
            json!({"body": "Body1", "max_elements": 0}),
            json!({"body": "Body1", "max_elements": 1e300}),
            json!({"body": "Body1", "attempts": 0}),
            json!({"body": "Body1", "attempts": 100}),
            json!({"body": "Body1", "max_sweeps": "many"}),
            json!({"body": "Body1", "size": 5.0, "max_depth": 2, "max_elements": 10}),
            json!({"body": "Nope"}),
            json!({"body": 7}),
            json!({"surface": 0}),
            json!({"surface": -1}),
            json!({"body": "Body1", "surface": 0}),
        ] {
            let r = s.execute("mesh.hex.octree", &p);
            assert!(r.is_err(), "{p}: {r:?}");
            assert_eq!(s.model, before, "{p}");
        }
        assert!(Session::default().execute("mesh.hex.octree", &json!({})).is_err());
    }
}
