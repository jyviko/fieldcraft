//! Thermal strain in nonlinear static steps against closed forms: free expansion and a fully
//! constrained heated block at finite strain (the multiplicative split F = F_m F_θ), a heated
//! bar held at its ends past yield at small strain (the additive split), the linear limit
//! against the linear solver, and a step's own heat conduction feeding its temperatures.

mod common;

use common::*;
use fieldcraft_model::*;
use fieldcraft_solver::{Limits, StaticResult, solve_nonlinear, solve_static};

fn expanding(g: &mut Grid, e: f64, nu: f64, alpha: f64, hyperelastic: Option<Hyperelastic>, plastic: Option<Plastic>) {
    let thermal = Thermal {
        expansion: Some(Directional::Isotropic(alpha)),
        reference_temperature: 20.0,
        conductivity: Some(Directional::Isotropic(45.0)),
        ..Thermal::default()
    };
    let m = Material { elastic: Some(iso(e, nu)), hyperelastic, plastic, thermal: Some(thermal), ..Material::default() };
    g.model.replace_material("Mat", m).unwrap();
}

fn heat_all(g: &mut Grid, value: f64) {
    g.node_set("everything", |_| true);
    g.load("heat", Load::Temperature { target: Target::Set("everything".into()), value });
}

fn solve(g: &Grid) -> StaticResult {
    solve_nonlinear(&g.model, Some("Step"), &Limits::default()).unwrap()
}

fn symmetric(g: &mut Grid) {
    g.symmetry("x0", Symmetry::X, |x| near(x[0], 0.0));
    g.symmetry("y0", Symmetry::Y, |x| near(x[1], 0.0));
    g.symmetry("z0", Symmetry::Z, |x| near(x[2], 0.0));
}

/// A free neo-Hookean cube heated by 200 with α = 10⁻³ grows by the thermal stretch 1.2 along
/// every edge, stress-free; held on all six faces it takes the pressure of the mechanical
/// volume ratio ϑ⁻³: σ = κ (ϑ⁻³ − 1), full and hybrid elements.
#[test]
fn finite_strain_free_and_constrained_expansion() {
    let (e, nu, alpha) = (10.0, 0.3, 1e-3);
    let kappa = e / (3.0 * (1.0 - 2.0 * nu));
    let theta: f64 = 1.0 + alpha * 200.0;
    for formulation in [Formulation::default(), Formulation { hybrid: true, ..Formulation::default() }] {
        let mut g = grid([2, 2, 2], ElementKind::Hex20, |x| x);
        g.define(iso(e, nu), None, formulation, Procedure::NonlinearStatic(NonlinearStatic::default()));
        expanding(&mut g, e, nu, alpha, Some(Hyperelastic::NeoHookean {}), None);
        symmetric(&mut g);
        heat_all(&mut g, 220.0);
        let r = solve(&g);
        for (label, u) in r.nodes.iter().zip(&r.displacement) {
            let x = g.model.node(*label).unwrap().xyz;
            assert!((0..3).all(|k| (u[k] - (theta - 1.0) * x[k]).abs() < 1e-9), "{formulation:?} node {label}: {u:?}");
        }
        assert!(r.stress.iter().flatten().all(|s| s.abs() < 1e-9 * kappa), "{formulation:?}: {:?}", r.stress[0]);
        assert!((r.elements[0].points[0].strain[0] - theta.ln()).abs() < 1e-9, "the total logarithmic strain");

        g.symmetry("x1", Symmetry::X, |x| near(x[0], 1.0));
        g.symmetry("y1", Symmetry::Y, |x| near(x[1], 1.0));
        g.symmetry("z1", Symmetry::Z, |x| near(x[2], 1.0));
        let r = solve(&g);
        let want = kappa * (theta.powi(-3) - 1.0);
        for s in &r.stress {
            assert!((0..3).all(|k| (s[k] - want).abs() < 1e-9 * kappa) && s[3].abs() < 1e-9 * kappa, "{formulation:?}: {s:?} vs {want}");
        }
        eprintln!("{formulation:?}: constrained σ = {:.9} (closed form {want:.9})", r.stress[0][0]);
    }
}

/// A steel bar held axially at both ends (lateral faces free) and heated at small strain: the
/// mechanical strain is −αΔT, so the stress is −EαΔT until it yields, then
/// −(σ_y + E_t (αΔT − σ_y/E)) with linear hardening.
#[test]
fn small_strain_heated_bar_held_at_its_ends_yields() {
    let (e, nu, alpha, y0, h) = (200_000.0, 0.3, 1.2e-5, 250.0, 2000.0);
    let small = NonlinearStatic { nonlinear_geometry: false, ..NonlinearStatic::default() };
    let plastic = Plastic::Bilinear { yield_stress: y0, hardening_modulus: h, hardening: Hardening::Isotropic };
    let et = e * h / (e + h);
    for (dt, want) in [(80.0, -e * alpha * 80.0), (300.0, -(y0 + et * (alpha * 300.0 - y0 / e)))] {
        let mut g = grid([4, 1, 1], ElementKind::Hex8, |x| [4.0 * x[0], x[1], x[2]]);
        g.define(iso(e, nu), None, Formulation::default(), Procedure::NonlinearStatic(small.clone()));
        expanding(&mut g, e, nu, alpha, None, Some(plastic.clone()));
        symmetric(&mut g);
        g.symmetry("x1", Symmetry::X, |x| near(x[0], 1.0));
        heat_all(&mut g, 20.0 + dt);
        let r = solve(&g);
        for s in &r.stress {
            assert!((s[0] - want).abs() < 1e-8 * want.abs() && s[1].abs() < 1e-8 * want.abs(), "ΔT = {dt}: {s:?} vs {want}");
        }
        eprintln!("ΔT = {dt}: σ = {:.6} (closed form {want:.6})", r.stress[0][0]);
    }
}

/// Without nonlinear geometry an elastic model is the linear solution, thermal strain
/// included; a step's own temperature boundary conditions give its temperatures by conduction,
/// as in the linear coupled step.
#[test]
fn thermal_linear_limit_and_conducted_temperatures() {
    let (e, nu, alpha) = (200_000.0, 0.3, 1.2e-5);
    let build = |procedure: Procedure, plastic: Option<Plastic>, conducted: bool| {
        let mut g = grid([6, 1, 2], ElementKind::Hex20, |q| [6.0 * q[0], q[1], q[2]]);
        g.define(iso(e, nu), None, Formulation::default(), procedure);
        expanding(&mut g, e, nu, alpha, None, plastic);
        g.fixed("root", |q| near(q[0], 0.0));
        if conducted {
            g.node_set("cold", |q| near(q[0], 0.0));
            g.bc("cold_t", BoundaryCondition::Temperature { target: Target::Set("cold".into()), value: 20.0 });
            g.node_set("hot", |q| near(q[0], 1.0));
            g.bc("hot_t", BoundaryCondition::Temperature { target: Target::Set("hot".into()), value: 320.0 });
        } else {
            // Hot on top, cold below: the bar bends.
            g.node_set("top", |q| near(q[2], 1.0));
            g.load("hot", Load::Temperature { target: Target::Set("top".into()), value: 120.0 });
        }
        g
    };
    let small = Procedure::NonlinearStatic(NonlinearStatic { nonlinear_geometry: false, ..NonlinearStatic::default() });
    let linear = solve_static(&build(Procedure::Static {}, None, false).model, None, &Limits::default()).unwrap();
    let r = solve(&build(small.clone(), None, false));
    let scale = linear.displacement.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
    for (a, b) in r.displacement.iter().zip(&linear.displacement) {
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-9 * scale), "{a:?} vs {b:?}");
    }
    for (a, b) in r.stress.iter().zip(&linear.stress) {
        assert!((0..6).all(|k| (a[k] - b[k]).abs() < 1e-6 * e * alpha * 100.0), "{a:?} vs {b:?}");
    }
    // Conducted: T linear in x from 20 to 320 over a bar fixed at its cold end; the step's
    // temperature conditions are solved first, as the coupled linear step does, and a plastic
    // material that stays below yield gives that step's solution.
    let plastic = Plastic::Bilinear { yield_stress: 250.0, hardening_modulus: 0.0, hardening: Hardening::Isotropic };
    let r = solve(&build(small, Some(plastic), true));
    let coupled = solve_static(&build(Procedure::CoupledTemperatureDisplacement {}, None, true).model, None, &Limits::default()).unwrap();
    let largest = coupled.von_mises.iter().fold(0.0f64, |m, v| m.max(*v));
    assert!(largest < 250.0 && largest > 1.0, "{largest}");
    let scale = coupled.displacement.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
    for (a, b) in r.displacement.iter().zip(&coupled.displacement) {
        assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-8 * scale), "{a:?} vs {b:?}");
    }
    assert!(r.elements.iter().flat_map(|e| &e.plastic_strain).all(|p| *p == 0.0));
    // Both ways of giving temperatures at once is refused.
    let mut both = build(Procedure::NonlinearStatic(NonlinearStatic::default()), None, true);
    heat_all(&mut both, 50.0);
    let err = solve_nonlinear(&both.model, None, &Limits::default()).unwrap_err().to_string();
    assert!(err.contains("both by temperature loads and by heat conditions"), "{err}");
}
