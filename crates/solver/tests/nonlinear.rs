//! Nonlinear static analysis against closed forms: uniaxial stretch of neo-Hookean and
//! St Venant–Kirchhoff blocks (3D, plane strain, axisymmetric), the nonlinear patch test, a
//! large rigid rotation, Newton's quadratic convergence, the linear limit, and what nonlinear
//! steps refuse or stop at. Run with `--nocapture` to see the residuals.

mod common;

use common::*;
use fieldcraft_model::*;
use fieldcraft_solver::{Limits, StaticResult, solve_nonlinear, solve_static};

fn nonlinear() -> Procedure {
    Procedure::NonlinearStatic(NonlinearStatic::default())
}

fn neo_hookean(g: &mut Grid, e: f64, nu: f64) {
    let m = Material { elastic: Some(iso(e, nu)), hyperelastic: Some(Hyperelastic::NeoHookean {}), ..Material::default() };
    g.model.replace_material("Mat", m).unwrap();
}

/// Prescribe every displacement component of each node where `f` holds from its position:
/// one set and one boundary condition per node.
fn prescribe(g: &mut Grid, f: impl Fn([f64; 3]) -> bool, u: impl Fn([f64; 3]) -> [f64; 3]) {
    for l in g.nodes_where(f) {
        let x = g.model.node(l).unwrap().xyz;
        let v = u(x);
        let name = format!("n{l}");
        g.model.add_set(&name, Set { nodes: vec![l], ..Set::default() }).unwrap();
        let values = [Some(v[0]), Some(v[1]), Some(v[2]), None, None, None];
        g.bc(&name, BoundaryCondition::Displacement { target: Target::Set(name.clone()), values });
    }
}

fn at(r: &StaticResult, label: u32) -> usize {
    r.nodes.binary_search(&label).unwrap()
}

fn solve(g: &Grid) -> StaticResult {
    solve_nonlinear(&g.model, Some("Step"), &Limits::default()).unwrap()
}

/// The lateral stretch of a neo-Hookean bar stretched by λ in uniaxial stress (σ_yy = σ_zz = 0)
/// and its axial Cauchy stress, by bisection on σ_yy(l) = μ J^(−5/3) (l² − I₁/3) + κ (J − 1).
fn neo_hookean_uniaxial(mu: f64, kappa: f64, lam: f64) -> (f64, f64) {
    let stress = |l: f64| {
        let (j, i1) = (lam * l * l, lam * lam + 2.0 * l * l);
        let c = mu * j.powf(-5.0 / 3.0);
        (c * (lam * lam - i1 / 3.0) + kappa * (j - 1.0), c * (l * l - i1 / 3.0) + kappa * (j - 1.0))
    };
    let (mut lo, mut hi) = (0.1, 1.5);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if stress(mid).1 > 0.0 { hi = mid } else { lo = mid }
    }
    let l = 0.5 * (lo + hi);
    (l, stress(l).0)
}

/// A unit block stretched to twice its length (x), free laterally (symmetry planes), in each
/// element kind: the homogeneous closed form at every node, and the logarithmic strain ln 2.
#[test]
fn neo_hookean_block_in_uniaxial_stretch() {
    let (e, nu, lam) = (10.0, 0.3, 2.0);
    let (mu, kappa) = (e / (2.0 * (1.0 + nu)), e / (3.0 * (1.0 - 2.0 * nu)));
    let (l, sigma) = neo_hookean_uniaxial(mu, kappa, lam);
    for (kind, formulation, name) in variants().into_iter().filter(|v| v.1 != incompatible()) {
        let mut g = grid([2, 2, 2], kind, |x| x);
        g.define(iso(e, nu), None, formulation, nonlinear());
        neo_hookean(&mut g, e, nu);
        g.symmetry("x0", Symmetry::X, |x| near(x[0], 0.0));
        g.symmetry("y0", Symmetry::Y, |x| near(x[1], 0.0));
        g.symmetry("z0", Symmetry::Z, |x| near(x[2], 0.0));
        g.node_set("x1", |x| near(x[0], 1.0));
        g.bc("pull", BoundaryCondition::Displacement { target: Target::Set("x1".into()), values: [Some(lam - 1.0), None, None, None, None, None] });
        let r = solve(&g);
        for (i, label) in r.nodes.iter().enumerate() {
            let x = g.model.node(*label).unwrap().xyz;
            let want = [(lam - 1.0) * x[0], (l - 1.0) * x[1], (l - 1.0) * x[2]];
            for k in 0..3 {
                assert!((r.displacement[i][k] - want[k]).abs() < 1e-8, "{name} node {label}: {:?} vs {want:?}", r.displacement[i]);
            }
        }
        for s in &r.stress {
            assert!((s[0] - sigma).abs() < 1e-8 * sigma && s[1].abs() < 1e-8 * sigma && s[3].abs() < 1e-8 * sigma, "{name}: {s:?} vs {sigma}");
        }
        let strain = r.elements[0].points[0].strain;
        assert!((strain[0] - lam.ln()).abs() < 1e-9 && (strain[1] - l.ln()).abs() < 1e-9, "{name}: {strain:?}");
        // The reactions on the pulled face balance the axial force σ times the deformed area l².
        let pulled: f64 = g.nodes_where(|x| near(x[0], 1.0)).iter().map(|n| r.reaction[at(&r, *n)][0]).sum();
        assert!((pulled - sigma * l * l).abs() < 1e-8 * sigma, "{name}: {pulled} vs {}", sigma * l * l);
        let h = r.nonlinear.as_ref().unwrap();
        assert!(h.increments.last().is_some_and(|i| i.converged && i.load_factor == 1.0), "{name}: {:?}", h.increments);
        assert_eq!(h.frames.first().map(|f| f.load_factor), Some(0.0));
        assert_eq!(h.frames.last().map(|f| f.load_factor), Some(1.0));
        eprintln!("{name}: {} increments, {} iterations", h.increments.len(), h.iterations);
    }
}

/// St Venant–Kirchhoff in uniaxial stress: S₁₁ = E E₁₁ with E₁₁ = (λ² − 1)/2, lateral
/// E₂₂ = −ν E₁₁; the Cauchy stress σ = λ² S₁₁ / J.
#[test]
fn st_venant_kirchhoff_uniaxial_stretch() {
    let (e, nu, lam): (f64, f64, f64) = (1000.0, 0.25, 1.3);
    let e11 = 0.5 * (lam * lam - 1.0);
    let l = (1.0 - 2.0 * nu * e11).sqrt();
    let sigma = lam * lam * e * e11 / (lam * l * l);
    let mut g = grid([1, 1, 1], ElementKind::Hex20, |x| x);
    g.define(iso(e, nu), None, full(), nonlinear());
    g.symmetry("x0", Symmetry::X, |x| near(x[0], 0.0));
    g.symmetry("y0", Symmetry::Y, |x| near(x[1], 0.0));
    g.symmetry("z0", Symmetry::Z, |x| near(x[2], 0.0));
    g.node_set("x1", |x| near(x[0], 1.0));
    g.bc("pull", BoundaryCondition::Displacement { target: Target::Set("x1".into()), values: [Some(lam - 1.0), None, None, None, None, None] });
    let r = solve(&g);
    let corner = g.nodes_where(|x| near(x[0], 1.0) && near(x[1], 1.0) && near(x[2], 1.0))[0];
    assert!((r.displacement[at(&r, corner)][1] - (l - 1.0)).abs() < 1e-9, "{:?}", r.displacement[at(&r, corner)]);
    assert!(r.stress.iter().all(|s| (s[0] - sigma).abs() < 1e-8 * sigma), "{:?} vs {sigma}", r.stress[0]);
}

/// The nonlinear patch test: a distorted mesh whose boundary nodes follow a homogeneous finite
/// deformation x = F X (stretch, shear and rotation) reproduces it at the inner nodes and gives
/// the uniform stress of F, for every element kind.
#[test]
fn nonlinear_patch_test() {
    let f = [[1.3, 0.2, -0.1], [-0.15, 0.9, 0.25], [0.05, -0.2, 1.1]];
    let affine = |x: [f64; 3]| -> [f64; 3] { std::array::from_fn(|i| (0..3).map(|j| f[i][j] * x[j]).sum::<f64>() - x[i]) };
    let distort = |q: [f64; 3]| {
        let bump = (std::f64::consts::PI * q[0]).sin() * (std::f64::consts::PI * q[1]).sin() * (std::f64::consts::PI * q[2]).sin();
        [q[0] + 0.12 * bump, q[1] - 0.08 * bump, q[2] + 0.1 * bump]
    };
    let (e, nu) = (100.0, 0.3);
    let neo = fieldcraft_fem::constitutive::MaterialModel::from_model(&iso(e, nu), Some(&Hyperelastic::NeoHookean {})).unwrap();
    let tau = neo.response(&f, &[], &mut []).unwrap().stress;
    let j = fieldcraft_fem::dense::det3(&f);
    let hybrid = Formulation { hybrid: true, ..Formulation::default() };
    let mut cases: Vec<(ElementKind, Formulation, &str)> = variants().into_iter().filter(|v| v.1 != incompatible()).collect();
    cases.extend([
        (ElementKind::Hex8, hybrid, "hex8 hybrid"),
        (ElementKind::Hex20, hybrid, "hex20 hybrid"),
        (ElementKind::Tet10, hybrid, "tet10 hybrid"),
    ]);
    for (kind, formulation, name) in cases {
        let mut g = grid([2, 2, 2], kind, distort);
        g.define(iso(e, nu), None, formulation, nonlinear());
        neo_hookean(&mut g, e, nu);
        let boundary = |q: [f64; 3]| q.iter().any(|c| near(*c, 0.0) || near(*c, 1.0));
        prescribe(&mut g, boundary, affine);
        let r = solve(&g);
        for (i, label) in r.nodes.iter().enumerate() {
            let want = affine(g.model.node(*label).unwrap().xyz);
            assert!((0..3).all(|k| (r.displacement[i][k] - want[k]).abs() < 1e-9), "{name} node {label}: {:?} vs {want:?}", r.displacement[i]);
        }
        for el in &r.elements {
            for p in &el.points {
                assert!((0..6).all(|k| (p.stress[k] - tau[k] / j).abs() < 1e-8 * e), "{name}: {:?} vs {:?}", p.stress, tau.map(|t| t / j));
            }
        }
    }
}

/// A block turned rigidly by 90° about an inclined axis, its boundary prescribed: no strain, no
/// stress, no reaction, whatever the path (objectivity of the formulation).
#[test]
fn rigid_rotation_strains_and_stresses_nothing() {
    let axis = [1.0 / 3f64.sqrt(); 3];
    let k = fieldcraft_fem::rotation::skew(&axis);
    let k2 = fieldcraft_fem::rotation::mul(&k, &k);
    let rot: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 } + k[i][j] + k2[i][j]));
    for (kind, formulation, name) in
        [(ElementKind::Hex20, full(), "hex20"), (ElementKind::Tet10, full(), "tet10"), (ElementKind::Hex8, full(), "hex8")]
    {
        let mut g = grid([2, 2, 2], kind, |x| x);
        g.define(iso(1000.0, 0.3), None, formulation, nonlinear());
        neo_hookean(&mut g, 1000.0, 0.3);
        let boundary = |q: [f64; 3]| q.iter().any(|c| near(*c, 0.0) || near(*c, 1.0));
        prescribe(&mut g, boundary, |x| std::array::from_fn(|i| (0..3).map(|j| rot[i][j] * x[j]).sum::<f64>() - x[i]));
        let r = solve(&g);
        let worst = r.elements.iter().flat_map(|e| &e.points).flat_map(|p| p.stress.iter().chain(&p.strain)).fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(worst < 1e-10 * 1000.0, "{name}: largest stress or strain {worst:e}");
        assert!(r.reaction.iter().flatten().all(|v| v.abs() < 1e-9), "{name}");
    }
}

/// Plane strain and axisymmetric 2D elements reproduce the closed forms: a plane strain block
/// stretched in x (σ_yy = 0, λ_z = 1) and a solid cylinder stretched along its axis (the 3D
/// uniaxial case).
#[test]
fn plane_strain_and_axisymmetric_stretch() {
    let (e, nu, lam) = (10.0, 0.3, 1.6);
    let (mu, kappa) = (e / (2.0 * (1.0 + nu)), e / (3.0 * (1.0 - 2.0 * nu)));
    // Plane strain: σ_yy(l) = μ J^(−5/3) (l² − I₁/3) + κ (J − 1) = 0 with J = λ l, I₁ = λ² + l² + 1.
    let syy = |l: f64| {
        let (j, i1) = (lam * l, lam * lam + l * l + 1.0);
        mu * j.powf(-5.0 / 3.0) * (l * l - i1 / 3.0) + kappa * (j - 1.0)
    };
    let (mut lo, mut hi) = (0.1, 1.5);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if syy(mid) > 0.0 { hi = mid } else { lo = mid }
    }
    let l_plane = 0.5 * (lo + hi);
    let (l_axi, sigma_axi) = neo_hookean_uniaxial(mu, kappa, lam);
    for (section, kind) in [
        (SectionKind::PlaneStrain { thickness: 1.0 }, ElementKind::Quad8),
        (SectionKind::PlaneStrain { thickness: 1.0 }, ElementKind::Tri3),
        (SectionKind::Axisymmetric {}, ElementKind::Quad8),
        (SectionKind::Axisymmetric {}, ElementKind::Quad4),
    ] {
        let mut m = Model::default();
        let n = 3usize;
        let label = |i: usize, j: usize| (1 + i + (2 * n + 1) * j) as u32;
        for j in 0..=2 * n {
            for i in 0..=2 * n {
                m.set_node(label(i, j), [i as f64 / (2 * n) as f64, j as f64 / (2 * n) as f64, 0.0]).unwrap();
            }
        }
        let mut next = 1;
        for cj in 0..n {
            for ci in 0..n {
                let (i, j) = (2 * ci, 2 * cj);
                let c = [label(i, j), label(i + 2, j), label(i + 2, j + 2), label(i, j + 2)];
                match kind {
                    ElementKind::Quad8 => {
                        let mid = [label(i + 1, j), label(i + 2, j + 1), label(i + 1, j + 2), label(i, j + 1)];
                        m.set_element(next, kind, c.into_iter().chain(mid).collect()).unwrap();
                        next += 1;
                    }
                    ElementKind::Quad4 => {
                        m.set_element(next, kind, c.to_vec()).unwrap();
                        next += 1;
                    }
                    _ => {
                        m.set_element(next, kind, vec![c[0], c[1], c[2]]).unwrap();
                        m.set_element(next + 1, kind, vec![c[0], c[2], c[3]]).unwrap();
                        next += 2;
                    }
                }
            }
        }
        let all: Vec<u32> = m.elements().keys().copied().collect();
        m.add_set("all", Set { elements: all, ..Set::default() }).unwrap();
        let mat = Material { elastic: Some(iso(e, nu)), hyperelastic: Some(Hyperelastic::NeoHookean {}), ..Material::default() };
        m.add_material("Mat", mat).unwrap();
        m.add_section("s", Section { set: "all".into(), material: "Mat".into(), kind: section, formulation: Formulation::default() }).unwrap();
        let axisymmetric = matches!(section, SectionKind::Axisymmetric {});
        // Stretch along x (plane strain) or along the axis y (axisymmetric).
        let (pulled, axis) = if axisymmetric { (1, 1) } else { (0, 0) };
        let on = |k: usize, v: f64| -> Vec<u32> { m.nodes().iter().filter(|(_, n)| near(n.xyz[k], v)).map(|(l, _)| *l).collect() };
        let (low, high, side) = (on(pulled, 0.0), on(pulled, 1.0), on(1 - pulled, 0.0));
        m.add_set("low", Set { nodes: low, ..Set::default() }).unwrap();
        m.add_set("high", Set { nodes: high, ..Set::default() }).unwrap();
        m.add_set("side", Set { nodes: side, ..Set::default() }).unwrap();
        let hold = |k: usize, v: Option<f64>| {
            let mut values = [None; 6];
            values[k] = v;
            values
        };
        m.add_boundary_condition("low", BoundaryCondition::Displacement { target: Target::Set("low".into()), values: hold(axis, Some(0.0)) })
            .unwrap();
        m.add_boundary_condition("high", BoundaryCondition::Displacement { target: Target::Set("high".into()), values: hold(axis, Some(lam - 1.0)) })
            .unwrap();
        m.add_boundary_condition("side", BoundaryCondition::Displacement { target: Target::Set("side".into()), values: hold(1 - axis, Some(0.0)) })
            .unwrap();
        let mut step = Step::new("Step", nonlinear());
        step.boundary_conditions = ["low", "high", "side"].iter().map(|s| s.to_string()).collect();
        m.add_step(step, None).unwrap();
        let r = solve_nonlinear(&m, None, &Limits::default()).unwrap();
        let lateral = if axisymmetric { l_axi } else { l_plane };
        for (i, label) in r.nodes.iter().enumerate() {
            let x = m.node(*label).unwrap().xyz;
            let want = [(lateral - 1.0) * x[1 - pulled], (lam - 1.0) * x[pulled]];
            let (got_axial, got_lateral) = (r.displacement[i][pulled], r.displacement[i][1 - pulled]);
            assert!(
                (got_axial - want[1]).abs() < 1e-8 && (got_lateral - want[0]).abs() < 1e-8,
                "{section:?} {kind:?} node {label}: {:?}",
                r.displacement[i]
            );
        }
        if axisymmetric {
            assert!(r.stress.iter().all(|s| (s[1] - sigma_axi).abs() < 1e-7 * sigma_axi && s[0].abs() < 1e-7 * sigma_axi), "{:?}", r.stress[0]);
        }
    }
}

/// Newton converges quadratically near the solution: in the last iterations of each increment
/// the residual falls at least like the square of the previous one.
#[test]
fn newton_converges_quadratically() {
    let mut g = grid([8, 1, 2], ElementKind::Hex20, |q| [10.0 * q[0], q[1], q[2]]);
    let mut step = NonlinearStatic { initial_increment: 0.5, line_search: false, ..NonlinearStatic::default() };
    step.tolerances.force = 1e-12;
    step.tolerances.displacement = 1e-12;
    step.tolerances.energy = 1e-20;
    g.define(iso(1000.0, 0.0), None, full(), Procedure::NonlinearStatic(step));
    g.fixed("root", |q| near(q[0], 0.0));
    g.face_set("tip", |q| near(q[0], 1.0));
    // P L² / (E I) = 1: a tip rotation of about 0.46 rad.
    let p = 1000.0 * (2.0f64.powi(3) / 12.0) / 100.0;
    g.load("push", Load::Force { target: Target::Set("tip".into()), vector: [0.0, 0.0, -p], distribution: Distribution::Total });
    let r = solve(&g);
    let h = r.nonlinear.unwrap();
    // The order of convergence ln(r₂/r₁) / ln(r₁/r₀) over successive residuals in the
    // asymptotic range (below 10⁻³, above round-off): 2 for Newton with the consistent tangent.
    let mut best = 0.0f64;
    for inc in h.increments.iter().filter(|i| i.converged) {
        let res: Vec<f64> = inc.iterations.iter().map(|i| i.force).collect();
        let orders: Vec<f64> = res.windows(3).filter(|w| w[0] < 1e-3 && w[2] > 1e-15).map(|w| (w[2] / w[1]).ln() / (w[1] / w[0]).ln()).collect();
        eprintln!("increment {} (λ = {}): residuals {res:?}, orders {orders:?}", inc.number, inc.load_factor);
        best = orders.iter().fold(best, |m, o| m.max(*o));
    }
    assert!(best > 1.8, "no quadratic convergence (best order {best})");
}

/// Without nonlinear geometry, and with a small load with it, the step is the linear solution.
#[test]
fn small_loads_and_small_kinematics_give_the_linear_solution() {
    let build = |procedure: Procedure, scale: f64| {
        let mut g = grid([6, 1, 2], ElementKind::Hex20, |q| [6.0 * q[0], q[1], q[2]]);
        g.define(iso(1000.0, 0.3), None, full(), procedure);
        g.fixed("root", |q| near(q[0], 0.0));
        g.face_set("tip", |q| near(q[0], 1.0));
        g.load("push", Load::Force { target: Target::Set("tip".into()), vector: [0.0, 0.0, -scale], distribution: Distribution::Total });
        g.face_set("top", |q| near(q[2], 1.0));
        g.load("press", Load::Pressure { set: "top".into(), magnitude: 0.1 * scale });
        g
    };
    let linear = solve_static(&build(Procedure::Static {}, 1e-3).model, None, &Limits::default()).unwrap();
    let small = NonlinearStatic { nonlinear_geometry: false, ..NonlinearStatic::default() };
    let r = solve(&build(Procedure::NonlinearStatic(small), 1e-3));
    let tip = linear.displacement.iter().fold(0.0f64, |m, u| m.max(u[2].abs()));
    for (a, b) in r.displacement.iter().zip(&linear.displacement) {
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-9 * tip), "{a:?} vs {b:?}");
    }
    let r = solve(&build(nonlinear(), 1e-3));
    for (a, b) in r.displacement.iter().zip(&linear.displacement) {
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-3 * tip), "geometric effects of order u/L: {a:?} vs {b:?}");
    }
}

/// Hostile and impossible input: the step's controls are refused when defined; an element
/// that inverts makes the step cut back and stop at the minimum increment with the element
/// named; the caps stop a step; unsupported elements and loads are refused by name.
#[test]
fn nonlinear_steps_fail_cleanly() {
    let base = || {
        let mut g = grid([2, 1, 1], ElementKind::Hex8, |q| q);
        let step = NonlinearStatic { min_increment: 1e-3, ..NonlinearStatic::default() };
        g.define(iso(100.0, 0.3), None, full(), Procedure::NonlinearStatic(step));
        neo_hookean(&mut g, 100.0, 0.3);
        g.fixed("root", |q| near(q[0], 0.0));
        g
    };
    // Squash the bar through itself: past −100 % every element inverts.
    let mut g = base();
    g.node_set("x1", |q| near(q[0], 1.0));
    g.bc("crush", BoundaryCondition::Displacement { target: Target::Set("x1".into()), values: [Some(-1.5), Some(0.0), Some(0.0), None, None, None] });
    let err = solve_nonlinear(&g.model, None, &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("stopped at load factor") && err.contains("element"), "{err}");
    // A tight cap on the Newton iterations.
    let err = solve_nonlinear(&g.model, None, &Limits { max_newton_iterations: 3, ..Limits::default() }).unwrap_err().to_string();
    assert!(err.contains("Newton iterations"), "{err}");
    // The increment cap.
    let mut capped = base();
    capped.node_set("x1", |q| near(q[0], 1.0));
    capped.bc("pull", BoundaryCondition::Displacement { target: Target::Set("x1".into()), values: [Some(0.5), None, None, None, None, None] });
    let mut step = capped.model.step("Step").unwrap().clone();
    step.set_procedure(Procedure::NonlinearStatic(NonlinearStatic {
        initial_increment: 0.01,
        max_increment: 0.01,
        min_increment: 0.01,
        max_increments: 3,
        ..NonlinearStatic::default()
    }));
    capped.model.replace_step(step).unwrap();
    let err = solve_nonlinear(&capped.model, None, &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("increments reached the step's cap"), "{err}");
    // Hostile controls never reach the solver.
    let d = NonlinearStatic::default();
    for bad in [
        NonlinearStatic { initial_increment: f64::NAN, ..d.clone() },
        NonlinearStatic { initial_increment: 0.0, ..d.clone() },
        NonlinearStatic { max_iterations: u32::MAX, ..d.clone() },
        NonlinearStatic { max_increments: 0, ..d.clone() },
        NonlinearStatic { tolerances: Tolerances { force: -1.0, ..Tolerances::default() }, ..d.clone() },
    ] {
        let mut step = capped.model.step("Step").unwrap().clone();
        step.set_procedure(Procedure::NonlinearStatic(bad.clone()));
        assert!(capped.model.replace_step(step).is_err(), "{bad:?}");
    }
    // Refused by name: incompatible modes, a hybrid St Venant–Kirchhoff element, temperature.
    let mut im = grid([2, 1, 1], ElementKind::Hex8, |q| q);
    im.define(iso(100.0, 0.3), None, incompatible(), nonlinear());
    im.fixed("root", |q| near(q[0], 0.0));
    im.face_set("end", |q| near(q[0], 1.0));
    im.load("p", Load::Pressure { set: "end".into(), magnitude: 1.0 });
    let err = solve_nonlinear(&im.model, None, &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("incompatible modes in nonlinear steps"), "{err}");
    let mut hy = grid([2, 1, 1], ElementKind::Hex8, |q| q);
    hy.define(iso(100.0, 0.3), None, Formulation { hybrid: true, ..Formulation::default() }, nonlinear());
    hy.fixed("root", |q| near(q[0], 0.0));
    hy.face_set("end", |q| near(q[0], 1.0));
    hy.load("p", Load::Pressure { set: "end".into(), magnitude: 1.0 });
    let err = solve_nonlinear(&hy.model, None, &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("hybrid elements of a St Venant–Kirchhoff material"), "{err}");
    hy.load("hot", Load::Temperature { target: Target::Set("root".into()), value: 50.0 });
    neo_hookean(&mut hy, 100.0, 0.3);
    let err = solve_nonlinear(&hy.model, None, &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("temperature loads"), "{err}");
    // A model that can move freely: the first tangent is singular, named like the linear solver.
    let mut free = grid([2, 1, 1], ElementKind::Hex8, |q| q);
    free.define(iso(100.0, 0.3), None, full(), nonlinear());
    free.face_set("end", |q| near(q[0], 1.0));
    free.load("p", Load::Pressure { set: "end".into(), magnitude: 1.0 });
    let err = solve_nonlinear(&free.model, None, &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("singular at node"), "{err}");
    // A linear static step is not a nonlinear one.
    let mut lin = grid([1, 1, 1], ElementKind::Hex8, |q| q);
    lin.define(iso(100.0, 0.3), None, full(), Procedure::Static {});
    assert!(solve_nonlinear(&lin.model, None, &Limits::default()).is_err());
}

/// The same model gives the same numbers bit for bit, whatever the thread count.
#[test]
fn nonlinear_results_are_deterministic() {
    let mut g = grid([3, 1, 1], ElementKind::Tet10, |q| [4.0 * q[0], q[1], q[2]]);
    g.define(iso(100.0, 0.3), None, full(), nonlinear());
    neo_hookean(&mut g, 100.0, 0.3);
    g.fixed("root", |q| near(q[0], 0.0));
    g.face_set("top", |q| near(q[2], 1.0));
    g.load("press", Load::Pressure { set: "top".into(), magnitude: 0.2 });
    let a = fieldcraft_parallel::with_threads(1, || solve(&g));
    for threads in [2, 7] {
        assert_eq!(fieldcraft_parallel::with_threads(threads, || solve(&g)), a, "{threads} threads");
    }
}
