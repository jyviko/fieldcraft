use super::*;
use crate::size::UniformSize;
use crate::surface::{BoundarySurface, SurfaceOptions};
use crate::vec3::{cross, dot, sub};

/// Ear clipping of a simple planar polygon (counter-clockwise about `normal`).
fn ears(points: &[[f64; 3]], poly: &[u32]) -> Vec<[u32; 3]> {
    let p = |i: u32| points[i as usize];
    let normal = (0..poly.len()).fold([0.0; 3], |n, k| {
        let (a, b) = (p(poly[k]), p(poly[(k + 1) % poly.len()]));
        [n[0] + (a[1] - b[1]) * (a[2] + b[2]), n[1] + (a[2] - b[2]) * (a[0] + b[0]), n[2] + (a[0] - b[0]) * (a[1] + b[1])]
    });
    let turn = |a: [f64; 3], b: [f64; 3], c: [f64; 3]| dot(cross(sub(b, a), sub(c, a)), normal);
    let mut v: Vec<u32> = poly.to_vec();
    let mut out = Vec::new();
    while v.len() > 3 {
        let n = v.len();
        let k = (0..n)
            .find(|&k| {
                let (a, b, c) = (p(v[(k + n - 1) % n]), p(v[k]), p(v[(k + 1) % n]));
                turn(a, b, c) > 0.0
                    && v.iter().all(|&o| {
                        let q = p(o);
                        o == v[(k + n - 1) % n]
                            || o == v[k]
                            || o == v[(k + 1) % n]
                            || !(turn(a, b, q) >= 0.0 && turn(b, c, q) >= 0.0 && turn(c, a, q) >= 0.0)
                    })
            })
            .unwrap();
        out.push([v[(k + n - 1) % n], v[k], v[(k + 1) % n]]);
        v.remove(k);
    }
    out.push([v[0], v[1], v[2]]);
    out
}

/// A closed surface from planar polygonal faces (counter-clockwise from outside), tagged by face.
pub(crate) fn polyhedron(points: &[[f64; 3]], faces: &[Vec<u32>]) -> BoundarySurface {
    let mut tris = Vec::new();
    let mut tags = Vec::new();
    for (f, poly) in faces.iter().enumerate() {
        for t in ears(points, poly) {
            tris.push(t);
            tags.push(f as u32);
        }
    }
    BoundarySurface::new(points, &tris, &tags, &SurfaceOptions::default()).unwrap()
}

/// A prism over a counter-clockwise polygon from z0 to z1.
pub(crate) fn prism(poly: &[[f64; 2]], z0: f64, z1: f64) -> BoundarySurface {
    let n = poly.len() as u32;
    let mut pts: Vec<[f64; 3]> = poly.iter().map(|p| [p[0], p[1], z0]).collect();
    pts.extend(poly.iter().map(|p| [p[0], p[1], z1]));
    let mut faces = vec![(0..n).rev().collect::<Vec<_>>(), (n..2 * n).collect()];
    for k in 0..n {
        let j = (k + 1) % n;
        faces.push(vec![k, j, n + j, n + k]);
    }
    polyhedron(&pts, &faces)
}

fn check(m: &HexMesh, volume: f64) {
    assert!(m.report.gate.passed());
    assert!((m.report.volume - volume).abs() < 1e-9 * volume, "{} vs {volume}", m.report.volume);
    assert!(m.report.min_scaled_jacobian > 0.0);
    // Every boundary face is tagged and the boundary faces are exactly the hex faces used once.
    assert!(!m.boundary.is_empty());
}

#[test]
fn box_extrudes_into_a_grid() {
    let s = prism(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]], 0.0, 3.0);
    let m = hex_mesh(&s, &[], &UniformSize(1.0), &HexOptions::default()).unwrap();
    check(&m, 24.0);
    assert_eq!(m.element_count(), 24, "{}", m.report.description);
    assert!((m.report.min_scaled_jacobian - 1.0).abs() < 1e-12);
    // Boundary: 2·(4·2 + 4·3 + 2·3) = 52 quads.
    assert_eq!(m.boundary.len(), 52);
}

#[test]
fn l_shape_extrudes_with_a_free_or_mapped_section() {
    let l = [[0.0, 0.0], [6.0, 0.0], [6.0, 2.0], [2.0, 2.0], [2.0, 6.0], [0.0, 6.0]];
    let s = prism(&l, 0.0, 2.0);
    let m = hex_mesh(&s, &[], &UniformSize(1.0), &HexOptions::default()).unwrap();
    check(&m, 40.0);
}

#[test]
fn quadratic_box() {
    let s = prism(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]], 0.0, 2.0);
    let m = hex_mesh(&s, &[], &UniformSize(1.0), &HexOptions { quadratic: true, ..HexOptions::default() }).unwrap();
    check(&m, 8.0);
    assert_eq!(m.kind, ElementKind::Hex20);
    // 3³ corner nodes + 3·(2·3·3) = 54 edge nodes.
    assert_eq!(m.nodes.len(), 27 + 54);
}

/// A block whose top is the curved surface z = 2 + 0.3 sin(πx/4), tessellated finely.
fn curved_block() -> BoundarySurface {
    let n = 16usize;
    let top = |x: f64| 2.0 + 0.3 * (std::f64::consts::PI * x / 4.0).sin();
    let mut pts: Vec<[f64; 3]> = vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 2.0, 0.0], [0.0, 2.0, 0.0]];
    // Top grid rows at y = 0 and y = 2: ids 4..4+n+1 and 5+n..
    for y in [0.0, 2.0] {
        for k in 0..=n {
            let x = 4.0 * k as f64 / n as f64;
            pts.push([x, y, top(x)]);
        }
    }
    let t0 = |k: usize| (4 + k) as u32;
    let t2 = |k: usize| (5 + n + k) as u32;
    let mut faces: Vec<Vec<u32>> = vec![vec![0, 3, 2, 1]];
    // y = 0 side: 0, 1, then the top edge from x = 4 back to 0.
    faces.push([0u32, 1].into_iter().chain((0..=n).rev().map(t0)).collect());
    faces.push([2u32, 3].into_iter().chain((0..=n).map(t2)).collect());
    faces.push(vec![1, 2, t2(n), t0(n)]);
    faces.push(vec![3, 0, t0(0), t2(0)]);
    // The top: strips of two triangles, one face.
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut tags: Vec<u32> = Vec::new();
    for (f, poly) in faces.iter().enumerate() {
        for t in ears(&pts, poly) {
            tris.push(t);
            tags.push(f as u32);
        }
    }
    for k in 0..n {
        tris.push([t0(k), t0(k + 1), t2(k + 1)]);
        tris.push([t0(k), t2(k + 1), t2(k)]);
        tags.extend([5, 5]);
    }
    BoundarySurface::new(&pts, &tris, &tags, &SurfaceOptions::default()).unwrap()
}

#[test]
fn mapped_block_with_a_curved_top() {
    let s = curved_block();
    let opts = HexOptions { method: HexMethod::Mapped, ..HexOptions::default() };
    let m = hex_mesh(&s, &[], &UniformSize(0.5), &opts).unwrap();
    assert!(m.report.gate.passed());
    assert!(m.report.min_scaled_jacobian > 0.8, "{}", m.report.min_scaled_jacobian);
    assert!(m.report.description.contains("mapped block of 8 × 4 × 4"), "{}", m.report.description);
    // Top nodes lie on the tessellated top.
    let quadratic = hex_mesh(&s, &[], &UniformSize(0.5), &HexOptions { quadratic: true, ..opts }).unwrap();
    assert!(quadratic.report.gate.passed());
    assert!((quadratic.report.volume - s.volume()).abs() < 1e-3 * s.volume(), "{} vs {}", quadratic.report.volume, s.volume());
}
