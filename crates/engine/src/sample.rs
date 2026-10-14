//! Sample models as scripts of ordinary commands, so they build the same way a user's script
//! does.
//!
//! [`script`], the model the desktop app opens with `--sample`: two parts side by side, a
//! 40 × 20 × 10 block of 5 mm hex8 cells, and a 15 mm cube split into 162 tet4 elements (each
//! 5 mm cell into six tetrahedra around its main diagonal, the Kuhn subdivision, which is
//! conforming between neighbouring cells). Every element is positively oriented.
//!
//! [`cantilever`], a model ready to solve: a steel beam clamped at one end with a load on the
//! other, a static and a frequency step. [`impact`]: the same beam flying onto a rigid wall, an
//! explicit dynamic step.
//!
//! The examples gallery (docs/examples.md), each ready to solve with `solve.run`:
//! [`drop_test`], [`rubber_seal`], [`flow_past`], [`heated_bracket`] and [`bracket_hex`]. Their
//! geometry is a SolidCraft script run when the sample opens ([`Sample::design`]): a corpus
//! recipe (`tests/corpus`) or [`UPRIGHT_BRACKET`].

use fieldcraft_geometry::Shape;
use serde_json::{Value, json};

/// A sample the app offers (Help › Samples, the empty viewport, `file.sample`, `--sample NAME`).
pub struct Sample {
    pub name: &'static str,
    pub label: &'static str,
    /// One line: what it shows.
    pub about: &'static str,
    /// The SolidCraft script (`{"commands": [...]}`, the corpus recipe form) that builds the
    /// sample's geometry, opened as its design before [`Sample::script`] runs.
    pub design: Option<&'static str>,
    /// The model's commands, given the design's bodies (none without a design).
    pub script: fn(&[Shape]) -> Value,
}

/// Every sample, the default first.
pub const SAMPLES: &[Sample] = &[
    Sample {
        name: "parts",
        label: "Hex and Tet Parts",
        about: "A hex8 block beside a tet4 cube: meshes to look at, nothing to solve",
        design: None,
        script: |_| script(),
    },
    Sample {
        name: "cantilever",
        label: "Cantilever",
        about: "A clamped steel beam with an end load: a static and a frequency step, ready to solve",
        design: None,
        script: |_| cantilever(),
    },
    Sample {
        name: "impact",
        label: "Bar Impact",
        about: "The steel beam flying onto a rigid wall: an explicit dynamic step, ready to solve",
        design: None,
        script: |_| impact(),
    },
];

/// The sample called `name`.
pub fn find(name: &str) -> Option<&'static Sample> {
    SAMPLES.iter().find(|s| s.name == name)
}

/// Hex block: cells along x, y, z and the cell size (mm).
const HEX_CELLS: [u32; 3] = [8, 4, 2];
/// Tet cube: cells per side and its offset from the origin (mm).
const TET_CELLS: u32 = 3;
const TET_ORIGIN: [f64; 3] = [50.0, 2.5, 0.0];
const CELL: f64 = 5.0;

/// The script: `mesh.node.add` and `mesh.element.add` steps.
pub fn script() -> Value {
    let mut steps = Vec::new();
    let mut node = 0u32;
    let mut element = 0u32;
    // Hex block.
    let [nx, ny, nz] = HEX_CELLS;
    let first = node + 1;
    let hid = |i: u32, j: u32, k: u32| first + i + j * (nx + 1) + k * (nx + 1) * (ny + 1);
    for k in 0..=nz {
        for j in 0..=ny {
            for i in 0..=nx {
                node += 1;
                steps.push(json!({"command": "mesh.node.add", "params": {"label": node, "xyz": [f64::from(i) * CELL, f64::from(j) * CELL, f64::from(k) * CELL]}}));
            }
        }
    }
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                element += 1;
                let nodes = [
                    hid(i, j, k),
                    hid(i + 1, j, k),
                    hid(i + 1, j + 1, k),
                    hid(i, j + 1, k),
                    hid(i, j, k + 1),
                    hid(i + 1, j, k + 1),
                    hid(i + 1, j + 1, k + 1),
                    hid(i, j + 1, k + 1),
                ];
                steps.push(json!({"command": "mesh.element.add", "params": {"label": element, "kind": "hex8", "nodes": nodes}}));
            }
        }
    }
    // Tet cube.
    let n = TET_CELLS;
    let first = node + 1;
    let tid = |i: u32, j: u32, k: u32| first + i + j * (n + 1) + k * (n + 1) * (n + 1);
    let at = |i: u32, j: u32, k: u32| [TET_ORIGIN[0] + f64::from(i) * CELL, TET_ORIGIN[1] + f64::from(j) * CELL, TET_ORIGIN[2] + f64::from(k) * CELL];
    for k in 0..=n {
        for j in 0..=n {
            for i in 0..=n {
                node += 1;
                steps.push(json!({"command": "mesh.node.add", "params": {"label": node, "xyz": at(i, j, k)}}));
            }
        }
    }
    // The six axis orders: each walks from the cell's corner (0,0,0) to (1,1,1) one axis at a time.
    const ORDERS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    for k in 0..n {
        for j in 0..n {
            for i in 0..n {
                for order in ORDERS {
                    let mut c = [0u32; 3];
                    let mut corners = vec![c];
                    for axis in order {
                        if let Some(x) = c.get_mut(axis) {
                            *x = 1;
                        }
                        corners.push(c);
                    }
                    let mut nodes: Vec<u32> = corners.iter().map(|c| tid(i + c[0], j + c[1], k + c[2])).collect();
                    let p: Vec<[f64; 3]> = corners.iter().map(|c| at(i + c[0], j + c[1], k + c[2])).collect();
                    if signed_volume(&p) < 0.0 {
                        nodes.swap(1, 2);
                    }
                    element += 1;
                    steps.push(json!({"command": "mesh.element.add", "params": {"label": element, "kind": "tet4", "nodes": nodes}}));
                }
            }
        }
    }
    Value::Array(steps)
}

/// The cantilever's length (x), width (y) and height (z) in mm, and its hex8 cells along each.
pub const CANTILEVER_SIZE: [f64; 3] = [100.0, 10.0, 20.0];
const CANTILEVER_CELLS: [u32; 3] = [20, 2, 4];

/// The beam's mesh in steel (E = 210 GPa, ν = 0.3, ρ = 7.85 t/m³ in mm, t, s, N, MPa): nodes,
/// hex8 elements, sets `beam` (elements), `root` (nodes at x = 0) and `tip` (at x = L), the
/// material `Steel` and a solid section `Beam` with the given formulation.
fn beam(formulation: Value) -> Vec<Value> {
    let [nx, ny, nz] = CANTILEVER_CELLS;
    let [lx, ly, lz] = CANTILEVER_SIZE;
    let id = |i: u32, j: u32, k: u32| 1 + i + j * (nx + 1) + k * (nx + 1) * (ny + 1);
    let mut steps = vec![json!({"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}})];
    for k in 0..=nz {
        for j in 0..=ny {
            for i in 0..=nx {
                let xyz = [lx * f64::from(i) / f64::from(nx), ly * f64::from(j) / f64::from(ny), lz * f64::from(k) / f64::from(nz)];
                steps.push(json!({"command": "mesh.node.add", "params": {"label": id(i, j, k), "xyz": xyz}}));
            }
        }
    }
    let mut element = 0u32;
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                element += 1;
                let nodes = [
                    id(i, j, k),
                    id(i + 1, j, k),
                    id(i + 1, j + 1, k),
                    id(i, j + 1, k),
                    id(i, j, k + 1),
                    id(i + 1, j, k + 1),
                    id(i + 1, j + 1, k + 1),
                    id(i, j + 1, k + 1),
                ];
                steps.push(json!({"command": "mesh.element.add", "params": {"label": element, "kind": "hex8", "nodes": nodes}}));
            }
        }
    }
    let end = |i: u32| -> Vec<u32> { (0..=nz).flat_map(|k| (0..=ny).map(move |j| id(i, j, k))).collect() };
    steps.extend([
        json!({"command": "model.set.create", "params": {"name": "beam", "elements": (1..=element).collect::<Vec<_>>()}}),
        json!({"command": "model.set.create", "params": {"name": "root", "nodes": end(0)}}),
        json!({"command": "model.set.create", "params": {"name": "tip", "nodes": end(nx)}}),
        json!({"command": "model.material.create", "params": {"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}, "density": 7.85e-9}}),
        json!({"command": "model.section.create", "params": {"name": "Beam", "set": "beam", "material": "Steel", "kind": {"type": "solid"}, "formulation": formulation}}),
    ]);
    steps
}

/// A steel cantilever of hex8 elements with incompatible modes ([`CANTILEVER_SIZE`]), clamped at
/// x = 0 (set `root`), with a total force of 1000 N down on the free end (set `tip`); step
/// `Load` (static) and step `Modes` (six modes). Euler–Bernoulli puts the tip
/// 4 P L³ / (E w h³) = 0.238 mm down.
pub fn cantilever() -> Value {
    let mut steps = beam(json!({"incompatible_modes": true}));
    steps.extend([
        json!({"command": "model.boundary_condition.create", "params": {"name": "Clamp", "type": "fixed", "target": {"set": "root"}}}),
        json!({"command": "model.load.create", "params": {"name": "Push", "type": "force", "target": {"set": "tip"}, "vector": [0, 0, -1000]}}),
        json!({"command": "model.step.create", "params": {"name": "Load", "procedure": {"type": "static"}, "boundary_conditions": ["Clamp"], "loads": ["Push"]}}),
        json!({"command": "model.step.create", "params": {"name": "Modes", "procedure": {"type": "frequency", "modes": 6}, "boundary_conditions": ["Clamp"]}}),
    ]);
    Value::Array(steps)
}

/// The same steel beam in one-point hex8 elements, flying at 10 m/s onto a rigid wall at x = 0:
/// step `Impact` (explicit dynamics, 40 µs, ten field frames, energies and the tip's history).
/// The compression wave runs the 100 mm bar in about 19 µs (c = √(E/ρ) ≈ 5.2 mm/µs).
pub fn impact() -> Value {
    let mut steps = beam(json!({"integration": "reduced"}));
    let step = json!({
        "type": "explicit_dynamic", "end_time": 4e-5, "output_interval": 4e-6, "history_interval": 2e-7, "history_nodes": "tip",
        "initial_velocities": [{"set": "all_nodes", "velocity": [-10000, 0, 0]}],
        "rigid_walls": [{"set": "root", "point": [0, 0, 0], "normal": [1, 0, 0]}]
    });
    let [nx, ny, nz] = CANTILEVER_CELLS;
    let nodes = (nx + 1) * (ny + 1) * (nz + 1);
    steps.extend([
        json!({"command": "model.set.create", "params": {"name": "all_nodes", "nodes": (1..=nodes).collect::<Vec<_>>()}}),
        json!({"command": "model.step.create", "params": {"name": "Impact", "procedure": step}}),
    ]);
    Value::Array(steps)
}

/// Six times the signed volume of the tetrahedron `p[0..4]` (positive when p3 lies on the side
/// p0 → p1 → p2 turns counter-clockwise around).
fn signed_volume(p: &[[f64; 3]]) -> f64 {
    let (Some(a), Some(b), Some(c), Some(d)) = (p.first(), p.get(1), p.get(2), p.get(3)) else { return 0.0 };
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let w = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
    u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0]) + u[2] * (v[0] * w[1] - v[1] * w[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    /// The `file.sample` form offers every sample, by name.
    #[test]
    fn the_form_lists_every_sample() {
        let Some(crate::schema::Kind::Choice(options)) = crate::cmd::forms::SAMPLE.first().map(|f| f.kind) else { panic!("not a choice") };
        let mut offered: Vec<&str> = options.iter().map(|(n, _)| *n).collect();
        let mut names: Vec<&str> = SAMPLES.iter().map(|s| s.name).collect();
        offered.sort_unstable();
        names.sort_unstable();
        assert_eq!(offered, names);
    }

    #[test]
    fn the_sample_builds_positive_elements_that_fill_both_parts() {
        let mut s = Session::default();
        s.run_script(&script()).unwrap();
        let info = s.execute("model.info", &json!({})).unwrap();
        assert_eq!(info["elements"]["hex8"], 64);
        assert_eq!(info["elements"]["tet4"], 162);
        assert_eq!(info["nodes"], 9 * 5 * 3 + 4 * 4 * 4);
        // Tet volumes are positive and add up to the cube's.
        let mut volume = 0.0;
        for e in s.model.elements().values().filter(|e| e.kind == fieldcraft_model::ElementKind::Tet4) {
            let p: Vec<[f64; 3]> = e.nodes.iter().map(|n| s.model.node(*n).unwrap().xyz).collect();
            let v = signed_volume(&p) / 6.0;
            assert!(v > 0.0);
            volume += v;
        }
        assert!((volume - 15.0f64.powi(3)).abs() < 1e-9, "{volume}");
    }

    #[test]
    fn the_cantilever_solves_close_to_beam_theory() {
        let mut s = Session::default();
        s.run_script(&cantilever()).unwrap();
        let r = s.execute("solve.static", &json!({})).unwrap();
        let [l, w, h] = CANTILEVER_SIZE;
        let beam = 4.0 * 1000.0 * l.powi(3) / (210000.0 * w * h.powi(3));
        let tip = r["max_displacement"]["value"].as_f64().unwrap();
        // Shear deformation adds a little (L/h = 5).
        assert!(tip > beam && tip < 1.1 * beam, "{tip} vs {beam}");
        let f = s.execute("solve.frequency", &json!({})).unwrap();
        assert_eq!(f["modes"].as_array().map(Vec::len), Some(6));
    }

    #[test]
    fn the_impact_runs_and_keeps_frames_with_stresses() {
        let mut s = Session::default();
        s.run_script(&impact()).unwrap();
        let r = s.execute("solve.explicit", &json!({})).unwrap();
        assert_eq!((r["completed"].as_bool(), r["frames"].as_u64()), (Some(true), Some(11)), "{r}");
        assert!(r["energy_balance"].as_f64().unwrap() < 0.05, "{r}");
    }
}
