//! Element-level checks: rigid-body modes, constant-strain reproduction (the single-element form
//! of the patch test), mass and load totals, extrapolation.

use modelcraft_model::ElementKind;

use crate::dense::{Cholesky, Matrix};
use crate::element::{Formulation, Solid};
use crate::material::{isotropic, orthotropic, stress};

/// A distorted but valid element of each kind, mid-side nodes off the straight edges for the
/// quadratic kinds when `curved`.
fn distorted(kind: ElementKind, curved: bool) -> Vec<[f64; 3]> {
    let corners: Vec<[f64; 3]> = match kind {
        ElementKind::Tet4 | ElementKind::Tet10 => vec![[0.1, -0.05, 0.0], [1.2, 0.1, 0.05], [0.2, 0.9, -0.1], [0.05, 0.15, 1.1]],
        ElementKind::Wedge6 => vec![[0.0, 0.0, 0.0], [2.1, 0.1, -0.1], [0.2, 1.3, 0.05], [0.1, -0.1, 0.9], [2.0, 0.05, 1.1], [0.15, 1.2, 1.0]],
        _ => vec![
            [0.0, 0.0, 0.0],
            [2.1, 0.1, -0.1],
            [2.3, 1.2, 0.05],
            [-0.1, 0.9, 0.1],
            [0.1, -0.1, 0.8],
            [2.0, 0.05, 1.1],
            [2.2, 1.1, 0.9],
            [0.05, 1.05, 1.2],
        ],
    };
    let mut xyz = corners.clone();
    for (i, e) in kind.edges().iter().enumerate() {
        if let [a, b, _] = **e {
            let bump = if curved { 0.03 * ((i % 3) as f64 - 1.0) } else { 0.0 };
            xyz.push([0, 1, 2].map(|k| (corners[a][k] + corners[b][k]) / 2.0 + bump));
        }
    }
    xyz
}

const CASES: [(ElementKind, Formulation); 10] = [
    (ElementKind::Tet4, Formulation::Full),
    (ElementKind::Tet10, Formulation::Full),
    (ElementKind::Wedge6, Formulation::Full),
    (ElementKind::Hex8, Formulation::Full),
    (ElementKind::Hex8, Formulation::IncompatibleModes),
    (ElementKind::Hex20, Formulation::Full),
    (ElementKind::Hex20, Formulation::Reduced),
    (ElementKind::Tet10, Formulation::Hybrid { pressures: 1 }),
    (ElementKind::Hex8, Formulation::Hybrid { pressures: 1 }),
    (ElementKind::Hex20, Formulation::Hybrid { pressures: 4 }),
];

fn rigid_modes(xyz: &[[f64; 3]]) -> Vec<Vec<f64>> {
    let mut modes = Vec::new();
    for k in 0..3 {
        modes.push(xyz.iter().flat_map(|_| [0, 1, 2].map(|c| if c == k { 1.0 } else { 0.0 })).collect());
    }
    for axis in 0..3 {
        // ω × x for ω along `axis`.
        let w = [0, 1, 2].map(|c| if c == axis { 1.0 } else { 0.0 });
        modes.push(xyz.iter().flat_map(|x| [w[1] * x[2] - w[2] * x[1], w[2] * x[0] - w[0] * x[2], w[0] * x[1] - w[1] * x[0]]).collect());
    }
    modes
}

#[test]
fn stiffness_is_symmetric_with_exactly_six_rigid_body_modes() {
    let d = isotropic(1000.0, 0.3).unwrap();
    for (kind, f) in CASES {
        let xyz = distorted(kind, true);
        let k = Solid::new(kind, &xyz, f).unwrap().stiffness(&d).unwrap();
        let scale = k.data.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        for i in 0..k.n {
            for j in 0..k.n {
                assert_eq!(k.get(i, j), k.get(j, i), "{kind:?} {f:?} symmetric");
            }
        }
        for m in rigid_modes(&xyz) {
            let r = k.mul_vec(&m);
            assert!(r.iter().all(|v| v.abs() < 1e-10 * scale), "{kind:?} {f:?}: rigid motion strains the element");
        }
        // Holding six degrees of freedom (node 1 fully, node 2 in y and z, node 3 in z) leaves a
        // positive definite matrix: no other zero-energy mode. Reduced hex20 has spurious modes
        // on a single element, and so has hybrid tet10: its deviatoric stiffness is blind to the
        // four dilatations with a linear volumetric strain, and its constant pressure controls
        // one; neighbouring elements restrain the other three (P2–P0, crate::hybrid). Both are
        // skipped.
        if f != Formulation::Reduced && !(kind == ElementKind::Tet10 && matches!(f, Formulation::Hybrid { .. })) {
            let mut held = k.clone();
            for dof in [0, 1, 2, 4, 5, 8] {
                held.add(dof, dof, 1e3 * scale);
            }
            assert!(Cholesky::new(&held).is_ok(), "{kind:?} {f:?} has a spurious zero-energy mode");
        }
    }
}

#[test]
fn constant_strain_is_reproduced_exactly() {
    // u = A x for a general A: every integration point sees ε(A), the stiffness's internal
    // forces balance (K u is the boundary load of a uniform stress), and the incompatible modes
    // stay idle.
    let a = [[1e-3, 2e-4, -3e-4], [5e-4, -2e-3, 1e-4], [-1e-4, 3e-4, 7e-4]];
    let strain = [a[0][0], a[1][1], a[2][2], a[0][1] + a[1][0], a[0][2] + a[2][0], a[1][2] + a[2][1]];
    for d in [isotropic(210e3, 0.3).unwrap(), orthotropic([140e3, 10e3, 12e3], [0.3, 0.25, 0.4], [5e3, 4e3, 3.8e3]).unwrap()] {
        let want = stress(&d, &strain);
        // Hybrid elements need an isotropic material.
        let hybrid_ok = crate::hybrid::split(&d).is_ok();
        for (kind, f) in CASES.into_iter().filter(|(_, f)| hybrid_ok || !matches!(f, Formulation::Hybrid { .. })) {
            // Curved mid-sides keep the field in the isoparametric span only for straight edges.
            let xyz = distorted(kind, false);
            let u: Vec<[f64; 3]> = xyz.iter().map(|x| [0, 1, 2].map(|i| (0..3).map(|j| a[i][j] * x[j]).sum())).collect();
            let e = Solid::new(kind, &xyz, f).unwrap();
            for p in e.stresses(&d, &u).unwrap() {
                for k in 0..6 {
                    assert!((p.strain[k] - strain[k]).abs() < 1e-14, "{kind:?} {f:?} strain {k}");
                    assert!((p.stress[k] - want[k]).abs() < 1e-9 * want[0].abs().max(1.0), "{kind:?} {f:?} stress {k}");
                }
            }
            let nodal = e.extrapolate(&e.stresses(&d, &u).unwrap().iter().map(|p| p.stress).collect::<Vec<_>>()).unwrap();
            assert_eq!(nodal.len(), xyz.len());
            assert!(nodal.iter().all(|s| (0..6).all(|k| (s[k] - want[k]).abs() < 1e-9 * want[0].abs())), "{kind:?} {f:?}");
        }
    }
}

#[test]
fn mass_volume_and_body_force_totals() {
    let rho = 7.85e-9;
    for (kind, f) in CASES {
        let xyz = distorted(kind, true);
        let e = Solid::new(kind, &xyz, f).unwrap();
        let v = e.volume().unwrap();
        assert!(v > 0.0);
        let m = e.mass(rho).unwrap();
        let total: f64 = m.data.iter().sum();
        assert!((total - rho * v).abs() < 1e-12 * rho * v, "{kind:?}: {total} vs {}", rho * v);
        let lumped = e.lumped_mass(rho).unwrap();
        assert!(lumped.iter().all(|x| *x > 0.0), "{kind:?}: lumped masses are positive");
        assert!((lumped.iter().sum::<f64>() - rho * v).abs() < 1e-12 * rho * v);
        let b = e.body_force([0.0, 0.0, -9.81]).unwrap();
        assert!((b.iter().map(|f| f[2]).sum::<f64>() + 9.81 * v).abs() < 1e-12 * v);
    }
    // A tetrahedron's volume is a sixth of its edge triple product.
    let xyz = distorted(ElementKind::Tet4, false);
    let e = |i: usize| [0, 1, 2].map(|k| xyz[i][k] - xyz[0][k]);
    let (a, b, c) = (e(1), e(2), e(3));
    let exact = (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0;
    let v = Solid::new(ElementKind::Tet4, &xyz, Formulation::Full).unwrap().volume().unwrap();
    assert!((v - exact).abs() < 1e-15);
}

#[test]
fn extrapolation_reproduces_linear_fields_on_affine_elements() {
    // Values linear in x at the integration points come back exactly at the nodes (tet4 has
    // one point and gives the mean).
    let f = |x: [f64; 3]| 1.0 + 2.0 * x[0] - 0.5 * x[1] + 3.0 * x[2];
    let affine = |kind: ElementKind| -> Vec<[f64; 3]> {
        let mut xyz: Vec<[f64; 3]> = match kind {
            ElementKind::Tet10 => vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            ElementKind::Wedge6 => vec![[0., 0., 0.], [2., 0., 0.], [0.5, 1., 0.], [0., 0.2, 1.], [2., 0.2, 1.], [0.5, 1.2, 1.]],
            _ => vec![[0., 0., 0.], [2., 0., 0.], [2.5, 1., 0.], [0.5, 1., 0.], [0., 0., 1.], [2., 0., 1.], [2.5, 1., 1.], [0.5, 1., 1.]],
        };
        let c = xyz.clone();
        for e in kind.edges() {
            if let [a, b, _] = **e {
                xyz.push([0, 1, 2].map(|k| (c[a][k] + c[b][k]) / 2.0));
            }
        }
        xyz
    };
    for (kind, form) in CASES.into_iter().filter(|(k, _)| *k != ElementKind::Tet4) {
        let xyz = affine(kind);
        let e = Solid::new(kind, &xyz, form).unwrap();
        let values: Vec<[f64; 6]> = e.integration_points().iter().map(|p| [f(e.position(p.xi)); 6]).collect();
        let nodal = e.extrapolate(&values).unwrap();
        for (x, v) in xyz.iter().zip(&nodal) {
            assert!((v[0] - f(*x)).abs() < 1e-12, "{kind:?} {form:?} at {x:?}: {} vs {}", v[0], f(*x));
        }
    }
}

#[test]
fn bad_input_is_an_error() {
    let xyz = distorted(ElementKind::Hex8, false);
    assert!(Solid::new(ElementKind::Hex8, &xyz[..7], Formulation::Full).is_err());
    assert!(Solid::new(ElementKind::Hex8, &xyz, Formulation::Reduced).is_err());
    assert!(Solid::new(ElementKind::Tet4, &xyz[..4], Formulation::IncompatibleModes).is_err());
    assert!(Solid::new(ElementKind::Quad4, &xyz[..4], Formulation::Full).is_err());
    assert!(Solid::new(ElementKind::Wedge6, &xyz[..6], Formulation::IncompatibleModes).is_err());
    let mut nan = xyz.clone();
    nan[3][1] = f64::NAN;
    assert!(Solid::new(ElementKind::Hex8, &nan, Formulation::Full).is_err());
    // An inverted element (top and bottom swapped) and a flat one.
    let mut inverted = xyz.clone();
    inverted.rotate_left(4);
    let d = isotropic(1.0, 0.3).unwrap();
    assert!(Solid::new(ElementKind::Hex8, &inverted, Formulation::Full).unwrap().stiffness(&d).is_err());
    let flat: Vec<[f64; 3]> = xyz.iter().map(|p| [p[0], p[1], 0.0]).collect();
    assert!(Solid::new(ElementKind::Hex8, &flat, Formulation::IncompatibleModes).unwrap().stiffness(&d).is_err());
    let e = Solid::new(ElementKind::Hex8, &xyz, Formulation::Full).unwrap();
    assert!(e.stresses(&d, &[[0.0; 3]; 3]).is_err());
    assert!(e.extrapolate(&[[0.0; 6]; 3]).is_err());
    let _ = Matrix::zeros(0).mul_vec(&[]);
}

#[test]
fn section_formulations() {
    use modelcraft_model::{Formulation as F, Integration};
    let full = F::default();
    let reduced = F { integration: Integration::Reduced, ..F::default() };
    let im = F { incompatible_modes: true, ..F::default() };
    let hybrid = F { hybrid: true, ..F::default() };
    assert_eq!(Formulation::from_section(ElementKind::Hex8, &full), Ok(Formulation::Full));
    assert_eq!(Formulation::from_section(ElementKind::Hex8, &im), Ok(Formulation::IncompatibleModes));
    assert_eq!(Formulation::from_section(ElementKind::Hex20, &reduced), Ok(Formulation::Reduced));
    assert!(Formulation::from_section(ElementKind::Hex8, &reduced).unwrap_err().to_string().contains("not supported yet"));
    assert!(Formulation::from_section(ElementKind::Tet10, &im).is_err());
    assert!(Formulation::from_section(ElementKind::Tet4, &hybrid).is_err());
    assert!(Formulation::from_section(ElementKind::Quad4, &full).is_err());
}

#[test]
fn uniform_thermal_strain_is_free_expansion_and_its_load_balances_it() {
    // A uniform ΔT with orthotropic α: the free expansion u = diag(α ΔT) x is stress-free, and
    // the thermal load equals K u for it (the element is in equilibrium with no other load).
    let ortho = orthotropic([1000.0, 800.0, 600.0], [0.3, 0.25, 0.2], [400.0, 300.0, 350.0]).unwrap();
    let iso = isotropic(1000.0, 0.4999).unwrap();
    let alpha = [1.2e-5, 2.0e-5, 0.7e-5];
    for (kind, f) in CASES {
        // Hybrid elements take isotropic (here nearly incompressible) materials.
        let d = if matches!(f, Formulation::Hybrid { .. }) { iso } else { ortho };
        let xyz = distorted(kind, true);
        let solid = Solid::new(kind, &xyz, f).unwrap();
        let th = crate::ThermalStrain { alpha, delta: vec![80.0; xyz.len()] };
        let u: Vec<[f64; 3]> = xyz.iter().map(|x| [0, 1, 2].map(|k| alpha[k] * 80.0 * x[k])).collect();
        for p in solid.stresses_with(&d, &u, Some(&th)).unwrap() {
            assert!(p.stress.iter().all(|s| s.abs() < 1e-9), "{kind:?} {f:?}: {:?}", p.stress);
            for k in 0..3 {
                assert!((p.strain[k] - alpha[k] * 80.0).abs() < 1e-12);
            }
        }
        let load = solid.thermal_load(&d, &th).unwrap();
        let ku = solid.stiffness(&d).unwrap().mul_vec(&u.iter().flatten().copied().collect::<Vec<_>>());
        let scale = ku.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        for (a, b) in load.iter().flatten().zip(&ku) {
            assert!((a - b).abs() < 1e-9 * scale, "{kind:?} {f:?}: {a} vs {b}");
        }
        // Without a thermal strain the two stress paths agree bit for bit.
        assert_eq!(solid.stresses_with(&d, &u, None).unwrap(), solid.stresses(&d, &u).unwrap());
        assert!(solid.thermal_load(&d, &crate::ThermalStrain { alpha, delta: vec![1.0; 2] }).is_err());
        assert!(solid.thermal_load(&d, &crate::ThermalStrain { alpha: [f64::NAN; 3], delta: vec![1.0; xyz.len()] }).is_err());
    }
}
