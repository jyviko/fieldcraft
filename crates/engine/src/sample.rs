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
    Sample {
        name: "drop-test",
        label: "Drop Test",
        about: "An aluminium L-bracket dropped 2 m onto a tilted rigid floor: explicit dynamics, the strike and its stress waves",
        design: Some(UPRIGHT_BRACKET),
        script: |_| drop_test(),
    },
    Sample {
        name: "rubber-seal",
        label: "Rubber Seal",
        about: "A rubber gasket ring squeezed 25 % between bonded plates: nonlinear static, Mooney–Rivlin, large strain",
        design: None,
        script: |_| rubber_seal(),
    },
    Sample {
        name: "flow-past",
        label: "Flow Past a Bracket",
        about: "Air flowing past an L-bracket on the floor: lattice Boltzmann flow and the force on each face",
        design: Some(UPRIGHT_BRACKET),
        script: |_| flow_past(),
    },
    Sample {
        name: "heated-bracket",
        label: "Heated Bracket",
        about: "A steel bracket clamped at its holes and heated at one end: steady heat conduction and the thermal stress",
        design: Some(FILLETED_BRACKET),
        script: |_| heated_bracket(),
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

/// An L-bracket standing on its foot, Z up, as a SolidCraft script: a 60 × 40 mm foot and a 50 mm
/// upright, both 8 mm thick, with an R6 fillet inside the bend (the corpus `filleted-bracket`'s
/// profile without its holes, sketched on XZ so it sweeps along y). Body `Bracket`; its faces
/// `F2:side:l1` (the foot's underside, z = 0), `l2` (the foot's end, x = 60), `l3` (the foot's top),
/// `l4` (the upright's inside, x = 8), `l5` (the upright's top, z = 50) and `l6` (its back, x = 0).
pub const UPRIGHT_BRACKET: &str = r#"{"commands": [
    {"command": "sketch.create", "params": {"plane": "XZ", "name": "Profile"}},
    {"command": "sketch.line", "params": {"points": [[0, 0], [60, 0], [60, -8], [8, -8], [8, -50], [0, -50]], "closed": true}},
    {"command": "sketch.finish", "params": {}},
    {"command": "solid.extrude", "params": {"distance": 40, "body_name": "Bracket"}},
    {"command": "solid.fillet", "params": {"edges": [[8, 20, 8]], "radius": 6}}
]}"#;

/// Aluminium (E = 70 GPa, ν = 0.33, ρ = 2.7 t/m³, yield 240 MPa with 700 MPa linear hardening),
/// in mm, t, s, N, MPa.
fn aluminium(name: &str) -> Value {
    json!({"command": "model.material.create", "params": {"name": name, "elastic": {"type": "isotropic", "young": 70000, "poisson": 0.33}, "density": 2.7e-9,
        "plastic": {"type": "bilinear", "yield_stress": 240, "hardening_modulus": 700, "hardening": "isotropic"}}})
}

/// Standard gravity, mm/s².
pub const GRAVITY: f64 = 9810.0;
/// The drop test's height of fall, mm: the bracket meets the floor at √(2 g h) = 6.26 m/s.
pub const DROP_HEIGHT: f64 = 2000.0;
/// The drop test's floor tilt about y, degrees: the foot's end strikes first.
const DROP_TILT: f64 = 15.0;

/// [`UPRIGHT_BRACKET`] in aluminium, hex8 elements swept 3 mm (one-point, stiffness hourglass
/// control), falling at √(2 g h) onto a rigid floor tilted 15° so the foot's end lands first,
/// 0.1 mm below that corner; gravity on. Step `Drop` (explicit dynamics, 1 ms: the strike, the
/// stress waves through the bend and the bracket pivoting onto its foot; 20 field frames, the
/// upright's top (set `top`) in the histories).
pub fn drop_test() -> Value {
    let speed = (2.0 * GRAVITY * DROP_HEIGHT).sqrt();
    let tilt = DROP_TILT.to_radians();
    let step = json!({
        "type": "explicit_dynamic", "end_time": 1e-3, "output_interval": 5e-5, "history_interval": 5e-6, "history_nodes": "top",
        "initial_velocities": [{"set": "bracket", "velocity": [0, 0, -speed]}],
        "rigid_walls": [{"set": "bracket", "point": [60, 0, -0.1], "normal": [-tilt.sin(), 0, tilt.cos()]}],
        "hourglass": {"type": "stiffness", "coefficient": 0.05}
    });
    json!([
        {"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}},
        {"command": "mesh.hex.generate", "params": {"body": "Bracket", "size": 3, "method": "extrude"}},
        {"command": "model.set.create", "params": {"name": "bracket", "geometry": [{"body": "Bracket"}]}},
        {"command": "model.set.create", "params": {"name": "top", "geometry": [{"body": "Bracket", "face": "F2:side:l5"}]}},
        aluminium("Aluminium"),
        {"command": "model.section.create", "params": {"name": "Bracket", "set": "bracket", "material": "Aluminium", "kind": {"type": "solid"}, "formulation": {"integration": "reduced"}}},
        {"command": "model.load.create", "params": {"name": "Gravity", "type": "gravity", "acceleration": [0, 0, -GRAVITY]}},
        {"command": "model.step.create", "params": {"name": "Drop", "procedure": step, "loads": ["Gravity"]}},
    ])
}

/// The rubber seal: inner and outer radius and height of the ring (mm), and how far it is squeezed.
pub const SEAL: [f64; 3] = [20.0, 30.0, 10.0];
pub const SEAL_SQUEEZE: f64 = 2.5;

/// A rubber gasket ring ([`SEAL`]: 20 mm inside radius, 30 mm outside, 10 mm high) of hybrid hex8
/// elements, four mapped quarter blocks with arc edges (body `Seal`, block faces `bottom` and
/// `top`), squeezed by [`SEAL_SQUEEZE`] (25 %) between plates bonded to it: `bottom` fixed, `top`
/// moved down with its in-plane motion held. Generic rubber: E = 6 MPa, ν = 0.49, Mooney–Rivlin
/// with C01 = 0.2 MPa (C10 = μ/2 − C01 = 0.81 MPa). Step `Squeeze` (nonlinear static).
pub fn rubber_seal() -> Value {
    let [inner, outer, height] = SEAL;
    let mut steps = vec![json!({"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}})];
    for quarter in 0..4u8 {
        let (a0, a1) = (f64::from(quarter) * std::f64::consts::FRAC_PI_2, f64::from(quarter + 1) * std::f64::consts::FRAC_PI_2);
        let at = |r: f64, a: f64, z: f64| [r * a.cos(), r * a.sin(), z];
        let corners =
            [at(inner, a0, 0.0), at(outer, a0, 0.0), at(outer, a1, 0.0), at(inner, a1, 0.0), at(inner, a0, height), at(outer, a0, height), at(outer, a1, height), at(inner, a1, height)];
        let arc = |a: u8, b: u8, z: f64| json!({"corners": [a, b], "arc": {"center": [0, 0, z]}});
        steps.push(json!({"command": "mesh.block", "params": {
            "body": "Seal", "kind": "hex8", "corners": corners, "divisions": [5, 10, 5],
            "edges": [arc(1, 4, 0.0), arc(2, 3, 0.0), arc(5, 8, height), arc(6, 7, height)],
            "faces": {"1": "bottom", "2": "top"}
        }}));
    }
    steps.extend([
        json!({"command": "model.set.create", "params": {"name": "seal", "geometry": [{"body": "Seal"}]}}),
        json!({"command": "model.set.create", "params": {"name": "bottom", "geometry": [{"body": "Seal", "face": "bottom"}]}}),
        json!({"command": "model.set.create", "params": {"name": "top", "geometry": [{"body": "Seal", "face": "top"}]}}),
        json!({"command": "model.material.create", "params": {"name": "Rubber", "elastic": {"type": "isotropic", "young": 6, "poisson": 0.49}, "density": 1.1e-9,
            "hyperelastic": {"type": "mooney_rivlin", "c01": 0.2}}}),
        json!({"command": "model.section.create", "params": {"name": "Seal", "set": "seal", "material": "Rubber", "kind": {"type": "solid"}, "formulation": {"hybrid": true}}}),
        json!({"command": "model.boundary_condition.create", "params": {"name": "Base", "type": "fixed", "target": {"set": "bottom"}}}),
        json!({"command": "model.boundary_condition.create", "params": {"name": "Press", "type": "displacement", "target": {"set": "top"}, "values": [0, 0, -SEAL_SQUEEZE, null, null, null]}}),
        json!({"command": "model.step.create", "params": {"name": "Squeeze", "procedure": {"type": "nonlinear_static", "initial_increment": 0.1}, "boundary_conditions": ["Base", "Press"]}}),
    ]);
    Value::Array(steps)
}

/// The flow sample's free-stream speed, mm/s.
pub const FLOW_SPEED: f64 = 25.0;
/// Air at 20 °C: density (t/mm³) and dynamic viscosity (MPa s).
const AIR: [f64; 2] = [1.2e-12, 1.8e-11];

/// Air at [`FLOW_SPEED`] past [`UPRIGHT_BRACKET`] standing on the floor, the upright's back face
/// to the wind: a 240 × 120 × 100 mm box of 2.5 mm cells (184 320), a uniform inlet at x_min, an
/// outlet at zero pressure at x_max, a no-slip floor and slip walls at the sides and top. Reynolds
/// number ρ U h / μ = 83 on the upright's 50 mm height: laminar, a steady wake. Step `Flow` (16 s,
/// about 6.7 flow-throughs; ten frames) reports the force on each named face of the bracket.
pub fn flow_past() -> Value {
    let [density, viscosity] = AIR;
    json!([
        {"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}},
        {"command": "model.material.create", "params": {"name": "Air", "density": density, "fluid": {"type": "newtonian", "viscosity": viscosity}}},
        {"command": "model.fluid_domain.create", "params": {
            "name": "Wind", "material": "Air", "min": [-60, -40, 0], "max": [180, 80, 100], "spacing": 2.5,
            "faces": {
                "x_min": {"type": "velocity", "velocity": [FLOW_SPEED, 0, 0]}, "x_max": {"type": "pressure", "pressure": 0},
                "y_min": {"type": "slip"}, "y_max": {"type": "slip"}, "z_min": {"type": "wall"}, "z_max": {"type": "slip"}
            },
            "obstacles": ["Bracket"], "initial_velocity": [FLOW_SPEED, 0, 0]
        }},
        {"command": "model.step.create", "params": {"name": "Flow", "procedure": {"type": "fluid", "domain": "Wind", "end_time": 16, "frames": 10}}},
    ])
}

/// The corpus filleted bracket (`tests/corpus/filleted-bracket`): an 8 mm L-bracket, foot along
/// x (60 mm) and upright along y (50 mm), extruded 40 mm along z, an R6 fillet, an 8 mm hole
/// through each leg (`F4` through the foot at x = 40, `F5` through the upright at y = 32).
pub const FILLETED_BRACKET: &str = include_str!("../../../tests/corpus/filleted-bracket/recipe.json");

/// The heated bracket: the heater's and the surrounding air's temperatures (°C, also the stress
/// free temperature), and steel's expansion coefficient (1/K).
pub const HEATER: f64 = 120.0;
pub const AMBIENT: f64 = 20.0;
pub const STEEL_EXPANSION: f64 = 1.2e-5;

/// [`FILLETED_BRACKET`] in steel (k = 50 W/(m K), α = 12 × 10⁻⁶/K, stress free at 20 °C), tet10
/// elements of 4 mm: its upright's end (`F2:side:l5`, set `hot`) held at [`HEATER`], the rest of
/// its outside (set `skin`) cooled by still air at [`AMBIENT`] (h = 10 W/(m² K)), both hole bores
/// clamped (set `holes`). Step `Heat` (coupled temperature–displacement, steady): the
/// temperature field and the stress of the expansion the clamps hold back. In mm, t, s, N, MPa:
/// k in mW/(mm K), h in mW/(mm² K).
pub fn heated_bracket() -> Value {
    let face = |f: &str| json!({"body": "Bracket", "face": f});
    let skin: Vec<Value> = ["F2:start", "F2:top", "F2:side:l1", "F2:side:l2", "F2:side:l3", "F2:side:l4", "F2:side:l6", "F3:blend:F2:side:l3|F2:side:l4"]
        .into_iter()
        .map(face)
        .collect();
    let holes: Vec<Value> = ["F4:bore:side:0", "F4:bore:side:1", "F5:bore:side:0", "F5:bore:side:1"].into_iter().map(face).collect();
    json!([
        {"command": "model.units", "params": {"system": "mm-t-s-N-MPa"}},
        {"command": "mesh.tet.generate", "params": {"body": "Bracket", "size": 4, "order": "quadratic", "tolerance": 0.05}},
        {"command": "model.set.create", "params": {"name": "bracket", "geometry": [{"body": "Bracket"}]}},
        {"command": "model.set.create", "params": {"name": "holes", "geometry": holes}},
        {"command": "model.set.create", "params": {"name": "hot", "geometry": [face("F2:side:l5")]}},
        {"command": "model.set.create", "params": {"name": "skin", "geometry": skin}},
        {"command": "model.material.create", "params": {"name": "Steel", "elastic": {"type": "isotropic", "young": 210000, "poisson": 0.3}, "density": 7.85e-9,
            "thermal": {"conductivity": 50, "specific_heat": 4.6e8, "expansion": STEEL_EXPANSION, "reference_temperature": AMBIENT}}},
        {"command": "model.section.create", "params": {"name": "Bracket", "set": "bracket", "material": "Steel", "kind": {"type": "solid"}}},
        {"command": "model.boundary_condition.create", "params": {"name": "Clamp", "type": "fixed", "target": {"set": "holes"}}},
        {"command": "model.boundary_condition.create", "params": {"name": "Heater", "type": "temperature", "target": {"set": "hot"}, "value": HEATER}},
        {"command": "model.load.create", "params": {"name": "Air", "type": "convection", "set": "skin", "film": 0.01, "sink": AMBIENT}},
        {"command": "model.step.create", "params": {"name": "Heat", "procedure": {"type": "coupled_temperature_displacement"}, "boundary_conditions": ["Clamp", "Heater"], "loads": ["Air"]}},
    ])
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

    /// `file.sample NAME`, checked clean: ready, no errors or warnings.
    fn open(name: &str) -> Session {
        let mut s = Session::default();
        s.execute("file.sample", &json!({"name": name})).unwrap();
        let check = s.execute("model.check", &json!({})).unwrap();
        assert_eq!((check["ready"].as_bool(), check["errors"].as_u64(), check["warnings"].as_u64()), (Some(true), Some(0), Some(0)), "{check}");
        s
    }

    /// The squeeze force of the ring, against the closed form for an incompressible Mooney–Rivlin
    /// solid in uniaxial compression, P = 2 (λ − λ⁻²)(C10 + C01/λ) per unit of original area: the
    /// same ring between frictionless plates (each face held only along z, the rigid motions
    /// held at three nodes on the symmetry planes) compresses evenly and matches it within 3 %
    /// (ν = 0.49 is nearly incompressible); bonded, the bulge needs more force (shape factor
    /// ¼ (b − a)/h = 0.5 by the loaded over the free area), within 2.5 times.
    #[test]
    fn the_rubber_seal_squeezes_as_mooney_rivlin_rubber() {
        let [inner, outer, height] = SEAL;
        let area = std::f64::consts::PI * (outer * outer - inner * inner);
        let lambda: f64 = 1.0 - SEAL_SQUEEZE / height;
        let (c01, c10) = (0.2, 6.0 / (2.0 * 1.49) / 2.0 - 0.2);
        let closed = 2.0 * (lambda - lambda.powi(-2)) * (c10 + c01 / lambda) * area;
        let squeeze = |s: &mut Session| -> f64 {
            s.execute("solve.nonlinear", &json!({})).unwrap();
            let r = s.execute("results.reactions", &json!({"step": "Squeeze", "sets": ["top"]})).unwrap();
            r["sets"][0]["force"][2].as_f64().unwrap()
        };
        let mut s = open("rubber-seal");
        let bonded = squeeze(&mut s);
        let node = |s: &Session, xyz: [f64; 3]| -> u32 {
            *s.model.nodes().iter().find(|(_, n)| (0..3).all(|k| (n.xyz[k] - xyz[k]).abs() < 1e-9)).unwrap().0
        };
        let held = [("hold_y", [inner, 0.0, 0.0], [None, Some(0.0), None]), ("hold_x", [0.0, inner, 0.0], [Some(0.0), None, None]), ("hold_y2", [-inner, 0.0, 0.0], [None, Some(0.0), None])];
        for (name, at, [ux, uy, _]) in held {
            let n = node(&s, at);
            s.execute("model.set.create", &json!({"name": name, "nodes": [n]})).unwrap();
            s.execute("model.boundary_condition.create", &json!({"name": name, "type": "displacement", "target": {"set": name}, "values": [ux, uy, null, null, null, null]})).unwrap();
        }
        s.execute("model.boundary_condition.edit", &json!({"name": "Base", "type": "displacement", "target": {"set": "bottom"}, "values": [null, null, 0, null, null, null]})).unwrap();
        s.execute("model.boundary_condition.edit", &json!({"name": "Press", "type": "displacement", "target": {"set": "top"}, "values": [null, null, -SEAL_SQUEEZE, null, null, null]})).unwrap();
        s.execute("model.step.edit", &json!({"name": "Squeeze", "boundary_conditions": ["Base", "Press", "hold_y", "hold_x", "hold_y2"]})).unwrap();
        let free = squeeze(&mut s);
        assert!((free - closed).abs() < 0.03 * closed.abs(), "frictionless {free} N against {closed} N");
        let stiffer = bonded / free;
        assert!(stiffer > 1.05 && stiffer < 2.5, "bonded {bonded} N, frictionless {free} N");
    }

    #[test]
    fn the_flow_sample_opens_ready_to_solve() {
        let s = open("flow-past");
        assert_eq!(s.model.fluid_domains().len(), 1);
    }

    /// The drag on the bracket, as a coefficient on the upright's frontal area (40 × 50 mm), is
    /// that of a bluff plate standing on a wall at a Reynolds number of 83, raised by the 17 %
    /// blockage of the box: between 1 and 3.5 (a flat plate broadside to the flow is about 1.2 at
    /// high Reynolds number and rises as it falls). The lattice conserves mass.
    #[test]
    #[ignore = "heavy: cargo xtask test --heavy (CI heavy job)"]
    fn the_flow_past_the_bracket_has_a_bluff_body_drag() {
        let mut s = open("flow-past");
        let r = s.execute("solve.run", &json!({})).unwrap();
        let r = &r["steps"][0]["summary"];
        let drag: f64 = r["forces"].as_array().unwrap().iter().filter(|f| f["body"] == "Bracket").map(|f| f["mean"][0].as_f64().unwrap()).sum();
        let [density, _] = AIR;
        let cd = drag / (0.5 * density * FLOW_SPEED * FLOW_SPEED * 40.0 * 50.0);
        assert!(cd > 1.0 && cd < 3.5, "Cd {cd}: {r}");
        assert!(r["mass"]["relative_change"].as_f64().unwrap().abs() < 0.01, "{r}");
    }

    /// The heated bracket's temperatures lie between the air's and the heater's, falling along
    /// the bracket away from the heater. Its thermal expansion: with the air at the heater's
    /// temperature the bracket warms evenly by ΔT, and held at the foot's hole alone (at x = 40)
    /// it grows freely, so a point moves α ΔT times its distance from the hole's axis along x:
    /// the upright's top by −0.0432 mm (within 2 %), the foot's end by +0.024 mm (within 6 %:
    /// the clamped bore cannot grow).
    #[test]
    #[ignore = "heavy: cargo xtask test --heavy (CI heavy job)"]
    fn the_heated_bracket_warms_and_expands_as_steel() {
        let mut s = open("heated-bracket");
        let r = s.execute("solve.run", &json!({})).unwrap();
        let r = &r["steps"][0]["summary"];
        assert!((r["max_temperature"]["value"].as_f64().unwrap() - HEATER).abs() < 1e-9, "{r}");
        let probe = |s: &mut Session| s.execute("solve.probe", &json!({"step": "Heat", "at": [[4.0, 50.0, 20.0], [60.0, 4.0, 20.0]]})).unwrap();
        let p = probe(&mut s);
        let (upright, foot) = (p["nodes"][0]["temperature"].as_f64().unwrap(), p["nodes"][1]["temperature"].as_f64().unwrap());
        assert!(upright > foot && foot > AMBIENT && upright <= HEATER, "{upright} {foot}");
        let foot_hole = json!([{"body": "Bracket", "face": "F4:bore:side:0"}, {"body": "Bracket", "face": "F4:bore:side:1"}]);
        s.execute("model.set.create", &json!({"name": "foot_hole", "geometry": foot_hole})).unwrap();
        s.execute("model.boundary_condition.edit", &json!({"name": "Clamp", "type": "fixed", "target": {"set": "foot_hole"}})).unwrap();
        s.execute("model.load.edit", &json!({"name": "Air", "type": "convection", "set": "skin", "film": 0.01, "sink": HEATER})).unwrap();
        s.execute("solve.static", &json!({})).unwrap();
        let p = probe(&mut s);
        let grow = STEEL_EXPANSION * (HEATER - AMBIENT);
        // The bore itself cannot grow, which holds back the foot's end, 16 mm from its edge, a
        // little more than the upright's top, 32 mm away.
        for (i, tolerance) in [(0, 0.02), (1, 0.06)] {
            let node = &p["nodes"][i];
            let (ux, expected) = (node["displacement"][0].as_f64().unwrap(), grow * (node["xyz"][0].as_f64().unwrap() - 40.0));
            assert!((ux - expected).abs() < tolerance * expected.abs(), "{node}: {ux} against {expected}");
        }
    }

    #[test]
    fn the_drop_test_opens_ready_to_solve() {
        let s = open("drop-test");
        assert!(s.model.elements().len() > 1000);
    }

    /// The bracket's mass is ρ V (V: foot, upright and the fillet's corner, 36 − 9π mm² by 40 mm),
    /// it meets the floor at √(2 g h) with ½ m v² of kinetic energy and conserves energy; by the
    /// end the floor has taken half its downward momentum and turned kinetic energy into
    /// vibration.
    #[test]
    #[ignore = "heavy: cargo xtask test --heavy (CI heavy job)"]
    fn the_drop_test_lands_at_the_free_fall_speed_and_bounces() {
        let mut s = open("drop-test");
        let r = s.execute("solve.run", &json!({})).unwrap();
        let r = &r["steps"][0]["summary"];
        let volume = 60.0 * 8.0 * 40.0 + 8.0 * 42.0 * 40.0 + (36.0 - 9.0 * std::f64::consts::PI) * 40.0;
        let mass = 2.7e-9 * volume;
        let close = |a: f64, b: f64, rel: f64| (a - b).abs() <= rel * b.abs();
        assert!(close(r["total_mass"].as_f64().unwrap(), mass, 0.005), "{r}");
        assert!(r["completed"].as_bool() == Some(true) && r["energy_balance"].as_f64().unwrap() < 0.01, "{r}");
        let h = s.execute("solve.history", &json!({"step": "Drop"})).unwrap();
        let speed = (2.0 * GRAVITY * DROP_HEIGHT).sqrt();
        let pz = |i: usize| h["momentum"][i][2].as_f64().unwrap();
        let kinetic = |i: usize| h["energies"][i]["kinetic"].as_f64().unwrap();
        assert!(close(pz(0), -mass * speed, 0.005), "{}", pz(0));
        assert!(close(kinetic(0), 0.5 * mass * speed * speed, 0.01), "{}", kinetic(0));
        let last = h["time"].as_array().unwrap().len() - 1;
        let kept = pz(last) / pz(0);
        assert!(kept > 0.4 && kept < 0.6 && kinetic(last) < kinetic(0), "{} {}", pz(last), kinetic(last));
    }
}
