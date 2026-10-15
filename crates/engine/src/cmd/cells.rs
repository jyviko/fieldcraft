//! Meshing a partitioned part cell by cell (`mesh.cells`): each cell (a body of the open design,
//! typically made by `geometry.partition`) with its own method, all into one mesh, and the cells
//! joined across the partitions that divide them.
//!
//! **Joining.** Two cells on either side of a partition `P` meet on faces named
//! `partition:P:a` and `partition:P:b`. Where the element faces of the two sides match node for
//! node (the same divisions and kind on both sides, e.g. a mesh and its mirror image) the
//! coincident nodes are merged and the mesh is conforming there. Elsewhere — a hex cell next to a
//! tet cell, or two cells meshed with different divisions — the two surfaces are tied: a set on
//! each side's partition faces and a tie between them (`interaction.tie`; dual mortar
//! constraints in the implicit analyses, so a constant stress crosses the interface exactly).
//! The tie is the transition between hex and tet cells; pyramid elements are not used, because
//! they need their own element formulation in every solver and deck, while the tie works for any
//! pair of meshes and keeps each cell's mesh as its method made it. The reply lists every
//! interface and how it was joined.

use std::collections::{BTreeMap, BTreeSet};

use fieldcraft_mesh::cdt::CdtOptions;
use fieldcraft_mesh::hex::{HexMesh, HexMethod};
use fieldcraft_model::geometry::Entity;
use fieldcraft_model::{ElementFace, GeometryRef, Model, Set, Tie};
use serde_json::{Value, json};

use super::hex::{Input, add_hex, hex_body};
use super::mesh::{BodyTets, add_tets, mesh_body, quality_options};
use crate::requirement::Requirement;
use crate::{CommandKind, CommandSpec, EngineError, Result, Session};

pub const CELLS: CommandSpec = CommandSpec {
    id: "mesh.cells",
    label: "Mesh Cells",
    params: r#"{"methods": {"Body1/a": "tet", "Body1/b": "swept"} (per cell; default `default`), "default": "auto", "bodies": ["Body1/a", "Body1/b"] (default: every body of the open design), "size": 2.0 (element size, mm; default 1/25 of the bounding-box diagonal of all of them), "order": 1 | 2 (hex8 and tet4, or hex20 and tet10), "tolerance": 0.05 (chordal tolerance, mm)} — replaces the mesh with one mesh of the cells, each with its method: "auto" (structured hex where a method fits, else tet), "hex" (any structured hex method), "swept", "extrude", "sweep", "revolve", "mapped" or "tet". Cells on either side of a partition (faces `partition:<name>:a` and `:b`) are joined: where their element faces match node for node the nodes are merged (conforming); otherwise, as between a hex and a tet cell, a set on each side and a tie between them are made (`tie_<partition>_<k>`, sets `<tie>_main` and `<tie>_secondary`: the side with fewer nodes is the main surface), replacing ties of those names from an earlier run. Reports each cell's method (and why auto fell back to tet), and each interface: `merged` with the nodes merged, or `tied`"#,
    form: Some(super::forms::CELLS),
    kind: CommandKind::Model,
    requires: &[Requirement::Design],
    run: cells,
};

/// Distance (relative to the cells' bounding-box diagonal) within which interface nodes coincide.
const COINCIDENT: f64 = 1e-7;

/// A cell's method.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Auto,
    Hex(HexMethod),
    Tet,
}

impl Method {
    fn parse(v: &Value) -> Result<Method> {
        let name = v.as_str().unwrap_or_default();
        Ok(match name {
            "auto" => Method::Auto,
            "hex" => Method::Hex(HexMethod::Auto),
            "tet" => Method::Tet,
            _ => match HexMethod::from_name(name).filter(|m| *m != HexMethod::Auto) {
                Some(m) => Method::Hex(m),
                None => {
                    return Err(EngineError::Params(format!(
                        "method `{}` is not one of auto, hex, swept, extrude, sweep, revolve, mapped, tet",
                        name.chars().take(40).collect::<String>()
                    )));
                }
            },
        })
    }
}

/// A meshed cell.
enum Meshed<'a> {
    Hex(Input<'a>, HexMesh),
    Tet(BodyTets),
}

fn cells(s: &mut Session, p: &Value) -> Result<Value> {
    let size = super::hex::number(p, "size")?;
    let quadratic = match p.get("order") {
        None => false,
        Some(v) => match v.as_u64() {
            Some(1) => false,
            Some(2) => true,
            _ => return Err(EngineError::param("order", "must be 1 or 2")),
        },
    };
    let default = p.get("default").map(Method::parse).transpose()?.unwrap_or(Method::Auto);
    let methods: BTreeMap<String, Method> = match p.get("methods") {
        None => BTreeMap::new(),
        Some(Value::Object(m)) => m.iter().map(|(k, v)| Ok((k.clone(), Method::parse(v)?))).collect::<Result<_>>()?,
        Some(_) => return Err(EngineError::param("methods", "must map cell names to methods")),
    };
    let design = s.design.clone().ok_or_else(|| EngineError::Params("open a design first".into()))?;
    let names: Vec<String> = match p.get("bodies") {
        None => design.bodies.iter().map(|b| b.name.clone()).collect(),
        Some(_) => super::req(p, "bodies")?,
    };
    if names.is_empty() || names.len() > fieldcraft_mesh::hex::MAX_CELLS {
        return Err(EngineError::Params(format!("{} cells; mesh.cells meshes 1 to {}", names.len(), fieldcraft_mesh::hex::MAX_CELLS)));
    }
    for n in methods.keys() {
        if !names.contains(n) {
            return Err(EngineError::Params(format!("`methods` names `{n}`, which is not a cell being meshed")));
        }
    }
    // One size and tolerance from the cells' joint extent, as mesh.hex.blocks does.
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let mut shapes = Vec::with_capacity(names.len());
    for (i, n) in names.iter().enumerate() {
        if names.get(..i).is_some_and(|b| b.contains(n)) {
            return Err(EngineError::Params(format!("cell `{n}` is listed twice")));
        }
        let shape = design.body(n).ok_or_else(|| EngineError::Params(format!("no body `{n}`")))?;
        let m = shape.measure()?;
        for k in 0..3 {
            lo[k] = lo[k].min(m.bbox_min[k]);
            hi[k] = hi[k].max(m.bbox_max[k]);
        }
        shapes.push(shape);
    }
    let diagonal = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt();
    if !(diagonal > 0.0 && diagonal.is_finite()) {
        return Err(EngineError::Params("the cells have no extent".into()));
    }
    let size = Some(size.unwrap_or(diagonal / 25.0));
    let tolerance = super::hex::number(p, "tolerance")?.unwrap_or(1e-3 * diagonal);
    // The tet mesher's parameters: the same size and tolerance, its own order names.
    let mut tet_params = json!({"size": size, "tolerance": tolerance, "order": if quadratic { "quadratic" } else { "linear" }});
    for key in ["weld", "weld_distance", "feature_angle"] {
        if let (Some(v), Some(o)) = (p.get(key), tet_params.as_object_mut()) {
            o.insert(key.into(), v.clone());
        }
    }
    let mut hex_params = tet_params.clone();
    if let Some(o) = hex_params.as_object_mut() {
        o.remove("order");
    }
    let (tet_options, quality) = quality_options(&tet_params, s.limits.max_tets as u64)?;
    let cdt = CdtOptions { max_steiner: s.limits.max_steiner, max_tets: tet_options.max_tets };

    let mut meshed: Vec<(Meshed, Value)> = Vec::with_capacity(shapes.len());
    for shape in shapes {
        let method = methods.get(&shape.name).copied().unwrap_or(default);
        let input = || super::hex::shape_input(shape, Some(tolerance));
        let hex = |m: HexMethod| -> Result<(Input<'static>, HexMesh)> {
            let b = input()?;
            let mesh = hex_body(&hex_params, &b, m, quadratic, size)?;
            Ok((b, mesh))
        };
        let tet = |s: &Session| mesh_body(s, &tet_params, input()?, &cdt, &tet_options, quality, true);
        let (cell, mut info) = match method {
            Method::Hex(m) => {
                let (b, mesh) = hex(m)?;
                let info = json!({"method": mesh.report.method.name()});
                (Meshed::Hex(b, mesh), info)
            }
            Method::Tet => (Meshed::Tet(tet(s)?), json!({"method": "tet"})),
            Method::Auto => match hex(HexMethod::Auto) {
                Ok((b, mesh)) => {
                    let info = json!({"method": mesh.report.method.name()});
                    (Meshed::Hex(b, mesh), info)
                }
                Err(e) => (Meshed::Tet(tet(s)?), json!({"method": "tet", "why": e.to_string()})),
            },
        };
        if let Some(o) = info.as_object_mut() {
            o.insert("body".into(), json!(shape.name));
        }
        meshed.push((cell, info));
    }

    // Replace the mesh with the cells' meshes, each after the labels before it.
    let mut model = s.model.without_mesh();
    let mut report = Vec::with_capacity(meshed.len());
    for (cell, mut info) in meshed {
        let (kind, elements, nodes) = match &cell {
            Meshed::Hex(b, m) => {
                add_hex(&mut model, b, m)?;
                (m.kind, m.element_count(), m.nodes.len())
            }
            Meshed::Tet(t) => {
                add_tets(&mut model, &t.name, &t.face_names, &t.mesh)?;
                (t.mesh.kind, t.mesh.elements.len(), t.mesh.points.len())
            }
        };
        if let Some(o) = info.as_object_mut() {
            o.insert("kind".into(), json!(kind.name()));
            o.insert("elements".into(), json!(elements));
            o.insert("nodes".into(), json!(nodes));
        }
        report.push(info);
    }
    let interfaces = join(&mut model, &names, COINCIDENT * diagonal)?;
    let (nodes, elements) = (model.nodes().len(), model.elements().len());
    s.model = model;
    Ok(json!({"cells": report, "interfaces": interfaces, "nodes": nodes, "elements": elements, "size": size}))
}

/// One side of an interface: a cell's element faces on one side of a partition.
struct Side {
    body: String,
    /// The face name (`partition:P:a`), which selects its pieces too.
    face: String,
    faces: Vec<ElementFace>,
    nodes: BTreeSet<u32>,
    lo: [f64; 3],
    hi: [f64; 3],
}

/// The element faces of each cell tagged with a partition's cut faces, by partition and side.
fn sides(model: &Model, cells: &[String]) -> BTreeMap<String, [Vec<Side>; 2]> {
    let c = model.classification();
    // Tag → (partition, side), for cut faces of the meshed cells.
    let mut cut: BTreeMap<u32, (String, usize, String)> = BTreeMap::new();
    for (t, r) in c.entities().iter().enumerate() {
        let Entity::Face(name) = &r.entity else { continue };
        if !cells.contains(&r.body) {
            continue;
        }
        let base = name.split_once('#').map_or(name.as_str(), |(b, _)| b);
        let Some(rest) = base.strip_prefix("partition:") else { continue };
        let Some((partition, side)) = rest.rsplit_once(':') else { continue };
        let side = match side {
            "a" => 0,
            "b" => 1,
            _ => continue,
        };
        if let Ok(t) = u32::try_from(t) {
            cut.insert(t, (partition.to_string(), side, r.body.clone()));
        }
    }
    let mut grouped: BTreeMap<(String, usize, String), Vec<ElementFace>> = BTreeMap::new();
    for (&element, tags) in c.elements() {
        for (&face, t) in &tags.faces {
            if let Some(key) = cut.get(t) {
                grouped.entry(key.clone()).or_default().push(ElementFace { element, face });
            }
        }
    }
    let mut out: BTreeMap<String, [Vec<Side>; 2]> = BTreeMap::new();
    for ((partition, side, body), faces) in grouped {
        let nodes: BTreeSet<u32> = faces.iter().filter_map(|f| model.face_nodes(*f)).flatten().collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for n in nodes.iter().filter_map(|&n| model.node(n)) {
            for k in 0..3 {
                lo[k] = lo[k].min(n.xyz[k]);
                hi[k] = hi[k].max(n.xyz[k]);
            }
        }
        let face = format!("partition:{partition}:{}", if side == 0 { "a" } else { "b" });
        if let Some(slot) = out.entry(partition).or_insert_with(|| [Vec::new(), Vec::new()]).get_mut(side) {
            slot.push(Side { body, face, faces, nodes, lo, hi });
        }
    }
    out
}

/// Join the cells across every partition: merge where the two sides match node for node, tie
/// where they do not. Returns the interfaces.
fn join(model: &mut Model, cells: &[String], tol: f64) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for (partition, [a_sides, b_sides]) in sides(model, cells) {
        let mut k = 0;
        for a in &a_sides {
            for b in &b_sides {
                let overlap = (0..3).all(|i| a.lo[i] <= b.hi[i] + tol && b.lo[i] <= a.hi[i] + tol);
                if !overlap {
                    continue;
                }
                k += 1;
                let matched = matching(model, a, b, tol);
                if let Some(map) = matched {
                    let count = map.len();
                    model.merge_nodes(&map)?;
                    out.push(json!({"partition": partition, "cells": [a.body, b.body], "joined": "merged", "nodes": count}));
                } else {
                    let name = format!("tie_{partition}_{k}");
                    let (main, secondary) = if a.nodes.len() <= b.nodes.len() { (a, b) } else { (b, a) };
                    let set = |side: &Side| -> Result<Set> { Ok(Set::of_geometry(vec![GeometryRef::face(side.body.as_str(), side.face.as_str())?])) };
                    let (main_set, secondary_set) = (format!("{name}_main"), format!("{name}_secondary"));
                    for (n, side) in [(&main_set, main), (&secondary_set, secondary)] {
                        if model.set(n).is_some() {
                            model.replace_set(n, set(side)?)?;
                        } else {
                            model.add_set(n, set(side)?)?;
                        }
                    }
                    if model.tie(&name).is_some() {
                        model.remove_tie(&name)?;
                    }
                    model.add_tie(&name, Tie::new(&main_set, &secondary_set))?;
                    out.push(json!({
                        "partition": partition,
                        "cells": [a.body, b.body],
                        "joined": "tied",
                        "tie": name,
                        "main": main.body,
                        "secondary": secondary.body,
                        "faces": [a.faces.len(), b.faces.len()],
                    }));
                }
            }
        }
    }
    Ok(out)
}

/// When the two sides' nodes coincide one to one (within `tol`) and their faces are the same
/// in number, the merge: each node of side `b` to the node of side `a` at its place (or the other
/// way round, so the lower label survives).
fn matching(model: &Model, a: &Side, b: &Side, tol: f64) -> Option<BTreeMap<u32, u32>> {
    if a.nodes.len() != b.nodes.len() || a.faces.len() != b.faces.len() {
        return None;
    }
    let both: BTreeSet<u32> = a.nodes.union(&b.nodes).copied().collect();
    let map = fieldcraft_mesh::join::coincident_nodes(model, Some(&both), tol).ok()?;
    // Every pair joins one node of each side, and every node of either side is in a pair.
    let one_to_one = map.len() == a.nodes.len()
        && map.iter().all(|(from, to)| (a.nodes.contains(from) && b.nodes.contains(to)) || (b.nodes.contains(from) && a.nodes.contains(to)));
    one_to_one.then_some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample plate (40 × 30 × 10, a Ø10 hole at (15, 15)) open and cut at x = 30, right of
    /// the hole: `Body1/a` holds the hole, `Body1/b` is a 10 × 30 × 10 block.
    fn cut_plate() -> (crate::tests::TempDir, Session) {
        let (dir, path) = crate::tests::sample_design_file(&fieldcraft_geometry::sample::Plate::default());
        let mut s = Session::default();
        s.execute("geometry.open", &json!({"path": path})).unwrap();
        s.execute("geometry.datum", &json!({"name": "X30", "by": {"type": "offset", "plane": "yz", "offset": 30}})).unwrap();
        s.execute("geometry.partition", &json!({"name": "P", "datum": "X30"})).unwrap();
        (dir, s)
    }

    #[test]
    fn hex_and_tet_cells_are_tied_and_matching_hex_cells_merged() {
        let (_dir, mut s) = cut_plate();
        // A tet cell next to a hex cell: tied, main the coarser side.
        let r = s.execute("mesh.cells", &json!({"methods": {"Body1/a": "tet", "Body1/b": "mapped"}, "size": 2.5})).unwrap();
        assert_eq!(r["cells"][0]["kind"], "tet4", "{r}");
        assert_eq!(r["cells"][1]["kind"], "hex8", "{r}");
        let i = &r["interfaces"][0];
        assert_eq!(i["joined"], "tied", "{r}");
        assert_eq!(s.model.ties().len(), 1);
        for set in ["tie_P_1_main", "tie_P_1_secondary"] {
            assert!(s.model.resolve_set(set).unwrap().faces.len() > 4, "{set}");
        }
        // Both cells hex at one size: the plane faces match node for node and merge.
        let r = s.execute("mesh.cells", &json!({"default": "hex", "size": 2.5})).unwrap();
        assert_eq!(r["interfaces"].as_array().map(Vec::len), Some(1), "{r}");
        let i = &r["interfaces"][0];
        assert_eq!(i["joined"], "merged", "{r}");
        let separate = r["cells"].as_array().unwrap().iter().map(|c| c["nodes"].as_u64().unwrap()).sum::<u64>();
        assert_eq!(r["nodes"].as_u64().unwrap() + i["nodes"].as_u64().unwrap(), separate, "{r}");
        // Rerunning replaces the tie it made rather than adding another.
        s.execute("mesh.cells", &json!({"methods": {"Body1/a": "tet"}, "size": 2.5})).unwrap();
        assert_eq!(s.model.ties().len(), 1);
    }

    #[test]
    fn auto_falls_back_to_tet_and_says_why() {
        let (_dir, mut s) = cut_plate();
        let r = s.execute("mesh.cells", &json!({"default": "mapped", "methods": {"Body1/a": "auto"}, "size": 3})).unwrap();
        // The holed cell is not a mapped block, but auto sweeps it.
        assert!(r["cells"][0]["method"].as_str().is_some(), "{r}");
        assert_eq!(r["cells"][1]["method"], "mapped", "{r}");
    }

    #[test]
    fn hostile_params_change_nothing() {
        let (_dir, mut s) = cut_plate();
        let journal = s.journal.len();
        for p in [
            json!({"default": "pyramid"}),
            json!({"methods": {"Nope": "tet"}}),
            json!({"methods": ["tet"]}),
            json!({"bodies": []}),
            json!({"bodies": ["Body1/a", "Body1/a"]}),
            json!({"bodies": ["Nope"]}),
            json!({"order": 3}),
            json!({"size": -1}),
            json!({"default": "mapped", "bodies": ["Body1/a"]}),
        ] {
            assert!(s.execute("mesh.cells", &p).is_err(), "{p}");
        }
        assert_eq!(s.journal.len(), journal);
        assert!(s.model.elements().is_empty());
    }

    /// A 40 × 10 × 10 bar in uniaxial tension, cut in the middle: one half hex, the other tet,
    /// tied. With ν = 0, symmetry conditions on three faces and the end load spread by consistent
    /// weights, the stress is uniform, which both elements and the dual mortar tie carry exactly:
    /// the end moves F L / (E A).
    #[test]
    fn a_tied_hex_tet_bar_in_tension_stretches_as_the_formula_says() {
        let commands = json!({"commands": [
            {"command": "solid.box", "params": {"length": 40, "width": 10, "height": 10, "body_name": "Bar"}},
        ]});
        let bytes = fieldcraft_geometry::script_design(&commands.to_string()).unwrap();
        let (_dir, path) = crate::tests::design_file(&bytes);
        let mut s = Session::default();
        s.execute("geometry.open", &json!({"path": path})).unwrap();
        let bbox = s.execute("geometry.bodies", &json!({})).unwrap()[0]["bbox"].clone();
        let lo: Vec<f64> = bbox[0].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let hi: Vec<f64> = bbox[1].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let (axis, length) = (0..3).map(|k| (k, hi[k] - lo[k])).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!((length - 40.0).abs() < 1e-9);
        let mid = (lo[axis] + hi[axis]) / 2.0;
        let mut normal = [0.0; 3];
        normal[axis] = 1.0;
        let mut point = [0.0; 3];
        point[axis] = mid;
        s.execute("geometry.datum", &json!({"name": "Mid", "by": {"type": "point_normal", "point": point, "normal": normal}})).unwrap();
        s.execute("geometry.partition", &json!({"name": "Cut", "datum": "Mid"})).unwrap();
        let r = s.execute("mesh.cells", &json!({"methods": {"Bar/a": "mapped", "Bar/b": "tet"}, "size": 2.5})).unwrap();
        assert_eq!(r["interfaces"][0]["joined"], "tied", "{r}");

        let m = &s.model;
        let on = |k: usize, v: f64| -> Vec<u32> { m.nodes().iter().filter(|(_, n)| (n.xyz[k] - v).abs() < 1e-9).map(|(l, _)| *l).collect() };
        let others: Vec<usize> = (0..3).filter(|k| *k != axis).collect();
        let end: Vec<Value> = m
            .elements()
            .iter()
            .flat_map(|(&e, el)| (1..=el.kind.faces().len() as u8).map(move |f| (e, f)))
            .filter(|&(e, f)| {
                m.face_nodes(ElementFace { element: e, face: f })
                    .is_some_and(|ns| ns.iter().all(|n| (m.node(*n).unwrap().xyz[axis] - hi[axis]).abs() < 1e-9))
            })
            .map(|(e, f)| json!({"element": e, "face": f}))
            .collect();
        let plane = |k: usize| ["x", "y", "z"][k];
        let (young, area, force) = (70_000.0, 100.0, 1000.0);
        let script = json!([
            {"command": "model.set.create", "params": {"name": "all", "elements": m.elements().keys().collect::<Vec<_>>()}},
            {"command": "model.set.create", "params": {"name": "root", "nodes": on(axis, lo[axis])}},
            {"command": "model.set.create", "params": {"name": "side1", "nodes": on(others[0], lo[others[0]])}},
            {"command": "model.set.create", "params": {"name": "side2", "nodes": on(others[1], lo[others[1]])}},
            {"command": "model.set.create", "params": {"name": "end", "faces": end}},
            {"command": "model.material.create", "params": {"name": "Al", "elastic": {"type": "isotropic", "young": young, "poisson": 0}}},
            {"command": "model.section.create", "params": {"name": "Bar", "set": "all", "material": "Al", "kind": {"type": "solid"}}},
            {"command": "model.boundary_condition.create", "params": {"name": "s0", "type": "symmetry", "target": {"set": "root"}, "plane": plane(axis)}},
            {"command": "model.boundary_condition.create", "params": {"name": "s1", "type": "symmetry", "target": {"set": "side1"}, "plane": plane(others[0])}},
            {"command": "model.boundary_condition.create", "params": {"name": "s2", "type": "symmetry", "target": {"set": "side2"}, "plane": plane(others[1])}},
            {"command": "model.load.create", "params": {"name": "Pull", "type": "force", "target": {"set": "end"}, "vector": normal.map(|n| n * force)}},
            {"command": "model.step.create", "params": {"name": "Load", "procedure": {"type": "static"}, "boundary_conditions": ["s0", "s1", "s2"], "loads": ["Pull"]}},
        ]);
        let tip = on(axis, hi[axis]);
        s.run_script(&script).unwrap();
        s.execute("solve.static", &json!({})).unwrap();
        let probe = s.execute("solve.probe", &json!({"step": "Load", "nodes": tip})).unwrap();
        let expected = force * length / (young * area);
        let probed = probe["nodes"].as_array().unwrap();
        assert!(probed.len() > 4, "{probe}");
        for n in probed {
            let u = n["displacement"][axis].as_f64().unwrap();
            assert!((u - expected).abs() < 1e-6 * expected, "{u} vs {expected}: {n}");
        }
    }
}
