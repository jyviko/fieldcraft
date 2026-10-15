//! `mesh.hex.project`: projection blocking — a block topology (a carved grid, a butterfly or
//! seven-block sphere template, O-grids, extruded faces) projected onto a body's B-rep and
//! smoothed ([`fieldcraft_mesh::project`]).

use std::collections::{BTreeMap, BTreeSet};

use fieldcraft_geometry::Shape;
use fieldcraft_mesh::hex::BoundaryFace;
use fieldcraft_mesh::join::{check_conforming, weld};
use fieldcraft_mesh::project::{ProjectOptions, Smoothing, Topology, project_mesh};
use fieldcraft_mesh::surface::BoundarySurface;
use fieldcraft_mesh::tet::geometry::Tessellation;
use fieldcraft_model::{ElementKind, GeometryRef};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const HEX_PROJECT: CommandSpec = CommandSpec {
    id: "mesh.hex.project",
    label: "Projection Blocking",
    params: r#"{"body": "Body1", "size": 2.0 (element size, mm; default 1/25 of the body's diagonal), "template": "grid" (default) | "sphere" | "butterfly" | "blocks", "center": [x, y, z], "radius": r, "axis": [0, 0, 1], "levels": [z0, z1, …] (sphere and butterfly: default from the body's bounding box; butterfly levels are distances along the axis from the centre), "inset": 0.55 (inner square or cube corner at this fraction of the radius), "grid": {"planes": {"x": [...], "y": [...], "z": [...]} or "cells": [ni, nj, nk] (even planes over the bounding box, default [1, 1, 1]), "carve": [{"i": [a, b], "j": [c, d], "k": [e, f]}] (block index ranges, inclusive, removed)}, "blocks": {"vertices": [[x, y, z], ...], "blocks": [[v1..v8], ...] (0-based vertices, hex8 order)}, "operations": [{"op": "o_grid", "block": b, "open": [faces], "inset": 0.5 or [i1, i2, i3], "extrude": {"offset": [dx, dy, dz], "layers": 1, "faces": "all" (the open face's butterfly: a round branch) | "inner" (its inner face: a square stub)}} | {"op": "extrude", "faces": [[block, face], ...], "offset": [dx, dy, dz], "layers": 1} | {"op": "connect", "from": [block, face], "to": [block, face], "layers": 1} (blocks bridging two faces: closes a handle) | {"op": "remove", "blocks": [...]} (carve; later blocks renumber) | {"op": "merge", "distance": d}] (applied in order; blocks are 0-based, new blocks appended, the O-grid's inner block keeping the old index; a face is its hex8 number 1-6 or the direction it faces, "+x" … "-z"), "preview": true (return the topology without meshing), "faces": [{"block": b, "face": 1-6, "to": ["face names"]}] (targets set by hand; other outer block faces go to the smooth group of B-rep faces nearest them), "divisions": [{"block": b, "direction": 1|2|3, "count": n}], "smoothing": "elliptic" (default) | "elliptic-controlled" (Thomas–Middlecoff) | "laplace" | "none", "iterations": 100, "min_quality": 0.2 (smallest scaled Jacobian accepted), "merge": distance (join nodes of other bodies' meshes within it; default 1e-6 of the diagonal, 0 never), "tolerance": chordal tolerance of the tessellation} — replaces the body's mesh with an all-hex mesh of the block topology: outer block faces projected onto their B-rep faces, edges between faces with different targets onto the curve where those meet (tangent-plane iteration), vertices on three onto their point, nodes placed by transfinite interpolation and smoothed with the boundary held on the geometry. Elements are tagged with the body and boundary faces with their B-rep faces. Reports blocks, divisions, targets and quality before and after smoothing; a corner that flattens on a curved face is an error that says to use an O-grid or template there"#,
    form: Some(super::forms::HEX_PROJECT),
    kind: CommandKind::Model,
    requires: &[Requirement::Geometry],
    run: hex_project,
};

/// Most operations, face targets or division settings one command takes.
const MAX_LIST: usize = 10_000;
/// Default element size relative to the body's diagonal.
const DEFAULT_RELATIVE_SIZE: f64 = 1.0 / 25.0;
const MIN_RELATIVE_SIZE: f64 = 1e-4;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GridParams {
    planes: Option<BTreeMap<String, Vec<f64>>>,
    cells: Option<[usize; 3]>,
    #[serde(default)]
    carve: Vec<BTreeMap<String, [usize; 2]>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlocksParams {
    vertices: Vec<[f64; 3]>,
    blocks: Vec<[u32; 8]>,
}

/// A block face: its hex8 number, or the direction it faces ("+x", "-z", …).
#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum FaceSel {
    Number(u8),
    Towards(String),
}

/// An inset: one fraction, or one per block direction.
#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
enum Inset {
    All(f64),
    Each([f64; 3]),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OGridExtrude {
    offset: [f64; 3],
    layers: Option<usize>,
    /// "all" (default: the whole open face, a butterfly section) or "inner" (the inner block's
    /// face alone, a four-sided section inside the face).
    faces: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, tag = "op", rename_all = "snake_case")]
enum Operation {
    OGrid {
        block: usize,
        #[serde(default)]
        open: Vec<FaceSel>,
        inset: Option<Inset>,
        extrude: Option<OGridExtrude>,
    },
    Extrude {
        faces: Vec<(usize, FaceSel)>,
        offset: [f64; 3],
        layers: Option<usize>,
    },
    Connect {
        from: (usize, FaceSel),
        to: (usize, FaceSel),
        layers: Option<usize>,
    },
    Remove {
        blocks: Vec<usize>,
    },
    Merge {
        distance: f64,
    },
}

fn face_of(t: &Topology, block: usize, f: &FaceSel) -> Result<u8> {
    match f {
        FaceSel::Number(n) if (1..=6).contains(n) => Ok(*n),
        FaceSel::Number(n) => Err(EngineError::Params(format!("face {n} is not a hex8 face (1 to 6)"))),
        FaceSel::Towards(d) => {
            let dir = match d.as_str() {
                "+x" => [1.0, 0.0, 0.0],
                "-x" => [-1.0, 0.0, 0.0],
                "+y" => [0.0, 1.0, 0.0],
                "-y" => [0.0, -1.0, 0.0],
                "+z" => [0.0, 0.0, 1.0],
                "-z" => [0.0, 0.0, -1.0],
                other => return Err(EngineError::Params(format!("face `{other}`: give a hex8 face number or +x, -x, +y, -y, +z, -z"))),
            };
            Ok(t.face_towards(block, dir)?)
        }
    }
}

/// Apply the topology operations in order.
fn operate(t: &mut Topology, ops: &[Operation]) -> Result<()> {
    for op in ops {
        match op {
            Operation::OGrid { block, open, inset, extrude } => {
                bounded(open, "operations.open")?;
                let open: Vec<u8> = open.iter().map(|f| face_of(t, *block, f)).collect::<Result<_>>()?;
                let inset = match inset.unwrap_or(Inset::All(0.5)) {
                    Inset::All(x) => [x; 3],
                    Inset::Each(v) => v,
                };
                let on = t.o_grid(*block, &open, inset)?;
                if let Some(e) = extrude {
                    let first = on.first().ok_or_else(|| EngineError::Params("an O-grid extrusion needs an open face".into()))?;
                    let faces: Vec<(usize, u8)> = match e.faces.as_deref().unwrap_or("all") {
                        "all" => first.clone(),
                        "inner" => first.iter().take(1).copied().collect(),
                        other => return Err(EngineError::Params(format!("O-grid extrusion faces `{other}`: all or inner"))),
                    };
                    t.extrude(&faces, e.offset, e.layers.unwrap_or(1))?;
                }
            }
            Operation::Extrude { faces, offset, layers } => {
                bounded(faces, "operations.faces")?;
                let faces: Vec<(usize, u8)> = faces.iter().map(|(b, f)| Ok((*b, face_of(t, *b, f)?))).collect::<Result<_>>()?;
                t.extrude(&faces, *offset, layers.unwrap_or(1))?;
            }
            Operation::Connect { from, to, layers } => {
                let a = (from.0, face_of(t, from.0, &from.1)?);
                let b = (to.0, face_of(t, to.0, &to.1)?);
                t.connect(a, b, layers.unwrap_or(1))?;
            }
            Operation::Remove { blocks } => {
                bounded(blocks, "operations.blocks")?;
                t.remove(blocks)?;
            }
            Operation::Merge { distance } => {
                if !(distance.is_finite() && *distance >= 0.0) {
                    return Err(EngineError::invalid("operations.distance", "a distance of 0 or more"));
                }
                t.merge(*distance);
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FaceTarget {
    block: usize,
    face: FaceSel,
    to: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Division {
    block: usize,
    direction: usize,
    count: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectParams {
    body: String,
    size: Option<f64>,
    tolerance: Option<f64>,
    template: Option<String>,
    center: Option<[f64; 3]>,
    radius: Option<f64>,
    axis: Option<[f64; 3]>,
    levels: Option<Vec<f64>>,
    inset: Option<f64>,
    grid: Option<GridParams>,
    blocks: Option<BlocksParams>,
    #[serde(default)]
    operations: Vec<Operation>,
    #[serde(default)]
    faces: Vec<FaceTarget>,
    #[serde(default)]
    divisions: Vec<Division>,
    smoothing: Option<String>,
    iterations: Option<usize>,
    min_quality: Option<f64>,
    merge: Option<f64>,
    // Read from the raw parameters by the surface preparation (mesh.tet.generate's options).
    #[allow(dead_code)]
    weld: Option<f64>,
    #[allow(dead_code)]
    weld_distance: Option<f64>,
    #[allow(dead_code)]
    feature_angle: Option<f64>,
    #[serde(default)]
    preview: bool,
}

/// The body, its tessellation as a boundary surface and projection target, and its size.
struct Body<'a> {
    shape: &'a Shape,
    input: super::hex::Input<'static>,
    surface: BoundarySurface,
    target: Tessellation,
    lo: [f64; 3],
    hi: [f64; 3],
}

fn open_body<'a>(s: &'a Session, name: &str, tolerance: Option<f64>, raw: &Value) -> Result<Body<'a>> {
    let shape =
        s.design.as_deref().and_then(|d| d.body(name)).ok_or_else(|| EngineError::Params(format!("no body `{name}` (open a design first)")))?;
    if tolerance.is_some_and(|t| !(t > 0.0 && t.is_finite())) {
        return Err(EngineError::Params("`tolerance` must be a positive number".into()));
    }
    let input = super::hex::shape_input(shape, tolerance)?;
    let surface =
        BoundarySurface::new(&input.mesh.positions, &input.mesh.triangles, &input.mesh.tri_face, &super::mesh::surface_options(raw, &input.mesh)?)
            .map_err(|e| EngineError::Params(format!("body `{name}`: {e}")))?;
    let target = Tessellation::new(&surface, super::mesh::analytic_surfaces(shape));
    let m = shape.measure()?;
    Ok(Body { shape, input, surface, target, lo: m.bbox_min, hi: m.bbox_max })
}

fn face_index(shape: &Shape, name: &str) -> Result<u32> {
    shape.face_index(name).ok_or_else(|| EngineError::Params(format!("body `{}` has no face `{name}`", shape.name)))
}

fn element_size(size: Option<f64>, diag: f64) -> Result<f64> {
    let h = size.unwrap_or(DEFAULT_RELATIVE_SIZE * diag);
    if !(h.is_finite() && h >= MIN_RELATIVE_SIZE * diag) {
        return Err(EngineError::Params(format!("`size` must be at least {} mm", MIN_RELATIVE_SIZE * diag)));
    }
    Ok(h)
}

fn merge_distance(merge: Option<f64>, diag: f64) -> Result<f64> {
    let m = merge.unwrap_or(1e-6 * diag);
    if !(m.is_finite() && m >= 0.0 && m <= 1e-2 * diag) {
        return Err(EngineError::Params(format!("`merge` must be from 0 to 1/100 of the body's diagonal ({diag})")));
    }
    Ok(m)
}

/// Replace `body`'s mesh with `nodes` and `hexes`, sharing nodes of other bodies within
/// `merge`; tag the elements with the body and `boundary` with its B-rep faces.
fn add_mesh(
    s: &mut Session,
    body: &super::hex::Input,
    nodes: &[[f64; 3]],
    hexes: &[[u32; 8]],
    boundary: &[BoundaryFace],
    merge: f64,
) -> Result<(Value, u32, usize)> {
    let mut model = s.model.clone();
    let cleared = super::replace::clear(&mut model, [body.name.as_str()]);
    let (first_node, first_element) = super::fresh_labels(&model, nodes.len(), hexes.len())?;
    let welded = if merge > 0.0 { weld(&model, nodes, merge)? } else { vec![None; nodes.len()] };
    let mut labels = Vec::with_capacity(nodes.len());
    let mut added = 0u32;
    for (xyz, existing) in nodes.iter().zip(&welded) {
        let label = match existing {
            Some(l) => *l,
            None => {
                let l = first_node + added;
                model.set_node(l, *xyz)?;
                added += 1;
                l
            }
        };
        labels.push(label);
    }
    let joined: BTreeSet<u32> = welded.iter().flatten().copied().collect();
    let label = |i: u32| labels.get(i as usize).copied().ok_or_else(|| EngineError::Params(format!("node {i} has no label")));
    let elements: Vec<(ElementKind, Vec<u32>)> =
        hexes.iter().map(|h| Ok((ElementKind::Hex8, h.iter().map(|&i| label(i)).collect::<Result<Vec<u32>>>()?))).collect::<Result<_>>()?;
    check_conforming(&model, &elements, &joined, merge)?;
    let body_tag = model.geometry_tag(&GeometryRef::body(body.name.as_str())?)?;
    for (l, (kind, nodes)) in (first_element..).zip(elements) {
        model.set_element(l, kind, nodes)?;
        model.tag_element(l, body_tag)?;
    }
    super::hex::tag_faces(&mut model, std::slice::from_ref(body), |_| 0, boundary, first_element)?;
    s.model = model;
    let reply = json!({"first_node": first_node, "first_element": first_element, "nodes": added, "joined": joined.len()});
    let reply = cleared.report(s, reply);
    Ok((reply, first_element, joined.len()))
}

fn extend(reply: &mut Value, more: Value) {
    if let (Some(o), Some(m)) = (reply.as_object_mut(), more.as_object()) {
        for (k, v) in m {
            o.insert(k.clone(), v.clone());
        }
    }
}

fn round3(x: f64) -> Value {
    if x.is_finite() { json!((x * 1000.0).round() / 1000.0) } else { Value::Null }
}

fn bounded<T>(v: &[T], what: &str) -> Result<()> {
    if v.len() > MAX_LIST { Err(EngineError::Params(format!("`{what}` lists more than {MAX_LIST} entries"))) } else { Ok(()) }
}

/// The blocks for a reply: corners (positions before projection) and divisions.
fn listing(t: &Topology, divisions: Option<&[[usize; 3]]>) -> Vec<Value> {
    t.blocks
        .iter()
        .enumerate()
        .take(500)
        .map(|(i, v)| {
            let corners: Vec<Value> =
                v.iter().map(|&k| t.vertices.get(k as usize).map_or(Value::Null, |p| json!(p.map(|x| (x * 1e4).round() / 1e4)))).collect();
            let mut b = json!({"block": i, "corners": corners});
            if let Some(d) = divisions.and_then(|d| d.get(i)) {
                b["divisions"] = json!(d);
            }
            b
        })
        .collect()
}

/// The base topology from the template parameters.
fn topology(p: &ProjectParams, lo: [f64; 3], hi: [f64; 3]) -> Result<Topology> {
    let centre = [0, 1, 2].map(|d| 0.5 * (lo[d] + hi[d]));
    let inset = p.inset.unwrap_or(0.55);
    let t = match p.template.as_deref().unwrap_or(if p.blocks.is_some() { "blocks" } else { "grid" }) {
        "sphere" => {
            let r = p.radius.unwrap_or_else(|| (0..3).map(|d| 0.5 * (hi[d] - lo[d])).fold(0.0, f64::max));
            Topology::sphere(p.center.unwrap_or(centre), r, inset)?
        }
        "butterfly" => {
            let axis = p.axis.unwrap_or([0.0, 0.0, 1.0]);
            let c = p.center.unwrap_or(centre);
            let len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
            if !(len > 0.0 && len.is_finite()) {
                return Err(EngineError::invalid("axis", "must be a nonzero direction"));
            }
            let a = axis.map(|x| x / len);
            // The bounding box's extent along the axis and across it.
            let corners: Vec<[f64; 3]> = (0..8).map(|k| [0, 1, 2].map(|d| if k >> d & 1 == 1 { hi[d] } else { lo[d] })).collect();
            let along: Vec<f64> = corners.iter().map(|q| (0..3).map(|d| (q[d] - c[d]) * a[d]).sum()).collect();
            let across = corners
                .iter()
                .zip(&along)
                .map(|(q, s)| (0..3).map(|d| (q[d] - c[d] - s * a[d]).powi(2)).sum::<f64>().sqrt() / 2f64.sqrt())
                .fold(0.0, f64::max);
            let r = p.radius.unwrap_or(across);
            let levels = p.levels.clone().unwrap_or_else(|| {
                vec![along.iter().copied().fold(f64::INFINITY, f64::min), along.iter().copied().fold(f64::NEG_INFINITY, f64::max)]
            });
            if levels.len() > 1001 {
                return Err(EngineError::Params("`levels` lists more than 1001 levels".into()));
            }
            Topology::butterfly(c, a, r, &levels, inset)?
        }
        "blocks" => {
            let b = p.blocks.as_ref().ok_or_else(|| EngineError::Params("template `blocks` needs `blocks`: {vertices, blocks}".into()))?;
            bounded(&b.blocks, "blocks.blocks")?;
            if b.vertices.len() > 8 * MAX_LIST {
                return Err(EngineError::Params("`blocks.vertices` lists too many vertices".into()));
            }
            Topology { vertices: b.vertices.clone(), blocks: b.blocks.clone() }
        }
        "grid" => {
            let g = p.grid.as_ref();
            let planes: [Vec<f64>; 3] = match g.and_then(|g| g.planes.as_ref()) {
                Some(pl) => {
                    if let Some(k) = pl.keys().find(|k| !matches!(k.as_str(), "x" | "y" | "z")) {
                        return Err(EngineError::Params(format!("grid planes are keyed x, y and z, not `{k}`")));
                    }
                    [("x", 0), ("y", 1), ("z", 2)].map(|(k, d)| pl.get(k).cloned().unwrap_or_else(|| vec![lo[d], hi[d]]))
                }
                None => {
                    let cells = g.and_then(|g| g.cells).unwrap_or([1, 1, 1]);
                    if cells.iter().any(|&c| !(1..=1000).contains(&c)) {
                        return Err(EngineError::invalid("grid.cells", "1 to 1000 per direction"));
                    }
                    [0, 1, 2].map(|d| (0..=cells[d]).map(|q| lo[d] + (hi[d] - lo[d]) * q as f64 / cells[d] as f64).collect())
                }
            };
            if planes.iter().any(|p| p.len() > 1001) {
                return Err(EngineError::Params("at most 1000 grid cells per direction".into()));
            }
            let mut t = Topology::grid([&planes[0], &planes[1], &planes[2]])?;
            if let Some(carve) = g.map(|g| &g.carve).filter(|c| !c.is_empty()) {
                bounded(carve, "grid.carve")?;
                let n = planes.each_ref().map(|p| p.len() - 1);
                let mut ranges = Vec::with_capacity(carve.len());
                for c in carve {
                    if let Some(k) = c.keys().find(|k| !matches!(k.as_str(), "i" | "j" | "k")) {
                        return Err(EngineError::Params(format!("carve ranges are keyed i, j and k, not `{k}`")));
                    }
                    ranges.push([("i", 0), ("j", 1), ("k", 2)].map(|(k, d)| c.get(k).copied().unwrap_or([0, n[d].saturating_sub(1)])));
                }
                t.carve(n, &ranges)?;
            }
            t
        }
        other => return Err(EngineError::Params(format!("unknown template `{other}` (grid, sphere, butterfly or blocks)"))),
    };
    Ok(t)
}

fn hex_project(s: &mut Session, raw: &Value) -> Result<Value> {
    let p: ProjectParams = serde_json::from_value(raw.clone()).map_err(|e| EngineError::Params(format!("mesh.hex.project: {e}")))?;
    bounded(&p.operations, "operations")?;
    bounded(&p.faces, "faces")?;
    bounded(&p.divisions, "divisions")?;
    let b = open_body(s, &p.body, p.tolerance, raw)?;
    let diag = b.input.diagonal;
    let size = element_size(p.size, diag)?;
    let mut t = topology(&p, b.lo, b.hi)?;
    operate(&mut t, &p.operations)?;
    if p.preview {
        return Ok(json!({"body": p.body, "blocks": t.blocks.len(), "topology": listing(&t, None)}));
    }
    let mut faces = Vec::with_capacity(p.faces.len());
    for f in &p.faces {
        bounded(&f.to, "faces.to")?;
        faces.push((f.block, face_of(&t, f.block, &f.face)?, f.to.iter().map(|n| face_index(b.shape, n)).collect::<Result<Vec<u32>>>()?));
    }
    let mut divisions = Vec::with_capacity(p.divisions.len());
    for d in &p.divisions {
        if !(1..=3).contains(&d.direction) {
            return Err(EngineError::invalid("divisions.direction", "1, 2 or 3 (along 1→2, 1→4, 1→5)"));
        }
        divisions.push((d.block, d.direction - 1, d.count));
    }
    let smoothing = match p.smoothing.as_deref().unwrap_or("elliptic") {
        "elliptic" => Smoothing::Elliptic,
        "elliptic-controlled" => Smoothing::EllipticControlled,
        "laplace" => Smoothing::Laplace,
        "none" => Smoothing::None,
        other => return Err(EngineError::Params(format!("unknown smoothing `{other}` (elliptic, elliptic-controlled, laplace or none)"))),
    };
    let iterations = p.iterations.unwrap_or(100);
    if iterations > 10_000 {
        return Err(EngineError::invalid("iterations", "at most 10000"));
    }
    let min_quality = p.min_quality.unwrap_or(fieldcraft_mesh::project::DEFAULT_MIN_QUALITY);
    if !(-1.0..=1.0).contains(&min_quality) {
        return Err(EngineError::invalid("min_quality", "from -1 to 1"));
    }
    let opts = ProjectOptions { size, divisions, faces, smoothing, iterations, max_elements: fieldcraft_model::MAX_ELEMENTS / 8, min_quality };
    let m = project_mesh(&t, &b.surface, &b.target, &opts).map_err(|e| EngineError::of_body(&p.body, e))?;
    let merge = merge_distance(p.merge, diag)?;
    let r = &m.report;
    let names = b.input.face_names.clone();
    let face_name = |f: &u32| names.get(*f as usize).cloned().unwrap_or_else(|| f.to_string());
    let blocks = listing(&t, Some(&r.divisions));
    let targets: Vec<Value> = r.targets.iter().map(|set| json!(set.iter().map(face_name).collect::<Vec<_>>())).collect();
    let mut reply = json!({
        "body": p.body,
        "kind": "hex8",
        "blocks": r.blocks,
        "elements": r.elements,
        "initial_min_scaled_jacobian": round3(r.initial_min_scaled_jacobian),
        "min_scaled_jacobian": round3(r.min_scaled_jacobian),
        "mean_scaled_jacobian": round3(r.mean_scaled_jacobian),
        "smoothing": {"iterations": r.smoothing.iterations, "refused": r.smoothing.refused},
        "max_gap": r.max_gap,
        "volume": r.volume,
        "body_volume": r.surface_volume,
        "targets": targets,
        "topology": blocks,
    });
    let (more, _, _) = add_mesh(s, &b.input, &m.nodes, &m.hexes, &m.boundary, merge)?;
    extend(&mut reply, more);
    Ok(reply)
}

#[cfg(test)]
mod tests;
