//! Arc-length (Riks–Crisfield) control through limit points: the snap-through of a shallow
//! sinusoidal arch of solids under a sinusoidal dead load, against the one-mode closed form of
//! shallow arch theory (exact for this load and shape on the symmetric path), the limit points'
//! load factors and the tangent's inertia along the path, and what arc-length steps refuse.
//!
//! Closed form: an arch y₀ = a sin(πx/L) on pins at a fixed span, bending stiffness EI, axial EA,
//! r² = I/A, under q = q₀ sin(πx/L) deflects in the same shape, w = c sin(πx/L), with
//! q₀ = EI k⁴ [c + (a − c)(2ac − c²)/(4r²)], k = π/L (the energy of bending and of the axial
//! shortening ½∫(y₀′² − y′²)dx, S. P. Timoshenko, J. M. Gere, *Theory of Elastic Stability*, 2nd
//! ed., 1961, §7.6, and Y. C. Fung, A. Kaplan, NACA TN 2840, 1952). Its limit points are at
//! c = a ∓ r √((a²/r² − 4)/3); for a/r = 3.5 the antisymmetric mode never bifurcates first (the
//! axial force stays below the second Euler load 4EIk², reached only at a/r = 4).

mod common;

use common::*;
use fieldcraft_model::*;
use fieldcraft_solver::nonlinear::CriticalKind;
use fieldcraft_solver::{Limits, solve_nonlinear};

const E: f64 = 1.0e4;
const SPAN: f64 = 100.0;
const H: f64 = 1.0;

fn rise() -> f64 {
    3.5 * H / 12f64.sqrt()
}

/// q₀ for a midspan deflection c (positive down).
fn load(c: f64) -> f64 {
    let (a, r2, k) = (rise(), H * H / 12.0, std::f64::consts::PI / SPAN);
    E * H.powi(3) / 12.0 * k.powi(4) * (c + (a - c) * (2.0 * a * c - c * c) / (4.0 * r2))
}

fn limit_points() -> (f64, f64) {
    let (a, r) = (rise(), H / 12f64.sqrt());
    let d = r * ((a * a / (r * r) - 4.0) / 3.0).sqrt();
    (a - d, a + d)
}

/// The arch (40 × 2 × 1 hex20, unit width), its load scaled to the closed-form limit load, and
/// the label of the midspan node at mid-height.
fn arch(arc: Option<ArcLength>) -> (Grid, u32, f64) {
    let a = rise();
    let mut g = grid([40, 2, 1], ElementKind::Hex20, |p| {
        let x = SPAN * p[0];
        [x, a * (std::f64::consts::PI * x / SPAN).sin() + H * (p[1] - 0.5), p[2]]
    });
    let reference = load(limit_points().0);
    let settings = NonlinearStatic { arc_length: arc, initial_increment: 0.1, max_increments: 200, ..NonlinearStatic::default() };
    g.define(iso(E, 0.0), None, Formulation::default(), Procedure::NonlinearStatic(settings));
    g.node_set("pins", |p| (near(p[0], 0.0) || near(p[0], 1.0)) && near(p[1], 0.5));
    g.bc("pins", BoundaryCondition::Displacement { target: Target::Set("pins".into()), values: [Some(0.0), Some(0.0), None, None, None, None] });
    g.node_set("width", |_| true);
    g.bc("width", BoundaryCondition::Displacement { target: Target::Set("width".into()), values: [None, None, Some(0.0), None, None, None] });
    // q₀ sin(πx/L) per unit length, piecewise constant per element on the top face (dead).
    let top = g.faces_where(|p| near(p[1], 1.0));
    for f in top {
        let nodes = &g.model.element(f.element).unwrap().nodes;
        let xc = nodes.iter().take(8).map(|n| g.model.node(*n).unwrap().xyz[0]).sum::<f64>() / 8.0;
        let name = format!("q{}", f.element);
        g.model.add_set(&name, Set { faces: vec![f], ..Set::default() }).unwrap();
        let total = -reference * (std::f64::consts::PI * xc / SPAN).sin() * SPAN / 40.0;
        g.load(&name, Load::Force { target: Target::Set(name.clone()), vector: [0.0, total, 0.0], distribution: Distribution::Total });
    }
    let mid = g.nodes_where(|p| near(p[0], 0.5) && near(p[1], 0.5) && near(p[2], 0.0))[0];
    (g, mid, reference)
}

#[test]
fn shallow_arch_snaps_through() {
    let a = rise();
    let (_, mid, _) = arch(None);
    let arc = ArcLength {
        displacement_limit: Some(DisplacementLimit { node: mid, direction: 2, displacement: -2.2 * a }),
        max_load_factor: 10.0,
        ..ArcLength::default()
    };
    let (g, _, reference) = arch(Some(arc));
    let r = solve_nonlinear(&g.model, Some("Step"), &Limits::default()).unwrap();
    let h = r.nonlinear.as_ref().unwrap();
    let at = r.nodes.binary_search(&mid).unwrap();
    // Every converged state on the closed-form path.
    let mut worst = 0.0f64;
    for frame in &h.frames {
        let c = -frame.displacement[at][1];
        let err = (frame.load_factor * reference - load(c)).abs() / reference;
        worst = worst.max(err);
    }
    let (_, c_valley) = limit_points();
    eprintln!("{} increments; worst path error {:.3} % of the limit load", h.increments.len(), 100.0 * worst);
    for p in &h.critical_points {
        eprintln!("  {:?} at increment {}: λ = {:.5} ({} → {} negative)", p.kind, p.increment, p.load_factor, p.negative_before, p.negative_after);
    }
    assert!(worst < 0.02, "{worst}");
    assert!(-r.displacement[at][1] >= 2.2 * a, "ended at the displacement limit");
    // The limit points: the peak (λ = 1 by construction) and the valley, from the inertia.
    let [peak, valley] = h.critical_points.as_slice() else { panic!("{:?}", h.critical_points) };
    assert_eq!((peak.kind, peak.negative_before, peak.negative_after), (CriticalKind::Limit, 0, 1));
    assert_eq!((valley.kind, valley.negative_before, valley.negative_after), (CriticalKind::Limit, 1, 0));
    assert!((peak.load_factor - 1.0).abs() < 0.02, "{}", peak.load_factor);
    assert!((valley.load_factor - load(c_valley) / reference).abs() < 0.02, "{} vs {}", valley.load_factor, load(c_valley) / reference);
    // Negative pivots exactly between the limit points: every converged state's count follows
    // the sign of dλ along the path.
    let states: Vec<(f64, usize)> = h.increments.iter().filter(|i| i.converged).map(|i| (i.load_factor, i.negative_pivots)).collect();
    for w in states.windows(3) {
        let falling = w[2].0 < w[1].0 && w[1].0 < w[0].0;
        let rising = w[2].0 > w[1].0 && w[1].0 > w[0].0;
        if falling {
            assert_eq!(w[1].1, 1, "{states:?}");
        }
        if rising {
            assert_eq!(w[1].1, 0, "{states:?}");
        }
    }
}

#[test]
fn arc_length_steps_refuse_what_they_cannot_do() {
    let arc = ArcLength::default();
    let mut settings = NonlinearStatic { arc_length: Some(arc.clone()), load_path: Some(vec![[0.0, 0.0], [1.0, 1.0]]), ..NonlinearStatic::default() };
    assert!(settings.validate().is_err(), "a load path");
    settings.load_path = None;
    settings.arc_length = Some(ArcLength { max_load_factor: f64::NAN, ..arc.clone() });
    assert!(settings.validate().is_err());
    settings.arc_length = Some(ArcLength { displacement_limit: Some(DisplacementLimit { node: 1, direction: 7, displacement: 1.0 }), ..arc.clone() });
    assert!(settings.validate().is_err());
    // A node that does not exist; no load at all.
    let (mut g, _, _) =
        arch(Some(ArcLength { displacement_limit: Some(DisplacementLimit { node: 999_999, direction: 2, displacement: 1.0 }), ..arc }));
    assert!(solve_nonlinear(&g.model, Some("Step"), &Limits::default()).is_err());
    let loads: Vec<String> = g.model.step("Step").unwrap().loads.iter().cloned().collect();
    for l in loads {
        g.model.set_load_active("Step", &l, false).unwrap();
    }
    assert!(solve_nonlinear(&g.model, Some("Step"), &Limits::default()).is_err());
}
