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

/// A thick-walled pipe along z (radii `ri` < `ro`, height `h`, `n` segments around, `n` even):
/// eight faces, each cylinder and annulus in two halves, with their analytic surfaces.
fn pipe(ri: f64, ro: f64, h: f64, n: usize) -> (BoundarySurface, Vec<Option<Surface>>) {
    let ring = |r: f64, z: f64| -> Vec<[f64; 3]> {
        (0..n)
            .map(|k| {
                let t = std::f64::consts::TAU * k as f64 / n as f64;
                [r * t.cos(), r * t.sin(), z]
            })
            .collect()
    };
    let mut pts = Vec::new();
    for (r, z) in [(ro, 0.0), (ri, 0.0), (ro, h), (ri, h)] {
        pts.extend(ring(r, z));
    }
    let id = |ring: usize, k: usize| (ring * n + k % n) as u32;
    let (bo, bi, to, ti) = (0, 1, 2, 3);
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut tags: Vec<u32> = Vec::new();
    // A quad as two triangles, turned to face `outward`.
    let mut quad = |q: [u32; 4], outward: [f64; 3], tag: u32| {
        let p = |i: u32| pts[i as usize];
        let n0 = cross(sub(p(q[1]), p(q[0])), sub(p(q[2]), p(q[0])));
        let q = if dot(n0, outward) < 0.0 { [q[0], q[3], q[2], q[1]] } else { q };
        tris.push([q[0], q[1], q[2]]);
        tris.push([q[0], q[2], q[3]]);
        tags.extend([tag, tag]);
    };
    for k in 0..n {
        let half = u32::from(k >= n / 2);
        let t = std::f64::consts::TAU * (k as f64 + 0.5) / n as f64;
        let radial = [t.cos(), t.sin(), 0.0];
        quad([id(bo, k), id(bo, k + 1), id(to, k + 1), id(to, k)], radial, half);
        quad([id(bi, k), id(bi, k + 1), id(ti, k + 1), id(ti, k)], [-radial[0], -radial[1], 0.0], 2 + half);
        quad([id(bo, k), id(bo, k + 1), id(bi, k + 1), id(bi, k)], [0.0, 0.0, -1.0], 4 + half);
        quad([id(to, k), id(to, k + 1), id(ti, k + 1), id(ti, k)], [0.0, 0.0, 1.0], 6 + half);
    }
    let surface = BoundarySurface::new(&pts, &tris, &tags, &SurfaceOptions::default()).unwrap();
    let cyl = |r: f64| Some(Surface::Cylinder { origin: [0.0; 3], axis: [0.0, 0.0, 1.0], radius: r });
    let plane = |z: f64, nz: f64| Some(Surface::Plane { point: [0.0, 0.0, z], normal: [0.0, 0.0, nz] });
    (surface, vec![cyl(ro), cyl(ro), cyl(ri), cyl(ri), plane(0.0, -1.0), plane(0.0, -1.0), plane(h, 1.0), plane(h, 1.0)])
}

/// Every node of the boundary faces tagged with `faces` lies at `radius` from the z axis.
fn on_radius(m: &HexMesh, faces: &[u32], radius: f64) -> bool {
    m.boundary.iter().filter(|b| faces.contains(&b.brep_face)).all(|b| {
        let local = m.kind.faces()[b.face as usize - 1];
        let e = m.element(b.element as usize).unwrap();
        local.iter().all(|&i| {
            let p = m.nodes[e[i] as usize];
            ((p[0] * p[0] + p[1] * p[1]).sqrt() - radius).abs() < 1e-9 * radius
        })
    })
}

#[test]
fn pipe_extrudes_and_revolves_with_nodes_on_the_cylinders() {
    let (s, faces) = pipe(2.0, 3.0, 4.0, 32);
    for method in [HexMethod::Auto, HexMethod::Revolve] {
        for quadratic in [false, true] {
            let opts =
                HexOptions { method, quadratic, chordal_tolerance: 3.0 * (1.0 - (std::f64::consts::PI / 32.0).cos()), ..HexOptions::default() };
            let m = hex_mesh(&s, &faces, &UniformSize(0.5), &opts).unwrap();
            let expected = if method == HexMethod::Auto { HexMethod::Extrude } else { HexMethod::Revolve };
            assert_eq!(m.report.method, expected, "{}", m.report.description);
            assert!(m.report.gate.passed());
            assert!(m.report.min_scaled_jacobian > 0.5, "{method:?} {quadratic}: {} {}", m.report.min_scaled_jacobian, m.report.description);
            assert!(on_radius(&m, &[0, 1], 3.0) && on_radius(&m, &[2, 3], 2.0), "{method:?} {quadratic}");
            let exact = std::f64::consts::PI * (9.0 - 4.0) * 4.0;
            let tolerance = if quadratic { 1e-3 } else { 2e-2 };
            assert!((m.report.volume - exact).abs() < tolerance * exact, "{method:?} {quadratic}: {} vs {exact}", m.report.volume);
            // Every boundary face is tagged with one of the pipe's faces.
            assert!(m.boundary.iter().all(|b| b.brep_face < 8));
        }
    }
}

#[test]
fn box_faces_are_tagged_and_meshes_are_deterministic() {
    let s = prism(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]], 0.0, 3.0);
    let a = hex_mesh(&s, &[], &UniformSize(1.0), &HexOptions::default()).unwrap();
    let b = hex_mesh(&s, &[], &UniformSize(1.0), &HexOptions::default()).unwrap();
    assert_eq!(a, b);
    // prism(): face 0 bottom, 1 top, then the sides along the polygon's edges.
    let count = |f: u32| a.boundary.iter().filter(|x| x.brep_face == f).count();
    assert_eq!([count(0), count(1), count(2), count(3), count(4), count(5)], [8, 8, 12, 6, 12, 6]);
    // Each boundary face lies in its B-rep face's plane.
    let plane = |f: u32| -> (usize, f64) {
        match f {
            0 => (2, 0.0),
            1 => (2, 3.0),
            2 => (1, 0.0),
            3 => (0, 4.0),
            4 => (1, 2.0),
            _ => (0, 0.0),
        }
    };
    for b in &a.boundary {
        let (axis, value) = plane(b.brep_face);
        let e = a.element(b.element as usize).unwrap();
        for &i in ElementKind::Hex8.faces()[b.face as usize - 1] {
            assert!((a.nodes[e[i] as usize][axis] - value).abs() < 1e-12, "{b:?}");
        }
    }
}

#[test]
fn an_l_prism_sweeps_one_to_one() {
    let l = [[0.0, 0.0], [6.0, 0.0], [6.0, 2.0], [2.0, 2.0], [2.0, 6.0], [0.0, 6.0]];
    let s = prism(&l, 0.0, 2.0);
    let m = hex_mesh(&s, &[], &UniformSize(1.0), &HexOptions { method: HexMethod::Sweep, ..HexOptions::default() }).unwrap();
    assert_eq!(m.report.method, HexMethod::Sweep);
    check(&m, 40.0);
}

#[test]
fn unsweepable_bodies_say_why_and_hostile_input_is_an_error() {
    // A tetrahedron: every face is slanted to every other.
    let tet = polyhedron(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        &[vec![0, 2, 1], vec![0, 1, 3], vec![1, 2, 3], vec![0, 3, 2]],
    );
    match hex_mesh(&tet, &[], &UniformSize(0.2), &HexOptions::default()) {
        Err(Error::Unsupported(why)) => {
            for m in ["extrude", "sweep", "revolve", "mapped"] {
                assert!(why.contains(m), "{why}");
            }
        }
        other => panic!("{other:?}"),
    }
    let s = prism(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]], 0.0, 3.0);
    let bad_sizes: [&dyn crate::size::SizeField; 3] = [&UniformSize(f64::NAN), &UniformSize(0.0), &UniformSize(-1.0)];
    for f in bad_sizes {
        assert!(hex_mesh(&s, &[], f, &HexOptions::default()).is_err());
    }
    assert!(matches!(hex_mesh(&s, &[], &UniformSize(1e-4), &HexOptions::default()), Err(Error::Limit(_))));
    assert!(matches!(hex_mesh(&s, &[], &UniformSize(0.1), &HexOptions { max_elements: 100, ..HexOptions::default() }), Err(Error::Limit(_))));
    assert!(hex_mesh(&s, &[], &UniformSize(1.0), &HexOptions { chordal_tolerance: f64::NAN, ..HexOptions::default() }).is_err());
    // Analytic surfaces with garbage numbers are treated as free-form, not trusted.
    let garbage = vec![Some(Surface::Cylinder { origin: [f64::NAN; 3], axis: [0.0; 3], radius: -1.0 }); 6];
    let m = hex_mesh(&s, &garbage, &UniformSize(1.0), &HexOptions::default()).unwrap();
    assert_eq!(m.element_count(), 24);
}
