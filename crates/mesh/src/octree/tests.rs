use super::*;
use crate::octree::geom::tests::box_surface;
use crate::size::UniformSize;

#[test]
fn a_box_meshes_with_exact_volume_and_tagged_faces() {
    let s = box_surface([0.0; 3], [10.0, 8.0, 6.0]);
    let m = octree_hex_mesh(&s, &UniformSize(2.0), &OctreeOptions::default()).unwrap();
    let r = &m.report;
    assert!((r.volume - 480.0).abs() < 1e-9 * 480.0, "{r:?}");
    assert!(r.min_scaled_jacobian > 0.3, "{r:?}");
    let faces: BTreeSet<u32> = m.boundary.iter().map(|b| b.brep_face).collect();
    assert_eq!(faces.len(), 6);
    // Each boundary quad lies in the plane of its face.
    for b in &m.boundary {
        let hx = m.hexes[b.element as usize];
        let q = topo::FACES[(b.face - 1) as usize].map(|i| m.nodes[hx[i] as usize]);
        let (axis, value) = match b.brep_face {
            0 => (2, 0.0),
            1 => (2, 6.0),
            2 => (1, 0.0),
            3 => (1, 8.0),
            4 => (0, 0.0),
            _ => (0, 10.0),
        };
        assert!(q.iter().all(|p| (p[axis] - value).abs() < 1e-9), "{b:?} {q:?}");
    }
}

/// A closed cylinder of radius `r` and height `h` on the z axis, `n` segments: faces 0 (bottom
/// cap), 1 (top cap), 2 (side).
pub(crate) fn cylinder_surface(r: f64, h: f64, n: usize) -> BoundarySurface {
    let mut p = vec![[0.0, 0.0, 0.0], [0.0, 0.0, h]];
    for z in [0.0, h] {
        for i in 0..n {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            p.push([r * a.cos(), r * a.sin(), z]);
        }
    }
    let (b, t) = (|i: usize| (2 + i % n) as u32, |i: usize| (2 + n + i % n) as u32);
    let mut tris = Vec::new();
    let mut face = Vec::new();
    for i in 0..n {
        tris.push([0, b(i + 1), b(i)]);
        face.push(0);
        tris.push([1, t(i), t(i + 1)]);
        face.push(1);
        tris.push([b(i), b(i + 1), t(i + 1)]);
        tris.push([b(i), t(i + 1), t(i)]);
        face.extend([2, 2]);
    }
    BoundarySurface::new(&p, &tris, &face, &crate::surface::SurfaceOptions::default()).unwrap()
}

/// A UV sphere of radius `r`, one face.
pub(crate) fn sphere_surface(r: f64, n: usize) -> BoundarySurface {
    let rings = n / 2;
    let mut p = vec![[0.0, 0.0, -r], [0.0, 0.0, r]];
    for j in 1..rings {
        let th = std::f64::consts::PI * j as f64 / rings as f64 - std::f64::consts::FRAC_PI_2;
        for i in 0..n {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            p.push([r * th.cos() * a.cos(), r * th.cos() * a.sin(), r * th.sin()]);
        }
    }
    let id = |j: usize, i: usize| (2 + (j - 1) * n + i % n) as u32;
    let mut tris = Vec::new();
    for i in 0..n {
        tris.push([0, id(1, i + 1), id(1, i)]);
        tris.push([1, id(rings - 1, i), id(rings - 1, i + 1)]);
    }
    for j in 1..rings - 1 {
        for i in 0..n {
            tris.push([id(j, i), id(j, i + 1), id(j + 1, i + 1)]);
            tris.push([id(j, i), id(j + 1, i + 1), id(j + 1, i)]);
        }
    }
    let face = vec![0; tris.len()];
    BoundarySurface::new(&p, &tris, &face, &crate::surface::SurfaceOptions::default()).unwrap()
}

#[test]
fn a_cylinder_meshes_with_its_rims_captured() {
    let s = cylinder_surface(5.0, 8.0, 64);
    let m = octree_hex_mesh(&s, &UniformSize(2.0), &OctreeOptions::default()).unwrap();
    let r = &m.report;
    assert!(((r.volume - r.surface_volume) / r.surface_volume).abs() < 0.01, "{r:?}");
    let faces: BTreeSet<u32> = m.boundary.iter().map(|b| b.brep_face).collect();
    assert_eq!(faces.len(), 3);
}

#[test]
fn a_sphere_meshes() {
    let s = sphere_surface(5.0, 48);
    let m = octree_hex_mesh(&s, &UniformSize(2.0), &OctreeOptions::default()).unwrap();
    let r = &m.report;
    assert!(((r.volume - r.surface_volume) / r.surface_volume).abs() < 0.01, "{r:?}");
}
