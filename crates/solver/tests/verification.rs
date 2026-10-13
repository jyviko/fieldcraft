//! The analytic verification suite: every element and formulation against closed-form
//! solutions. Run with `--nocapture` to see the errors.

mod common;

use std::f64::consts::PI;

use common::*;
use modelcraft_model::{BoundaryCondition, ElementKind, Load, Procedure, Symmetry, Target};
use modelcraft_solver::{FrequencyOptions, Limits, MassKind, StaticResult, solve_frequency, solve_static};

fn static_solve(g: &Grid) -> StaticResult {
    solve_static(&g.model, "Step", &Limits::default()).unwrap()
}

fn at(r: &StaticResult, label: u32) -> usize {
    r.nodes.binary_search(&label).unwrap()
}

/// Displacement patch test (MacNeal & Harder 1985 style, on a distorted 2 × 2 × 2 patch): a
/// linear displacement prescribed on the boundary is reproduced exactly inside, with exactly
/// constant strain and stress in every element.
#[test]
fn patch_test_every_element() {
    let a = [[1e-3, 2e-4, -3e-4], [5e-4, -2e-3, 1e-4], [-1e-4, 3e-4, 7e-4]];
    let (young, nu) = (1000.0, 0.25);
    let lambda = young * nu / ((1.0 + nu) * (1.0 - 2.0 * nu));
    let mu = young / (2.0 * (1.0 + nu));
    let strain = [a[0][0], a[1][1], a[2][2], a[0][1] + a[1][0], a[0][2] + a[2][0], a[1][2] + a[2][1]];
    let tr = strain[0] + strain[1] + strain[2];
    let stress = [
        lambda * tr + 2.0 * mu * strain[0],
        lambda * tr + 2.0 * mu * strain[1],
        lambda * tr + 2.0 * mu * strain[2],
        mu * strain[3],
        mu * strain[4],
        mu * strain[5],
    ];
    let field = |x: [f64; 3]| [0, 1, 2].map(|i| (0..3).map(|j| a[i][j] * x[j]).sum::<f64>());
    // Interior distortion that leaves the cube's boundary in place.
    let map = |p: [f64; 3]| {
        let b = (PI * p[0]).sin() * (PI * p[1]).sin() * (PI * p[2]).sin();
        [p[0] + 0.12 * b, p[1] - 0.08 * b, p[2] + 0.1 * b]
    };
    for (kind, formulation, name) in variants() {
        let mut g = grid([2, 2, 2], kind, map);
        g.define(iso(young, nu), None, formulation, Procedure::Static {});
        let on_boundary = |p: [f64; 3]| p.iter().any(|c| near(*c, 0.0) || near(*c, 1.0));
        for (i, label) in g.nodes_where(on_boundary).into_iter().enumerate() {
            let x = g.model.node(label).unwrap().xyz;
            let u = field(x);
            let set = format!("b{i}");
            g.model.add_set(&set, modelcraft_model::Set { nodes: vec![label], ..Default::default() }).unwrap();
            g.bc(
                &set,
                BoundaryCondition::Displacement { target: Target::Set(set.clone()), values: [Some(u[0]), Some(u[1]), Some(u[2]), None, None, None] },
            );
        }
        let r = static_solve(&g);
        let mut worst: f64 = 0.0;
        for (label, u) in r.nodes.iter().zip(&r.displacement) {
            let want = field(g.model.node(*label).unwrap().xyz);
            worst = worst.max((0..3).map(|k| (u[k] - want[k]).abs()).fold(0.0, f64::max));
        }
        let mut worst_stress: f64 = 0.0;
        for e in &r.elements {
            for p in &e.points {
                worst_stress = worst_stress.max((0..6).map(|k| (p.stress[k] - stress[k]).abs()).fold(0.0, f64::max));
            }
        }
        for s in &r.stress {
            worst_stress = worst_stress.max((0..6).map(|k| (s[k] - stress[k]).abs()).fold(0.0, f64::max));
        }
        eprintln!("patch {name}: displacement error {worst:.1e}, stress error {worst_stress:.1e}");
        assert!(worst < 1e-12, "{name}: {worst}");
        assert!(worst_stress < 1e-10, "{name}: {worst_stress}");
    }
}

/// Uniaxial tension by a consistent surface load on a distorted block with symmetry planes:
/// σxx = p exactly, ux = p L / E, the lateral contraction −ν p y / E, and the reactions balance
/// the load (the force form of the patch test, and a check of pressure loads).
#[test]
fn uniaxial_tension_by_pressure() {
    let (young, nu, p) = (2000.0, 0.3, 5.0);
    let (lx, ly, lz) = (3.0, 2.0, 1.5);
    for (kind, formulation, name) in variants() {
        let mut g = grid([2, 2, 2], kind, |q| {
            let b = (PI * q[0]).sin() * (PI * q[1]).sin() * (PI * q[2]).sin();
            [lx * q[0] + 0.2 * b, ly * q[1] + 0.1 * b, lz * q[2] - 0.1 * b]
        });
        g.define(iso(young, nu), None, formulation, Procedure::Static {});
        g.symmetry("x0", Symmetry::X, |q| near(q[0], 0.0));
        g.symmetry("y0", Symmetry::Y, |q| near(q[1], 0.0));
        g.symmetry("z0", Symmetry::Z, |q| near(q[2], 0.0));
        g.face_set("end", |q| near(q[0], 1.0));
        // A negative pressure pulls.
        g.load("pull", Load::Pressure { set: "end".into(), magnitude: -p });
        let r = static_solve(&g);
        let mut worst: f64 = 0.0;
        for (label, u) in r.nodes.iter().zip(&r.displacement) {
            let x = g.model.node(*label).unwrap().xyz;
            let want = [p * x[0] / young, -nu * p * x[1] / young, -nu * p * x[2] / young];
            worst = worst.max((0..3).map(|k| (u[k] - want[k]).abs()).fold(0.0, f64::max));
        }
        let s_err = r.stress.iter().map(|s| (s[0] - p).abs().max(s[1].abs()).max(s[5].abs())).fold(0.0, f64::max);
        eprintln!("tension {name}: displacement error {worst:.1e}, stress error {s_err:.1e}");
        assert!(worst < 1e-11, "{name}: {worst}");
        assert!(s_err < 1e-9, "{name}: {s_err}");
        assert!((r.applied_total[0] - p * ly * lz).abs() < 1e-9 && (r.reaction_total[0] + p * ly * lz).abs() < 1e-9, "{name}");
        assert!(r.von_mises.iter().all(|v| (v - p).abs() < 1e-9));
    }
}

/// Gravity is ρ g V, held by the reactions.
#[test]
fn gravity_and_body_force_totals() {
    let mut g = grid([3, 2, 2], ElementKind::Hex20, |q| [4.0 * q[0], 2.0 * q[1], q[2]]);
    g.define(iso(1000.0, 0.3), Some(2.5), full(), Procedure::Static {});
    g.fixed("base", |q| near(q[2], 0.0));
    g.load("g", Load::Gravity { acceleration: [0.0, 0.0, -9.81], set: None });
    g.load("b", Load::BodyForce { set: "all".into(), vector: [1.0, 0.0, 0.0] });
    let r = static_solve(&g);
    let v = 8.0;
    assert!((r.applied_total[2] + 2.5 * 9.81 * v).abs() < 1e-9, "{:?}", r.applied_total);
    assert!((r.applied_total[0] - v).abs() < 1e-9);
    for k in 0..3 {
        assert!((r.applied_total[k] + r.reaction_total[k]).abs() < 1e-8, "{k}");
    }
}

/// Cantilever L × b × h, clamped at x = 0, a shear traction on the free end. Timoshenko
/// deflection δ = P L³ / (3 E I) + P L / (κ G A), κ = 5/6, with ν = 0 so the clamp adds no
/// lateral restraint.
fn cantilever_tip(kind: ElementKind, formulation: modelcraft_model::Formulation, n: [usize; 3]) -> f64 {
    let (l, b, h, young, p) = (10.0, 1.0, 1.0, 1000.0, 1.0);
    let mut g = grid(n, kind, |q| [l * q[0], b * q[1], h * q[2]]);
    g.define(iso(young, 0.0), None, formulation, Procedure::Static {});
    g.fixed("root", |q| near(q[0], 0.0));
    g.face_set("end", |q| near(q[0], 1.0));
    g.load("shear", Load::Traction { set: "end".into(), vector: [0.0, 0.0, -p / (b * h)] });
    let r = static_solve(&g);
    let end = g.nodes_where(|q| near(q[0], 1.0));
    -end.iter().map(|l| r.displacement[at(&r, *l)][2]).sum::<f64>() / end.len() as f64
}

fn timoshenko() -> f64 {
    let (l, b, h, young, p) = (10.0f64, 1.0f64, 1.0f64, 1000.0, 1.0);
    let i = b * h.powi(3) / 12.0;
    p * l.powi(3) / (3.0 * young * i) + p * l / (5.0 / 6.0 * young / 2.0 * b * h)
}

#[test]
fn cantilever_tip_deflection_and_convergence() {
    let exact = timoshenko();
    let euler_bernoulli = 4.0;
    eprintln!("cantilever: Timoshenko {exact:.5}, Euler–Bernoulli {euler_bernoulli:.5}");
    let err = |d: f64| (d - exact).abs() / exact;
    let study = |kind, f, meshes: &[[usize; 3]], name: &str| -> Vec<f64> {
        meshes
            .iter()
            .map(|n| {
                let d = cantilever_tip(kind, f, *n);
                eprintln!("  {name} {n:?}: {d:.5} (error {:.2}%)", 100.0 * err(d));
                err(d)
            })
            .collect()
    };
    // Quadratic and enhanced elements are accurate on coarse meshes.
    let hex20 = study(ElementKind::Hex20, full(), &[[10, 1, 1], [20, 2, 2]], "hex20");
    assert!(hex20[1] < 0.01);
    let hex20r = study(ElementKind::Hex20, reduced(), &[[10, 2, 2]], "hex20 reduced");
    assert!(hex20r[0] < 0.01);
    let im = study(ElementKind::Hex8, incompatible(), &[[10, 1, 1], [20, 2, 2]], "hex8 incompatible");
    assert!(im[0] < 0.03 && im[1] < 0.015);
    let tet10 = study(ElementKind::Tet10, full(), &[[10, 2, 2]], "tet10");
    assert!(tet10[0] < 0.02);
    // Linear elements lock in bending and converge under refinement.
    let meshes = [[5, 1, 1], [10, 2, 2], [20, 4, 4], [40, 8, 8]];
    let hex8 = study(ElementKind::Hex8, full(), &meshes, "hex8");
    let tet4 = study(ElementKind::Tet4, full(), &meshes, "tet4");
    for (name, e) in [("hex8", &hex8), ("tet4", &tet4)] {
        let rates: Vec<f64> = e.windows(2).map(|w| (w[0] / w[1]).log2()).collect();
        eprintln!("  {name} convergence rates {rates:.2?}");
        assert!(e.windows(2).all(|w| w[1] < w[0]), "{name} converges monotonically");
        assert!(*rates.last().unwrap() > 1.5, "{name} approaches second order: {rates:?}");
    }
    assert!(hex8[3] < 0.04 && tet4[3] < 0.12, "{hex8:?} {tet4:?}");
}

/// Lamé's thick cylinder under internal pressure, plane strain (u_z = 0 on both ends), quarter
/// model: σ_r = A − B/r², σ_θ = A + B/r², A = p a²/(b² − a²), B = p a² b²/(b² − a²),
/// u_r = (1 + ν)/E ((1 − 2ν) A r + B / r).
#[test]
fn lame_thick_cylinder() {
    let (a, b, t, p, young, nu) = (1.0f64, 2.0f64, 0.25, 10.0, 1000.0, 0.3);
    let big_a = p * a * a / (b * b - a * a);
    let big_b = p * a * a * b * b / (b * b - a * a);
    let ur = |r: f64| (1.0 + nu) / young * ((1.0 - 2.0 * nu) * big_a * r + big_b / r);
    let sigma_t = |r: f64| big_a + big_b / (r * r);
    for (kind, formulation, n, tol_u, tol_s, name) in [
        (ElementKind::Hex20, full(), [6, 8, 1], 0.002, 0.01, "hex20"),
        (ElementKind::Hex8, incompatible(), [12, 16, 1], 0.01, 0.04, "hex8 incompatible"),
        (ElementKind::Tet10, full(), [6, 8, 1], 0.005, 0.03, "tet10"),
    ] {
        let mut g = grid(n, kind, |q| {
            let (r, th) = (a + (b - a) * q[0], PI / 2.0 * q[1]);
            [r * th.cos(), r * th.sin(), t * q[2]]
        });
        g.define(iso(young, nu), None, formulation, Procedure::Static {});
        g.symmetry("y0", Symmetry::Y, |q| near(q[1], 0.0));
        g.symmetry("x0", Symmetry::X, |q| near(q[1], 1.0));
        g.node_set("ends", |q| near(q[2], 0.0) || near(q[2], 1.0));
        g.bc("ends", BoundaryCondition::Symmetry { target: Target::Set("ends".into()), plane: Symmetry::Z });
        g.face_set("bore", |q| near(q[0], 0.0));
        g.load("p", Load::Pressure { set: "bore".into(), magnitude: p });
        let r = static_solve(&g);
        let mut eu: f64 = 0.0;
        for (label, u) in r.nodes.iter().zip(&r.displacement) {
            let x = g.model.node(*label).unwrap().xyz;
            let rad = x[0].hypot(x[1]);
            let got = (u[0] * x[0] + u[1] * x[1]) / rad;
            eu = eu.max((got - ur(rad)).abs() / ur(a));
        }
        // Hoop stress at the bore (nodal average), on the 45° line to stay away from the
        // symmetry edges.
        let bore: Vec<u32> = g.nodes_where(|q| near(q[0], 0.0) && near(q[1], 0.5));
        let st: f64 = bore
            .iter()
            .map(|l| {
                let s = r.stress[at(&r, *l)];
                let x = g.model.node(*l).unwrap().xyz;
                let (c, sn) = (x[0] / x[0].hypot(x[1]), x[1] / x[0].hypot(x[1]));
                // σ_θ = s_xx sin² − 2 s_xy sin cos + s_yy cos².
                s[0] * sn * sn - 2.0 * s[3] * sn * c + s[1] * c * c
            })
            .sum::<f64>()
            / bore.len() as f64;
        let es = (st - sigma_t(a)).abs() / sigma_t(a);
        eprintln!("lamé {name} {n:?}: u_r error {:.3}%, bore hoop stress {st:.4} vs {:.4} ({:.2}%)", 100.0 * eu, sigma_t(a), 100.0 * es);
        assert!(eu < tol_u && es < tol_s, "{name}");
    }
}

/// Kirsch: a plate with a hole of radius a under remote tension σ₀, plane stress, quarter model
/// with the plate's half-width 20a. The hoop stress at the top of the hole tends to 3σ₀ (the
/// finite width adds well under 1%).
#[test]
fn kirsch_plate_with_hole() {
    let (a, w, s0) = (1.0f64, 20.0f64, 1.0);
    let ratio: f64 = 40.0;
    let map = |q: [f64; 3]| {
        let th = PI / 2.0 * q[1];
        let outer = if th <= PI / 4.0 { [w, w * th.tan()] } else { [w / th.tan(), w] };
        // Geometric grading toward the hole.
        let s = (ratio.powf(q[0]) - 1.0) / (ratio - 1.0);
        let inner = [a * th.cos(), a * th.sin()];
        [inner[0] + s * (outer[0] - inner[0]), inner[1] + s * (outer[1] - inner[1]), 0.1 * q[2]]
    };
    let mut kt = Vec::new();
    for (kind, formulation, n, name) in [
        (ElementKind::Hex20, full(), [6, 8, 1], "hex20 coarse"),
        (ElementKind::Hex20, full(), [12, 16, 1], "hex20 fine"),
        (ElementKind::Hex8, incompatible(), [24, 32, 1], "hex8 incompatible"),
    ] {
        let mut g = grid(n, kind, map);
        g.define(iso(1000.0, 0.3), None, formulation, Procedure::Static {});
        g.symmetry("x0", Symmetry::X, |q| near(q[1], 1.0));
        g.symmetry("y0", Symmetry::Y, |q| near(q[1], 0.0));
        g.symmetry("z0", Symmetry::Z, |q| near(q[2], 0.0));
        g.face_set("right", |q| near(q[0], 1.0) && q[1] <= 0.5 + 1e-9);
        g.load("pull", Load::Traction { set: "right".into(), vector: [s0, 0.0, 0.0] });
        let r = static_solve(&g);
        let top: Vec<u32> = g.nodes_where(|q| near(q[0], 0.0) && near(q[1], 1.0));
        let k = top.iter().map(|l| r.stress[at(&r, *l)][0]).sum::<f64>() / top.len() as f64 / s0;
        eprintln!("kirsch {name} {n:?}: Kt = {k:.4} (error vs 3: {:.2}%)", 100.0 * (k - 3.0).abs() / 3.0);
        kt.push(k);
    }
    assert!((kt[1] - 3.0).abs() < (kt[0] - 3.0).abs(), "refinement approaches 3: {kt:?}");
    assert!((kt[1] - 3.0).abs() / 3.0 < 0.02, "{kt:?}");
    assert!((kt[2] - 3.0).abs() / 3.0 < 0.04, "{kt:?}");
}

/// Euler–Bernoulli frequencies f = (βL)² / (2π L²) √(EI/(ρA)).
fn beam_frequency(beta_l: f64, l: f64, young: f64, rho: f64, side: f64) -> f64 {
    let i = side.powi(4) / 12.0;
    let area = side * side;
    beta_l * beta_l / (2.0 * PI * l * l) * (young * i / (rho * area)).sqrt()
}

fn beam(kind: ElementKind, n: [usize; 3], clamped: bool, modes: u32) -> Grid {
    let l = 20.0;
    let mut g = grid(n, kind, |q| [l * q[0], q[1], q[2]]);
    g.define(iso(1000.0, 0.0), Some(1.0), full(), Procedure::Frequency { modes, min_hz: None, max_hz: None });
    if clamped {
        g.fixed("root", |q| near(q[0], 0.0));
    }
    g
}

#[test]
fn cantilever_natural_frequencies() {
    let f1 = beam_frequency(1.875_104_07, 20.0, 1000.0, 1.0, 1.0);
    let f2 = beam_frequency(4.694_091_13, 20.0, 1000.0, 1.0, 1.0);
    for (kind, n, mass, name) in [
        (ElementKind::Hex20, [20, 1, 1], MassKind::Consistent, "hex20"),
        (ElementKind::Hex20, [20, 1, 1], MassKind::Lumped, "hex20 lumped"),
        (ElementKind::Tet10, [20, 2, 2], MassKind::Consistent, "tet10"),
    ] {
        let g = beam(kind, n, true, 4);
        let r = solve_frequency(&g.model, "Step", &FrequencyOptions { mass, shift: None }, &Limits::default()).unwrap();
        let f: Vec<f64> = r.modes.iter().map(|m| m.frequency).collect();
        eprintln!("cantilever modes {name}: {f:.6?} vs EB {f1:.6}, {f2:.6} ({} Lanczos vectors)", r.stats.lanczos_vectors);
        assert_eq!(f.len(), 4);
        // The square section bends alike in y and z: two pairs.
        for (got, want, tol) in [(f[0], f1, 0.01), (f[1], f1, 0.01), (f[2], f2, 0.03), (f[3], f2, 0.03)] {
            assert!((got - want).abs() / want < tol, "{name}: {got} vs {want}");
        }
        assert!((r.total_mass - 20.0).abs() < 1e-9);
    }
}

#[test]
fn free_free_beam_has_six_rigid_body_modes() {
    let g = beam(ElementKind::Hex20, [20, 1, 1], false, 8);
    let r = solve_frequency(&g.model, "Step", &FrequencyOptions::default(), &Limits::default()).unwrap();
    let f: Vec<f64> = r.modes.iter().map(|m| m.frequency).collect();
    let f1 = beam_frequency(4.730_040_74, 20.0, 1000.0, 1.0, 1.0);
    eprintln!("free-free modes: {f:.6?} vs EB {f1:.6}");
    assert_eq!(f.len(), 8);
    assert!(f[..6].iter().all(|x| *x < 1e-3 * f1), "{f:?}");
    assert!(f[6..].iter().all(|x| (x - f1).abs() / f1 < 0.015), "{f:?}");
    // The rigid translations carry the whole mass as effective mass.
    for d in 0..3 {
        let m: f64 = r.modes[..6].iter().map(|m| m.effective_mass[d]).sum();
        assert!((m - 20.0).abs() < 1e-6 * 20.0, "direction {d}: {m}");
    }
    assert!(r.warnings.iter().any(|w| w.contains("rigid-body")));
}

#[test]
fn results_are_deterministic() {
    let g = beam(ElementKind::Tet10, [6, 1, 1], true, 3);
    let a = solve_frequency(&g.model, "Step", &FrequencyOptions::default(), &Limits::default()).unwrap();
    let b = solve_frequency(&g.model, "Step", &FrequencyOptions::default(), &Limits::default()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn errors_name_the_problem() {
    // No boundary condition in a static step: singular, naming a node.
    let mut g = grid([2, 1, 1], ElementKind::Hex8, |q| q);
    g.define(iso(1000.0, 0.3), None, full(), Procedure::Static {});
    g.face_set("end", |q| near(q[0], 1.0));
    g.load("p", Load::Pressure { set: "end".into(), magnitude: 1.0 });
    let e = solve_static(&g.model, "Step", &Limits::default()).unwrap_err().to_string();
    assert!(e.contains("singular at node"), "{e}");
    // Caps.
    g.fixed("root", |q| near(q[0], 0.0));
    let tight = Limits { max_dofs: 10, ..Limits::default() };
    assert!(solve_static(&g.model, "Step", &tight).unwrap_err().to_string().contains("limit"));
    let tight = Limits { max_factor_entries: 5, ..Limits::default() };
    assert!(solve_static(&g.model, "Step", &tight).unwrap_err().to_string().contains("limit"));
    assert!(solve_static(&g.model, "Nope", &Limits::default()).is_err());
    assert!(solve_frequency(&g.model, "Step", &FrequencyOptions::default(), &Limits::default()).is_err(), "a static step");
    assert!(solve_static(&g.model, "Step", &Limits::default()).is_ok());
}
