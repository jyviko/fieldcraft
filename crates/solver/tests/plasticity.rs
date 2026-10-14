//! Implicit J2 plasticity in nonlinear static steps against closed forms: uniaxial tension at
//! finite strain (Kirchhoff stress against the logarithmic strain) and at small strain in every
//! element kind, the Bauschinger effect of linear kinematic hardening on a load path that comes
//! back, Newton's quadratic rate with the consistent tangent, and what plastic steps refuse or
//! stop at. Run with `--nocapture` to see the residuals.

mod common;

use common::*;
use fieldcraft_model::*;
use fieldcraft_solver::{Limits, StaticResult, solve_nonlinear};

const E: f64 = 200_000.0;
const NU: f64 = 0.3;
const Y0: f64 = 250.0;
const H: f64 = 2000.0;

fn bilinear(hardening: Hardening) -> Plastic {
    Plastic::Bilinear { yield_stress: Y0, hardening_modulus: H, hardening }
}

fn plastic(g: &mut Grid, p: Plastic) {
    let m = Material { elastic: Some(iso(E, NU)), plastic: Some(p), ..Material::default() };
    g.model.replace_material("Mat", m).unwrap();
}

/// A unit cube on three symmetry planes, its x = 1 face pulled by `stretch` (uniaxial stress).
fn cube(kind: ElementKind, formulation: Formulation, step: NonlinearStatic, p: Plastic, stretch: f64) -> Grid {
    let mut g = grid([2, 2, 2], kind, |x| x);
    g.define(iso(E, NU), None, formulation, Procedure::NonlinearStatic(step));
    plastic(&mut g, p);
    g.symmetry("x0", Symmetry::X, |x| near(x[0], 0.0));
    g.symmetry("y0", Symmetry::Y, |x| near(x[1], 0.0));
    g.symmetry("z0", Symmetry::Z, |x| near(x[2], 0.0));
    g.node_set("x1", |x| near(x[0], 1.0));
    g.bc("pull", BoundaryCondition::Displacement { target: Target::Set("x1".into()), values: [Some(stretch), None, None, None, None, None] });
    g
}

fn solve(g: &Grid) -> StaticResult {
    solve_nonlinear(&g.model, Some("Step"), &Limits::default()).unwrap()
}

/// The uniaxial stress of linear hardening at strain ε past yield: σ_y + E_t (ε − σ_y/E).
fn uniaxial(strain: f64) -> f64 {
    let et = E * H / (E + H);
    Y0 + et * (strain - Y0 / E)
}

fn hybrid() -> Formulation {
    Formulation { hybrid: true, ..Formulation::default() }
}

/// Every element kind (and hybrid hex8, hex20, tet10) stretched to a logarithmic strain of 0.2:
/// the Kirchhoff stress τ = J σ follows the small-strain curve in the logarithmic strain
/// exactly (Simo's exponential map integrates a proportional path exactly), and the
/// equivalent plastic strain is (τ − σ_y)/H.
#[test]
fn finite_strain_uniaxial_tension_follows_the_logarithmic_closed_form() {
    let strain: f64 = 0.2;
    let tau = uniaxial(strain);
    let mut cases: Vec<(ElementKind, Formulation, &str)> = variants().into_iter().filter(|v| v.1 != incompatible()).collect();
    cases.extend([
        (ElementKind::Hex8, hybrid(), "hex8 hybrid"),
        (ElementKind::Hex20, hybrid(), "hex20 hybrid"),
        (ElementKind::Tet10, hybrid(), "tet10 hybrid"),
    ]);
    for (kind, formulation, name) in cases {
        let g = cube(kind, formulation, NonlinearStatic::default(), bilinear(Hardening::Isotropic), strain.exp() - 1.0);
        let r = solve(&g);
        for e in &r.elements {
            for (p, ep) in e.points.iter().zip(&e.plastic_strain) {
                // J from the logarithmic strains, σ the Cauchy stress.
                let j = (p.strain[0] + p.strain[1] + p.strain[2]).exp();
                assert!((p.stress[0] * j - tau).abs() < 1e-7 * tau, "{name}: τ = {} vs {tau}", p.stress[0] * j);
                assert!(p.stress[1].abs() < 1e-7 * tau && p.stress[3].abs() < 1e-7 * tau, "{name}: {:?}", p.stress);
                assert!((ep - (tau - Y0) / H).abs() < 1e-9, "{name}: ε_p = {ep}");
                assert!((p.strain[0] - strain).abs() < 1e-12, "{name}: {:?}", p.strain);
            }
            assert_eq!(e.plastic_strain.len(), e.points.len(), "{name}");
        }
        let h = r.nonlinear.as_ref().unwrap();
        eprintln!("{name}: {} increments, {} iterations", h.increments.len(), h.iterations);
    }
}

/// Without nonlinear geometry: σ = σ_y + E_t (ε − σ_y/E) in the small strain, isotropic and
/// kinematic hardening alike under monotonic loading, hybrid elements too.
#[test]
fn small_strain_uniaxial_tension_follows_the_closed_form() {
    let strain = 0.01;
    let sigma = uniaxial(strain);
    let small = NonlinearStatic { nonlinear_geometry: false, ..NonlinearStatic::default() };
    for hardening in [Hardening::Isotropic, Hardening::Kinematic] {
        for (kind, formulation, name) in [
            (ElementKind::Hex8, full(), "hex8"),
            (ElementKind::Tet10, full(), "tet10"),
            (ElementKind::Hex20, reduced(), "hex20 reduced"),
            (ElementKind::Hex20, hybrid(), "hex20 hybrid"),
        ] {
            let r = solve(&cube(kind, formulation, small.clone(), bilinear(hardening), strain));
            for e in &r.elements {
                for (p, ep) in e.points.iter().zip(&e.plastic_strain) {
                    assert!((p.stress[0] - sigma).abs() < 1e-8 * sigma && p.stress[1].abs() < 1e-8 * sigma, "{name} {hardening:?}: {:?}", p.stress);
                    assert!((ep - (sigma - Y0) / H).abs() < 1e-10, "{name}: {ep}");
                }
            }
        }
    }
}

/// The Bauschinger effect: pulled to ε_a and brought back to zero strain on a load path,
/// linear kinematic hardening yields again in reverse at σ_a − 2σ_y and ends at the residual
/// stress σ_a − 2σ_y − E_t (ε_a − 2σ_y/E); isotropic hardening yields again only at −σ_a and
/// ends at −σ_a − E_t (ε_a − 2σ_a/E).
#[test]
fn kinematic_hardening_shows_the_bauschinger_effect_on_unloading() {
    let strain = 0.01;
    let sa = uniaxial(strain);
    let et = E * H / (E + H);
    let path = NonlinearStatic {
        nonlinear_geometry: false,
        initial_increment: 0.05,
        max_increment: 0.05,
        load_path: Some(vec![[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]]),
        ..NonlinearStatic::default()
    };
    for (hardening, want) in
        [(Hardening::Kinematic, sa - 2.0 * Y0 - et * (strain - 2.0 * Y0 / E)), (Hardening::Isotropic, -sa - et * (strain - 2.0 * sa / E))]
    {
        let r = solve(&cube(ElementKind::Hex8, full(), path.clone(), bilinear(hardening), strain));
        let s = r.elements[0].points[0].stress[0];
        assert!((s - want).abs() < 1e-8 * sa, "{hardening:?}: residual stress {s} vs {want}");
        assert!(r.displacement.iter().all(|u| u[0].abs() < 1e-12), "back at zero strain");
        let h = r.nonlinear.unwrap();
        assert!(h.increments.iter().any(|i| i.converged && i.load_factor == 1.0), "the path's turn is an increment's end");
        assert_eq!(h.frames.last().map(|f| f.load_factor), Some(0.0));
        eprintln!("{hardening:?}: residual stress {s:.6} (closed form {want:.6})");
    }
}

/// Newton with the consistent tangent converges quadratically in a plastic structure: a notched
/// bar pulled past general yield, and a cantilever bent into plastic flow at finite strain.
#[test]
fn plastic_newton_converges_quadratically() {
    for nonlinear_geometry in [false, true] {
        let mut step = NonlinearStatic { initial_increment: 0.5, nonlinear_geometry, ..NonlinearStatic::default() };
        step.tolerances.force = 1e-8;
        step.tolerances.displacement = 1e-8;
        step.tolerances.energy = 1e-16;
        let mut g = grid([8, 1, 2], ElementKind::Hex20, |q| [10.0 * q[0], q[1], q[2]]);
        g.define(iso(E, NU), None, full(), Procedure::NonlinearStatic(step));
        plastic(&mut g, bilinear(Hardening::Isotropic));
        g.fixed("root", |q| near(q[0], 0.0));
        g.node_set("tip", |q| near(q[0], 1.0));
        // A tip deflection of ten times the first yield's.
        let first_yield = 2.0 * Y0 * 100.0 / (3.0 * E * 2.0);
        g.bc(
            "push",
            BoundaryCondition::Displacement { target: Target::Set("tip".into()), values: [None, None, Some(-10.0 * first_yield), None, None, None] },
        );
        let r = solve(&g);
        assert!(r.elements.iter().flat_map(|e| &e.plastic_strain).any(|p| *p > 0.0), "plastic");
        let h = r.nonlinear.unwrap();
        let mut best = 0.0f64;
        for inc in h.increments.iter().filter(|i| i.converged) {
            let res: Vec<f64> = inc.iterations.iter().map(|i| i.force).collect();
            let orders: Vec<f64> = res.windows(3).filter(|w| w[0] < 1e-2 && w[2] > 1e-11).map(|w| (w[2] / w[1]).ln() / (w[1] / w[0]).ln()).collect();
            eprintln!("geometry {nonlinear_geometry}: increment {} (λ = {}): residuals {res:?}, orders {orders:?}", inc.number, inc.load_factor);
            best = orders.iter().fold(best, |m, o| m.max(*o));
        }
        assert!(best > 1.8, "nonlinear geometry {nonlinear_geometry}: no quadratic convergence (best order {best})");
    }
}

/// What a static step cannot take is refused by name; a material that softens faster than it
/// unloads makes the step cut back and stop cleanly; the results do not depend on the threads.
#[test]
fn plastic_steps_refuse_and_fail_cleanly() {
    let jc = |c: f64| Plastic::JohnsonCook {
        a: 350.0,
        b: 275.0,
        n: 0.36,
        c,
        m: 1.0,
        reference_strain_rate: 1.0,
        room_temperature: 293.0,
        melt_temperature: 1356.0,
        specific_heat: Some(4.5e8),
        taylor_quinney: 0.9,
    };
    let err = solve_nonlinear(&cube(ElementKind::Hex8, full(), NonlinearStatic::default(), jc(0.02), 0.01).model, None, &Limits::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("strain-rate term") && err.contains("C = 0.02"), "{err}");
    // C = 0 is the rate-independent law: it solves, warning that a static step does not heat.
    let r = solve(&cube(ElementKind::Hex8, full(), NonlinearStatic::default(), jc(0.0), 0.01));
    assert!(r.warnings.iter().any(|w| w.contains("isothermal")), "{:?}", r.warnings);
    let err = solve_nonlinear(
        &cube(ElementKind::Hex8, full(), NonlinearStatic::default(), bilinear(Hardening::Kinematic), 0.01).model,
        None,
        &Limits::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("kinematic hardening") && err.contains("finite strain"), "{err}");
    let small = NonlinearStatic { nonlinear_geometry: false, ..NonlinearStatic::default() };
    let err = solve_nonlinear(
        &cube(ElementKind::Hex8, incompatible(), small.clone(), bilinear(Hardening::Isotropic), 0.01).model,
        None,
        &Limits::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("incompatible modes with an inelastic material"), "{err}");
    // Softening steeper than −3G past a small plastic strain: the point localises, the step cuts
    // back to its minimum increment and stops, naming the element.
    let g_mod = E / (2.0 * (1.0 + NU));
    let softening =
        Plastic::Table { points: vec![HardeningPoint { strain: 0.0, stress: Y0 }, HardeningPoint { strain: 1e-4, stress: Y0 - 4.0 * g_mod * 1e-4 }] };
    let soft = NonlinearStatic { min_increment: 1e-3, ..small.clone() };
    let err = solve_nonlinear(&cube(ElementKind::Hex8, full(), soft, softening, 0.01).model, None, &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("stopped at load factor") && err.contains("element"), "{err}");
    // Damage is left out with a warning.
    let mut g = cube(ElementKind::Hex8, full(), small, bilinear(Hardening::Isotropic), 0.01);
    let mut m = g.model.material("Mat").unwrap().clone();
    m.damage = Some(Damage::JohnsonCook { d1: 0.5, d2: 0.0, d3: 0.0, d4: 0.0, d5: 0.0 });
    g.model.replace_material("Mat", m).unwrap();
    let r = solve(&g);
    assert!(r.warnings.iter().any(|w| w.contains("damage and erosion")), "{:?}", r.warnings);
    // Hostile plasticity never reaches the solver: non-finite or negative tables and yield.
    let mut g = cube(ElementKind::Hex8, full(), NonlinearStatic::default(), bilinear(Hardening::Isotropic), 0.01);
    for bad in [
        Plastic::Bilinear { yield_stress: -250.0, hardening_modulus: H, hardening: Hardening::Isotropic },
        Plastic::Bilinear { yield_stress: Y0, hardening_modulus: f64::NAN, hardening: Hardening::Isotropic },
        Plastic::Table { points: vec![HardeningPoint { strain: 0.0, stress: f64::INFINITY }] },
        Plastic::Table { points: vec![HardeningPoint { strain: 0.0, stress: Y0 }, HardeningPoint { strain: f64::NAN, stress: Y0 }] },
        Plastic::Table { points: vec![HardeningPoint { strain: 0.0, stress: Y0 }, HardeningPoint { strain: 0.1, stress: -1.0 }] },
    ] {
        let m = Material { elastic: Some(iso(E, NU)), plastic: Some(bad.clone()), ..Material::default() };
        assert!(g.model.replace_material("Mat", m).is_err(), "{bad:?}");
    }
    // Bit for bit on any number of threads.
    let g = cube(ElementKind::Tet10, full(), NonlinearStatic::default(), bilinear(Hardening::Isotropic), 0.1);
    let a = fieldcraft_parallel::with_threads(1, || solve(&g));
    for threads in [2, 7] {
        assert_eq!(fieldcraft_parallel::with_threads(threads, || solve(&g)), a, "{threads} threads");
    }
}
