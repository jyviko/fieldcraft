//! One validity criterion: the meshers' gate and both solvers decide every element alike.
//!
//! The end-to-end demo of 2026-10-13 met a raw CDT sliver that the gate accepted (its exact
//! orientation is positive) and the solver refused (its Jacobian was below the solver's
//! tolerance). Both now use `fieldcraft_model::admissible_jacobian`: the gate on the whole
//! element, the implicit solver at its integration points, the explicit tetrahedron on its one
//! Jacobian.

use fieldcraft_fem::uniform::UniformStrain;
use fieldcraft_fem::{Formulation, Solid, material::isotropic};
use fieldcraft_mesh::quality::{Validity, element_validity};
use fieldcraft_model::{ElementKind, MIN_NORMALIZED_JACOBIAN, normalized_jacobian, tet4_jacobian};

fn implicit_accepts(kind: ElementKind, xyz: &[[f64; 3]]) -> bool {
    let d = isotropic(210_000.0, 0.3).unwrap();
    Solid::new(kind, xyz, Formulation::Full).and_then(|e| e.stiffness(&d)).is_ok()
}

fn gate_accepts(kind: ElementKind, xyz: &[[f64; 3]]) -> bool {
    element_validity(kind, xyz).map(|c| c.validity == Validity::Valid).unwrap_or(false)
}

/// Slivers flattening through the criterion's threshold, near and far from the origin: the gate,
/// the implicit solver and the explicit tetrahedron agree on every one, and some of them are
/// exactly positive yet inadmissible (the case the demo met).
#[test]
fn gate_and_solvers_agree_on_slivers() {
    let mut refused_positive = 0;
    let mut accepted = 0;
    for offset in [0.0, 1e3, -7.25e4] {
        for k in 0..400 {
            // Heights from 1e-17 to 1e-7 of the edge length, on a log scale with jitter.
            let h = 10f64.powf(-17.0 + 10.0 * k as f64 / 400.0) * (1.0 + 0.37 * ((k * 7919) % 13) as f64 / 13.0);
            let o = [offset, 0.5 * offset, -offset];
            let xyz = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.4, 0.35, h]].map(|p| [p[0] + o[0], p[1] + o[1], p[2] + o[2]]);
            let gate = gate_accepts(ElementKind::Tet4, &xyz);
            let implicit = implicit_accepts(ElementKind::Tet4, &xyz);
            let explicit = UniformStrain::tet4(&xyz).is_ok();
            assert_eq!(
                (gate, implicit, explicit),
                (gate, gate, gate),
                "h = {h:e}, offset {offset}: gate {gate}, implicit {implicit}, explicit {explicit}"
            );
            let q = normalized_jacobian(&tet4_jacobian(&xyz));
            if !gate && q > 0.0 {
                refused_positive += 1;
            }
            accepted += usize::from(gate);
            assert_eq!(gate, q > MIN_NORMALIZED_JACOBIAN, "h = {h:e}");
        }
    }
    assert!(refused_positive > 0 && accepted > 0, "{refused_positive} refused, {accepted} accepted");
}

/// A curved tetrahedron the gate accepts is accepted by the implicit solver at every integration
/// point; the gate proves the criterion on the whole element.
#[test]
fn gate_accepts_only_what_the_solver_accepts_for_tet10() {
    let corners = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, 0.3, 1.0]];
    let edges = [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)];
    for k in 0..200 {
        let bend = -0.6 + 1.2 * k as f64 / 200.0;
        let mut xyz: Vec<[f64; 3]> = corners.to_vec();
        for (e, &(a, b)) in edges.iter().enumerate() {
            let (p, q): ([f64; 3], [f64; 3]) = (corners[a], corners[b]);
            let mut m = [0.5 * (p[0] + q[0]), 0.5 * (p[1] + q[1]), 0.5 * (p[2] + q[2])];
            if e == 1 {
                m[2] += bend;
            }
            xyz.push(m);
        }
        if gate_accepts(ElementKind::Tet10, &xyz) {
            assert!(implicit_accepts(ElementKind::Tet10, &xyz), "bend {bend}");
        }
    }
}
