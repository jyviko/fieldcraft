//! Analytic tests: every expected value is closed-form.

use modelcraft_model::{ElementKind, Model};

use super::*;

const UNIT_CUBE: [[f64; 3]; 8] = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]];
const RIGHT_TET: [[f64; 3]; 4] = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * (1.0 + b.abs())
}

fn regular_tet() -> [[f64; 3]; 4] {
    let s = 1.0 / 2.0_f64.sqrt();
    // Alternate cube corners: edge √2·(1/√2) = 1.
    [[0., 0., 0.], [s, 0., s], [s, s, 0.], [0., s, s]]
}

fn metric(q: &ElementQuality, m: Metric) -> f64 {
    q.get(m).unwrap()
}

/// Quadratic element nodes from corner nodes with straight edges.
fn midpoints(corners: &[[f64; 3]], edges: &[(usize, usize)]) -> Vec<[f64; 3]> {
    let mut out = corners.to_vec();
    for &(a, b) in edges {
        out.push([0.5 * (corners[a][0] + corners[b][0]), 0.5 * (corners[a][1] + corners[b][1]), 0.5 * (corners[a][2] + corners[b][2])]);
    }
    out
}

#[test]
fn regular_tetrahedron_scores_one() {
    let p = regular_tet();
    let q = element_quality(ElementKind::Tet4, &p).unwrap();
    assert!(close(q.size, 1.0 / (6.0 * 2.0_f64.sqrt())), "{}", q.size);
    assert_eq!(q.validity(), Some(Validity::Valid));
    let dihedral = (1.0_f64 / 3.0).acos().to_degrees();
    assert!(close(metric(&q, Metric::MinDihedral), dihedral));
    assert!(close(metric(&q, Metric::MaxDihedral), dihedral));
    for m in [Metric::RadiusRatio, Metric::AspectRatio, Metric::VolumeLength, Metric::ScaledJacobian, Metric::JacobianRatio] {
        assert!(close(metric(&q, m), 1.0), "{m:?} = {}", metric(&q, m));
    }
}

#[test]
fn right_corner_tetrahedron() {
    let q = element_quality(ElementKind::Tet4, &RIGHT_TET).unwrap();
    assert!(close(q.size, 1.0 / 6.0));
    assert!(close(metric(&q, Metric::MinDihedral), (1.0 / 3.0_f64.sqrt()).acos().to_degrees()));
    assert!(close(metric(&q, Metric::MaxDihedral), 90.0));
    // Inradius 1/(3 + √3), circumradius √3/2.
    let r = 1.0 / (3.0 + 3.0_f64.sqrt());
    assert!(close(metric(&q, Metric::RadiusRatio), 3.0 * r / (3.0_f64.sqrt() / 2.0)));
    // Corner at (1,0,0): edges 1, √2, √2 → 6V/2 · √2.
    assert!(close(metric(&q, Metric::ScaledJacobian), 2.0_f64.sqrt() / 2.0));
    // ℓ_rms² = (3·1 + 3·2)/6 = 3/2.
    assert!(close(metric(&q, Metric::VolumeLength), 2.0_f64.sqrt() / 1.5_f64.powf(1.5)));
}

#[test]
fn inverted_and_flat_tetrahedra() {
    let mut p = regular_tet();
    p.swap(1, 2);
    let q = element_quality(ElementKind::Tet4, &p).unwrap();
    assert!(q.size < 0.0);
    assert_eq!(q.validity(), Some(Validity::Invalid));
    assert!(close(metric(&q, Metric::ScaledJacobian), -1.0));
    assert!(close(metric(&q, Metric::RadiusRatio), -1.0));
    assert!(close(metric(&q, Metric::JacobianRatio), -1.0));

    let flat = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]];
    let q = element_quality(ElementKind::Tet4, &flat).unwrap();
    assert_eq!(q.size, 0.0);
    assert_eq!(q.validity(), Some(Validity::Invalid));
    assert_eq!(metric(&q, Metric::RadiusRatio), 0.0);
    assert_eq!(metric(&q, Metric::AspectRatio), f64::INFINITY);
    assert!(q.metrics.iter().all(|(_, v)| !v.is_nan()));

    // A sliver with an exactly positive orientation is valid, however thin (exact orient3d).
    let sliver = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0.3, 0.3, 1e-30]];
    assert_eq!(element_validity(ElementKind::Tet4, &sliver).unwrap().validity, Validity::Valid);
    let below = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0.3, 0.3, -1e-30]];
    assert_eq!(element_validity(ElementKind::Tet4, &below).unwrap().validity, Validity::Invalid);

    // All four nodes at one point.
    let q = element_quality(ElementKind::Tet4, &[[1., 2., 3.]; 4]).unwrap();
    assert_eq!(q.validity(), Some(Validity::Invalid));
    assert!(q.metrics.iter().all(|(_, v)| !v.is_nan()));
}

#[test]
fn unit_cube_scores_one() {
    for (kind, nodes) in [(ElementKind::Hex8, UNIT_CUBE.to_vec()), (ElementKind::Hex20, midpoints(&UNIT_CUBE, &jacobian::HEX_EDGES))] {
        let q = element_quality(kind, &nodes).unwrap();
        assert!(close(q.size, 1.0), "{kind:?} {}", q.size);
        assert_eq!(q.validity(), Some(Validity::Valid));
        for m in [Metric::ScaledJacobian, Metric::JacobianRatio, Metric::Shape, Metric::EdgeRatio] {
            assert!(close(metric(&q, m), 1.0), "{kind:?} {m:?} = {}", metric(&q, m));
        }
    }
}

#[test]
fn sheared_hexahedron() {
    // x += s·z: a parallelepiped, so det J is constant and the volume stays 1.
    let s = 0.75;
    let p: Vec<[f64; 3]> = UNIT_CUBE.iter().map(|c| [c[0] + s * c[2], c[1], c[2]]).collect();
    let q = element_quality(ElementKind::Hex8, &p).unwrap();
    assert!(close(q.size, 1.0));
    assert_eq!(q.validity(), Some(Validity::Valid));
    assert!(close(metric(&q, Metric::JacobianRatio), 1.0));
    assert!(close(metric(&q, Metric::ScaledJacobian), 1.0 / (1.0 + s * s).sqrt()));
}

#[test]
fn mirrored_hexahedron_is_inverted() {
    let p: Vec<[f64; 3]> = UNIT_CUBE.iter().map(|c| [-c[0], c[1], c[2]]).collect();
    let q = element_quality(ElementKind::Hex8, &p).unwrap();
    assert!(close(q.size, -1.0));
    assert_eq!(q.validity(), Some(Validity::Invalid));
    assert!(close(metric(&q, Metric::ScaledJacobian), -1.0));
    assert_eq!(metric(&q, Metric::Shape), 0.0);
}

/// The top face is the bottom face mapped by A = diag(−2, −3) (centred on the axis). The
/// trilinear map is x = ((1−w) I + w A) p, z = w, so det J = (1 − 3w)(1 − 4w): 1 at the bottom
/// corners, 6 at the top corners, 1/2 at the centre, −1/48 at w = 7/24, and the volume is
/// ∫(1 − 7w + 12w²) = 3/2.
fn inside_out_hex() -> Vec<[f64; 3]> {
    let bottom = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]];
    let mut p: Vec<[f64; 3]> = bottom.iter().map(|b| [b[0], b[1], 0.0]).collect();
    p.extend(bottom.iter().map(|b| [-2.0 * b[0], -3.0 * b[1], 1.0]));
    p
}

#[test]
fn hex_with_positive_corners_inverted_inside_is_invalid() {
    let p = inside_out_hex();
    // Every corner Jacobian (three edge vectors in right-handed order) is positive.
    let e = |a: usize, b: usize| shape::sub(p[b], p[a]);
    for (o, x, y, z) in [(0, 1, 3, 4), (1, 2, 0, 5), (2, 3, 1, 6), (3, 0, 2, 7), (4, 7, 5, 0), (5, 4, 6, 1), (6, 5, 7, 2), (7, 6, 4, 3)] {
        assert!(shape::det(e(o, x), e(o, y), e(o, z)) > 0.0, "corner {o}");
    }
    let q = element_quality(ElementKind::Hex8, &p).unwrap();
    assert!(metric(&q, Metric::ScaledJacobian) > 0.0, "corners and centre look fine");
    assert!(close(q.size, 1.5));
    assert_eq!(q.validity(), Some(Validity::Invalid));
    assert!(metric(&q, Metric::JacobianRatio) < 0.0);
}

#[test]
fn quarter_point_hex20_is_degenerate() {
    // Move the mid-side node of edge 1-2 (node 9) along the edge to parameter m. On that edge
    // x'(0) = 4m − 1, so the element is valid for 1/4 < m < 3/4 and degenerate at m = 1/4.
    for (m, valid) in [(0.5, true), (0.3, true), (0.7, true), (0.25, false), (0.2, false), (0.8, false)] {
        let mut p = midpoints(&UNIT_CUBE, &jacobian::HEX_EDGES);
        p[8] = [m, 0.0, 0.0];
        let check = element_validity(ElementKind::Hex20, &p).unwrap();
        assert_eq!(check.validity == Validity::Valid, valid, "m = {m}: {check:?}");
        assert!(check.validity != Validity::Undetermined);
    }
}

#[test]
fn tet10_with_a_moved_mid_side_node() {
    // Moving node 5 (edge 1-2) to parameter m gives det J = det_lin (1 + 4(m − ½)(λ1 − λ2)),
    // extreme at the two edge ends: valid iff |m − ½| < ¼, ratio (1 − 4d)/(1 + 4d).
    for m in [0.5, 0.3, 0.6, 0.25, 0.1, 0.9] {
        let mut p = midpoints(&RIGHT_TET, &jacobian::TET_EDGES);
        p[4] = [m, 0.0, 0.0];
        let q = element_quality(ElementKind::Tet10, &p).unwrap();
        let d: f64 = (m - 0.5_f64).abs();
        let valid = d < 0.25;
        assert_eq!(q.validity() == Some(Validity::Valid), valid, "m = {m}");
        if valid {
            assert!(close(metric(&q, Metric::JacobianRatio), (1.0 - 4.0 * d) / (1.0 + 4.0 * d)), "m = {m}");
        }
        // The node moves along a straight edge, so the volume is unchanged.
        assert!(close(q.size, 1.0 / 6.0), "m = {m}: {}", q.size);
    }
}

#[test]
fn wedges() {
    let right = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [0., 1., 1.]];
    let q = element_quality(ElementKind::Wedge6, &right).unwrap();
    assert!(close(q.size, 0.5));
    assert_eq!(q.validity(), Some(Validity::Valid));
    assert!(close(metric(&q, Metric::ScaledJacobian), 0.5_f64.sqrt()));
    assert!(close(metric(&q, Metric::JacobianRatio), 1.0));
    assert!(close(metric(&q, Metric::EdgeRatio), 2.0_f64.sqrt()));

    let h = 3.0_f64.sqrt() / 2.0;
    let equilateral = [[0., 0., 0.], [1., 0., 0.], [0.5, h, 0.], [0., 0., 1.], [1., 0., 1.], [0.5, h, 1.]];
    let q = element_quality(ElementKind::Wedge6, &equilateral).unwrap();
    assert!(close(metric(&q, Metric::ScaledJacobian), h));

    let upside_down: Vec<[f64; 3]> = right.iter().map(|p| [p[0], p[1], -p[2]]).collect();
    let q = element_quality(ElementKind::Wedge6, &upside_down).unwrap();
    assert!(close(q.size, -0.5));
    assert_eq!(q.validity(), Some(Validity::Invalid));
}

#[test]
fn shells() {
    let h = 3.0_f64.sqrt() / 2.0;
    let q = element_quality(ElementKind::Tri3, &[[0., 0., 0.], [1., 0., 0.], [0.5, h, 0.]]).unwrap();
    assert!(close(q.size, h / 2.0));
    assert!(close(metric(&q, Metric::MinAngle), 60.0) && close(metric(&q, Metric::MaxAngle), 60.0));
    assert!(close(metric(&q, Metric::AspectRatio), 1.0));
    assert_eq!(q.validity(), None);

    let square = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
    let q = element_quality(ElementKind::Quad4, &square).unwrap();
    assert!(close(q.size, 1.0));
    assert!(close(metric(&q, Metric::ScaledJacobian), 1.0));
    assert!(close(metric(&q, Metric::MinAngle), 90.0));
    // A concave quadrilateral: corner 3 pushed past the diagonal.
    let dart = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0.7, 0.3, 0.]];
    assert!(metric(&element_quality(ElementKind::Quad4, &dart).unwrap(), Metric::ScaledJacobian) < 0.0);

    assert!(element_quality(ElementKind::Rod2, &[[0.; 3], [1., 0., 0.]]).is_err());
}

#[test]
fn hostile_coordinates_are_errors() {
    let mut p = UNIT_CUBE;
    p[3][1] = f64::NAN;
    assert!(element_quality(ElementKind::Hex8, &p).is_err());
    p[3][1] = 1e300;
    assert!(element_validity(ElementKind::Hex8, &p).is_err());
    assert!(element_quality(ElementKind::Hex8, &UNIT_CUBE[..7]).is_err());
    assert!(element_validity(ElementKind::Tri3, &[[0.; 3]; 3]).is_err());
    // Far from the origin and at the coordinate limit: still exact.
    let far: Vec<[f64; 3]> = UNIT_CUBE.iter().map(|c| [c[0] * 1e9, c[1] * 1e9, c[2] * 1e9 - 1e9]).collect();
    let q = element_quality(ElementKind::Hex8, &far).unwrap();
    assert_eq!(q.validity(), Some(Validity::Valid));
    assert!(close(q.size, 1e27));
}

fn model_of(elements: &[(ElementKind, Vec<[f64; 3]>)]) -> Model {
    let mut m = Model::default();
    let mut next = 1;
    for (e, (kind, nodes)) in elements.iter().enumerate() {
        let mut labels = Vec::new();
        for p in nodes {
            m.set_node(next, *p).unwrap();
            labels.push(next);
            next += 1;
        }
        m.set_element(e as u32 + 1, *kind, labels).unwrap();
    }
    m
}

#[test]
fn report_and_gate() {
    let mut flipped = UNIT_CUBE.to_vec();
    flipped.iter_mut().for_each(|p| p[0] = -p[0] - 2.0);
    let model = model_of(&[
        (ElementKind::Hex8, UNIT_CUBE.to_vec()),
        (ElementKind::Hex8, inside_out_hex()),
        (ElementKind::Tet4, regular_tet().to_vec()),
        (ElementKind::Hex8, flipped),
        (ElementKind::Rod2, vec![[0.; 3], [1., 0., 0.]]),
    ]);
    let r = quality_report(&model, None, 4).unwrap();
    assert_eq!(r.measured, 4);
    assert_eq!(r.skipped, vec![(ElementKind::Rod2, 1)]);
    assert_eq!((r.valid, r.invalid.clone()), (2, vec![2, 4]));
    let sj = r.metrics.iter().find(|s| s.metric == Metric::ScaledJacobian).unwrap();
    assert_eq!((sj.count, sj.worst, sj.min, sj.histogram.iter().sum::<usize>()), (4, 4, -1.0, 4));
    assert_eq!(sj.histogram[0], 1);
    assert_eq!(r.metrics.iter().find(|s| s.metric == Metric::RadiusRatio).unwrap().count, 1);
    assert!(quality_report(&model, None, 0).is_err());
    assert!(quality_report(&model, Some(&[99]), 4).is_err());
    assert_eq!(quality_report(&model, Some(&[1, 1, 3]), 4).unwrap().measured, 2);

    let gate = validity_gate(&model, &GateOptions::default()).unwrap();
    assert_eq!((gate.checked, gate.invalid.clone()), (4, vec![2, 4]));
    assert!(gate.into_result().is_err());
    // The same gate straight from a mesher's arrays.
    let raw = validity_gate_elements([(7, ElementKind::Hex8, inside_out_hex()), (8, ElementKind::Hex8, UNIT_CUBE.to_vec())], &GateOptions::default())
        .unwrap();
    assert_eq!((raw.checked, raw.invalid.clone()), (2, vec![7]));
    // And from checks a mesher already made with element_quality.
    let checks: Vec<(u32, JacobianCheck)> = [(7, inside_out_hex()), (8, UNIT_CUBE.to_vec())]
        .into_iter()
        .map(|(l, x)| (l, element_quality(ElementKind::Hex8, &x).unwrap().jacobian.unwrap()))
        .collect();
    assert_eq!(validity_gate_checks(checks.iter().map(|(l, c)| (*l, c)), &GateOptions::default()).unwrap(), raw);

    // A unit cube split into six tetrahedra around the 1-7 diagonal passes against its boundary.
    let c = UNIT_CUBE;
    let tets = [[0, 1, 2, 6], [0, 2, 3, 6], [0, 3, 7, 6], [0, 7, 4, 6], [0, 4, 5, 6], [0, 5, 1, 6]];
    let model = model_of(&tets.map(|t| (ElementKind::Tet4, t.iter().map(|&i| c[i]).collect())));
    let faces = [[0, 3, 2], [0, 2, 1], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]];
    let boundary = enclosed_volume(&c, &faces).unwrap();
    assert!(close(boundary, 1.0));
    let ok = validity_gate(&model, &GateOptions { expected_volume: Some(boundary), ..Default::default() }).unwrap();
    assert!(ok.passed(), "{ok:?}");
    let short = validity_gate(&model, &GateOptions { expected_volume: Some(1.1), ..Default::default() }).unwrap();
    assert!(!short.volume_matches() && short.into_result().is_err());
    assert!(validity_gate(&model, &GateOptions { expected_volume: Some(f64::NAN), ..Default::default() }).is_err());
    assert!(enclosed_volume(&c, &[[0, 1, 99]]).is_err());
}
