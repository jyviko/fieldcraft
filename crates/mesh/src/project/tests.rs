use std::f64::consts::{PI, TAU};

use super::target::{Constraint, Geometry, place};
use super::*;
use crate::surface::{BoundarySurface, SurfaceOptions};
use crate::tet::geometry::{AnalyticSurface, Tessellation};

/// A UV sphere of radius `r` about the origin, one face, with its exact surface.
fn sphere(r: f64, n: usize) -> (BoundarySurface, Tessellation) {
    let rings = n / 2;
    let mut p = vec![[0.0, 0.0, -r], [0.0, 0.0, r]];
    for j in 1..rings {
        let th = PI * j as f64 / rings as f64 - PI / 2.0;
        for i in 0..n {
            let a = TAU * i as f64 / n as f64;
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
    let s = BoundarySurface::new(&p, &tris, &face, &SurfaceOptions::default()).unwrap();
    let t = Tessellation::new(&s, vec![Some(AnalyticSurface::Sphere { center: [0.0; 3], radius: r })]);
    (s, t)
}

/// A closed cylinder of radius `r` from z = 0 to `h`: faces 0 (bottom), 1 (top), 2 (side).
fn cylinder(r: f64, h: f64, n: usize) -> (BoundarySurface, Tessellation) {
    let mut p = vec![[0.0, 0.0, 0.0], [0.0, 0.0, h]];
    for z in [0.0, h] {
        for i in 0..n {
            let a = TAU * i as f64 / n as f64;
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
    let s = BoundarySurface::new(&p, &tris, &face, &SurfaceOptions::default()).unwrap();
    let analytic = vec![
        Some(AnalyticSurface::Plane { normal: [0.0, 0.0, -1.0], point: [0.0; 3] }),
        Some(AnalyticSurface::Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, h] }),
        Some(AnalyticSurface::Cylinder { origin: [0.0; 3], axis: [0.0, 0.0, 1.0], radius: r }),
    ];
    let t = Tessellation::new(&s, analytic);
    (s, t)
}

fn options(size: f64) -> ProjectOptions {
    ProjectOptions { size, ..ProjectOptions::default() }
}

/// The 7-block sphere: all hex, every boundary node on the sphere, min scaled Jacobian at least
/// 0.5 at every refinement, and the volume converging to the ball's.
#[test]
fn seven_block_sphere_converges_without_losing_quality() {
    let r = 10.0;
    let (s, t) = sphere(r, 96);
    let topo = Topology::sphere([0.0; 3], r, 0.55).unwrap();
    assert_eq!(topo.blocks.len(), 7);
    let exact = 4.0 / 3.0 * PI * r * r * r;
    let mut last_error = f64::INFINITY;
    for size in [3.0, 1.5, 0.75] {
        let m = project_mesh(&topo, &s, &t, &options(size)).unwrap();
        let rep = &m.report;
        assert!(rep.min_scaled_jacobian >= 0.5, "size {size}: {rep:?}");
        assert!(rep.max_gap < 1e-9 * r, "size {size}: {rep:?}");
        for b in &m.boundary {
            let hex = m.hexes[b.element as usize];
            for &l in ElementKind::Hex8.faces()[b.face as usize - 1] {
                let p = m.nodes[hex[l] as usize];
                assert!((crate::vec3::norm(p) - r).abs() < 1e-9, "{p:?}");
            }
        }
        let error = (rep.volume - exact).abs() / exact;
        assert!(error < last_error, "size {size}: {error} after {last_error}");
        last_error = error;
    }
    assert!(last_error < 0.01, "{last_error}");
}

/// More smoothing iterations never leave a worse mesh: the smoother keeps the best grid its
/// sweeps passed through (#489), so where the sweeps happen to stop does not decide the result.
#[test]
fn more_iterations_never_lower_the_smallest_scaled_jacobian() {
    let (r, h) = (10.0, 2.0);
    let (s, t) = cylinder(r, h, 128);
    let topo = Topology::butterfly([0.0; 3], [0.0, 0.0, 1.0], r, &[0.0, h], 0.55).unwrap();
    let mut last = f64::NEG_INFINITY;
    for iterations in [5, 20, 100, 300] {
        let m = project_mesh(&topo, &s, &t, &ProjectOptions { iterations, ..options(0.75) }).unwrap();
        let sj = m.report.min_scaled_jacobian;
        assert!(sj >= last - 1e-9, "{iterations} iterations: {sj} after {last}");
        last = sj;
    }
}

/// A disc: the five-block butterfly, its rim on the circle where the side meets the caps.
#[test]
fn butterfly_disc_meshes_with_its_rim_on_the_circle() {
    let (r, h) = (10.0, 2.0);
    let (s, t) = cylinder(r, h, 128);
    let topo = Topology::butterfly([0.0; 3], [0.0, 0.0, 1.0], r, &[0.0, h], 0.55).unwrap();
    assert_eq!(topo.blocks.len(), 5);
    let mut last = f64::INFINITY;
    for size in [2.0, 1.0, 0.5] {
        let m = project_mesh(&topo, &s, &t, &options(size)).unwrap();
        let rep = &m.report;
        assert!(rep.min_scaled_jacobian >= 0.5, "size {size}: {rep:?}");
        // Every node on the side lies on the cylinder; the rims on both circles.
        let side: Vec<&BoundaryFace> = m.boundary.iter().filter(|b| b.brep_face == 2).collect();
        assert!(!side.is_empty());
        for b in side {
            let hex = m.hexes[b.element as usize];
            for &l in ElementKind::Hex8.faces()[b.face as usize - 1] {
                let p = m.nodes[hex[l] as usize];
                assert!((p[0].hypot(p[1]) - r).abs() < 1e-9, "{p:?}");
            }
        }
        let tags: std::collections::BTreeSet<u32> = m.boundary.iter().map(|b| b.brep_face).collect();
        assert_eq!(tags.len(), 3);
        let error = (rep.volume - PI * r * r * h).abs();
        assert!(error < last, "size {size}");
        last = error;
    }
}

/// A plain grid on a box: corners land on the box's corners, edges on its edges.
#[test]
fn a_grid_on_a_cylinder_is_refused_where_corners_would_flatten() {
    // A single block projected onto a cylinder puts its four side edges on the circle: the
    // block's corners flatten to 180°, which the mesher reports instead of meshing.
    let (s, t) = cylinder(5.0, 4.0, 64);
    let topo = Topology::grid([&[-3.0, 3.0], &[-3.0, 3.0], &[0.0, 4.0]]).unwrap();
    let e = project_mesh(&topo, &s, &t, &options(1.0)).unwrap_err().to_string();
    assert!(e.contains("O-grid") || e.contains("inverted"), "{e}");
}

/// A geometry of exact surfaces, for the curve and corner iterations.
struct Exact(Vec<AnalyticSurface>);

impl Geometry for Exact {
    fn project(&self, face: u32, p: P) -> Option<P> {
        self.0.get(face as usize)?.project(p)
    }
    fn normal(&self, face: u32, p: P) -> Option<P> {
        self.0.get(face as usize)?.normal_at(p)
    }
}

/// Nodes on two targets converge to their intersection curve; on three, to the common point.
#[test]
fn curve_and_corner_nodes_converge_by_tangent_planes() {
    let g = Exact(vec![
        AnalyticSurface::Cylinder { origin: [0.0; 3], axis: [1.0, 0.0, 0.0], radius: 10.0 },
        AnalyticSurface::Cylinder { origin: [0.0; 3], axis: [0.0, 0.0, 1.0], radius: 6.0 },
        AnalyticSurface::Plane { normal: [0.0, 1.0, 0.0], point: [0.0; 3] },
    ]);
    let targets = vec![vec![0], vec![1], vec![2]];
    for start in [[3.0, 3.0, 9.0], [-5.0, 1.0, 7.0], [1.0, -4.0, 11.0]] {
        let p = place(&g, &targets, Constraint::Curve(0, 1), start, 1e-13);
        assert!((p[1].hypot(p[2]) - 10.0).abs() < 1e-9 && (p[0].hypot(p[1]) - 6.0).abs() < 1e-9, "{p:?}");
    }
    let p = place(&g, &targets, Constraint::Corner(0, 1, 2), [5.0, 1.0, 9.0], 1e-13);
    assert!((p[0] - 6.0).abs() < 1e-9 && p[1].abs() < 1e-9 && (p[2] - 10.0).abs() < 1e-9, "{p:?}");
}

#[test]
fn o_grid_keeps_neighbours_and_opens_faces() {
    let mut t = Topology::grid([&[0.0, 1.0, 2.0], &[0.0, 1.0], &[0.0, 1.0]]).unwrap();
    let on = t.o_grid(0, &[2], [0.5; 3]).unwrap();
    assert_eq!(on[0].len(), 5);
    // Inner block + five side blocks (top open), the neighbour untouched.
    assert_eq!(t.blocks.len(), 7);
    t.check().unwrap();
    let uses = lattice::face_uses(&t).unwrap();
    // The top of the old block is now five faces, each used once.
    // Inner block to its five side blocks, the four ring blocks to the bottom one and to each
    // other, and the untouched neighbour.
    assert_eq!(uses.values().filter(|&&u| u == 2).count(), 5 + 4 + 4 + 1, "{uses:?}");
    let made = t.extrude(&on[0], [0.0, 0.0, 1.0], 2).unwrap();
    assert_eq!(made.len(), 10);
    t.check().unwrap();
    let face = t.face_towards(made[8], [0.0, 0.0, 1.0]).unwrap();
    assert_eq!(face, 2);
    let up = t.extrude(&[(made[8], 2)], [0.0, 0.0, 1.0], 1).unwrap();
    let side = t.face_towards(1, [1.0, 0.0, 0.0]).unwrap();
    t.connect((up[0], 2), (1, side), 2).unwrap();
    t.remove(&[1]).unwrap();
    t.check().unwrap();
}

/// A split runs through the whole sheet of blocks: here a 2×2 grid split across its first
/// direction, and the reversed neighbour counts its fractions from the shared end.
#[test]
fn split_cuts_the_whole_sheet_and_stays_conforming() {
    let mut t = Topology::grid([&[0.0, 1.0, 2.0], &[0.0, 1.0, 2.0], &[0.0, 1.0]]).unwrap();
    // Block 1 relabelled so its direction 0 runs the other way (still right-handed).
    t.blocks[1] = [t.blocks[1][2], t.blocks[1][3], t.blocks[1][0], t.blocks[1][1], t.blocks[1][6], t.blocks[1][7], t.blocks[1][4], t.blocks[1][5]];
    t.check().unwrap();
    // Splitting block 0 along Y cuts block 1 (its X neighbour shares the Y edges) but not 2 or 3.
    let made = t.split(0, 1, &[0.25]).unwrap();
    assert_eq!(made.len(), 2);
    assert_eq!(t.blocks.len(), 6);
    t.check().unwrap();
    let uses = lattice::face_uses(&t).unwrap();
    assert!(uses.values().all(|&u| u <= 2));
    // Every new vertex is a quarter of the way up in Y; the cut faces are shared.
    assert!(t.vertices.iter().any(|v| (v[1] - 0.25).abs() < 1e-12));
    assert!(t.vertices.iter().all(|v| [0.0, 0.25, 1.0, 2.0].iter().any(|y| (v[1] - y).abs() < 1e-12)));
    // Splitting along X goes through the whole grid's X sheet: four more blocks.
    let made = t.split(2, 0, &[1.0 / 3.0, 2.0 / 3.0]).unwrap();
    assert_eq!(t.blocks.len(), 6 + 3 * 2);
    assert!(!made.is_empty());
    t.check().unwrap();
    assert!(lattice::face_uses(&t).unwrap().values().all(|&u| u <= 2));
    let mut u = t.clone();
    for bad in [vec![], vec![0.0], vec![0.5, 0.4], vec![1.2], vec![f64::NAN]] {
        assert!(u.split(0, 0, &bad).is_err(), "{bad:?}");
    }
    assert!(u.split(0, 3, &[0.5]).is_err() && u.split(99, 0, &[0.5]).is_err());
    assert_eq!(u, t);
}

#[test]
fn hostile_topologies_are_errors() {
    let (s, g) = sphere(1.0, 32);
    let cube = Topology::grid([&[0.0, 1.0], &[0.0, 1.0], &[0.0, 1.0]]).unwrap();
    let mut inside_out = cube.clone();
    inside_out.blocks[0].swap(1, 3);
    let mut repeated = cube.clone();
    repeated.blocks[0][1] = repeated.blocks[0][0];
    let mut missing = cube.clone();
    missing.blocks[0][7] = 99;
    let mut nan = cube.clone();
    nan.vertices[0] = [f64::NAN, 0.0, 0.0];
    for t in [Topology::default(), inside_out, repeated, missing, nan] {
        assert!(project_mesh(&t, &s, &g, &options(0.5)).is_err(), "{t:?}");
    }
    for size in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(project_mesh(&cube, &s, &g, &options(size)).is_err());
    }
    assert!(project_mesh(&cube, &s, &g, &ProjectOptions { size: 1e-6, max_elements: 1000, ..ProjectOptions::default() }).is_err());
    let bad_face = ProjectOptions { faces: vec![(0, 9, vec![0])], ..options(0.5) };
    assert!(project_mesh(&cube, &s, &g, &bad_face).is_err());
    let bad_target = ProjectOptions { faces: vec![(0, 1, vec![7])], ..options(0.5) };
    assert!(project_mesh(&cube, &s, &g, &bad_target).is_err());
    let mut t = cube.clone();
    assert!(t.o_grid(0, &[], [1.5; 3]).is_err() && t.o_grid(0, &[7], [0.5; 3]).is_err() && t.o_grid(3, &[], [0.5; 3]).is_err());
    assert!(t.extrude(&[(0, 0)], [0.0, 0.0, 1.0], 1).is_err() && t.extrude(&[(0, 1)], [f64::NAN; 3], 1).is_err());
    assert!(Topology::grid([&[0.0], &[0.0, 1.0], &[0.0, 1.0]]).is_err());
    assert!(Topology::grid([&[1.0, 0.0], &[0.0, 1.0], &[0.0, 1.0]]).is_err());
    assert!(Topology::sphere([0.0; 3], -1.0, 0.5).is_err() && Topology::butterfly([0.0; 3], [0.0; 3], 1.0, &[0.0, 1.0], 0.5).is_err());
    let mut g3 = Topology::grid([&[0.0, 1.0], &[0.0, 1.0], &[0.0, 1.0]]).unwrap();
    assert!(g3.carve([1, 1, 1], &[[[0, 0], [0, 0], [0, 0]]]).is_err());
}

/// The quads of `m` on B-rep face `face`, as a patch over its own nodes.
fn patch_of(m: &ProjectedMesh, face: u32) -> FixedPatch {
    let mut index: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    let mut patch = FixedPatch { faces: vec![face], ..FixedPatch::default() };
    for b in m.boundary.iter().filter(|b| b.brep_face == face) {
        let hex = m.hexes[b.element as usize];
        let local = ElementKind::Hex8.faces()[b.face as usize - 1];
        let q = [0, 1, 2, 3].map(|i| {
            let v = hex[local[i]];
            *index.entry(v).or_insert_with(|| {
                patch.nodes.push(m.nodes[v as usize]);
                patch.nodes.len() as u32 - 1
            })
        });
        patch.quads.push(q);
    }
    patch
}

/// A fixed patch is taken as given (#490): a disc meshed against the top quads of another disc's
/// mesh (moved down onto its bottom face, as the face two stacked bodies share) keeps those nodes
/// exactly and their divisions, though its own size asks for coarser elements, and smooths the
/// rest around them.
#[test]
fn a_fixed_patch_sets_the_nodes_and_divisions_of_the_faces_on_it() {
    let (r, h) = (10.0, 2.0);
    let (s, t) = cylinder(r, h, 128);
    let topo = Topology::butterfly([0.0; 3], [0.0, 0.0, 1.0], r, &[0.0, h], 0.55).unwrap();
    let below = project_mesh(&topo, &s, &t, &options(0.75)).unwrap();
    let mut patch = patch_of(&below, 1);
    for p in &mut patch.nodes {
        p[2] -= h;
    }
    patch.faces = vec![0];
    let m = project_mesh(&topo, &s, &t, &ProjectOptions { fixed: vec![patch.clone()], ..options(1.5) }).unwrap();
    assert_eq!(m.report.fixed_nodes, patch.nodes.len());
    assert!(m.report.min_scaled_jacobian >= 0.5, "{:?}", m.report);
    let bottom = patch_of(&m, 0);
    assert_eq!(bottom.quads.len(), patch.quads.len());
    for p in &patch.nodes {
        assert!(bottom.nodes.iter().any(|q| dist(*p, *q) == 0.0), "{p:?} is not a node of the mesh");
    }
    // The top is meshed at the patch's divisions, not the coarser size's.
    assert_eq!(patch_of(&m, 1).quads.len(), patch.quads.len());
}

/// A patch the blocks do not match is an error that names the patch, not a mesh that leaves a
/// gap: here half the bottom's quads, and a patch on a face no block lies on.
#[test]
fn a_fixed_patch_the_blocks_do_not_match_is_an_error() {
    let (r, h) = (10.0, 2.0);
    let (s, t) = cylinder(r, h, 128);
    let topo = Topology::butterfly([0.0; 3], [0.0, 0.0, 1.0], r, &[0.0, h], 0.55).unwrap();
    let m = project_mesh(&topo, &s, &t, &options(1.0)).unwrap();
    let mut half = patch_of(&m, 0);
    half.quads.retain(|q| q.iter().all(|&v| half.nodes[v as usize][0] <= 1e-9));
    let e = project_mesh(&topo, &s, &t, &ProjectOptions { fixed: vec![half], ..options(1.0) }).unwrap_err().to_string();
    assert!(e.contains("fixed patch 0"), "{e}");
    let elsewhere = FixedPatch { faces: vec![7], ..patch_of(&m, 0) };
    let e = project_mesh(&topo, &s, &t, &ProjectOptions { fixed: vec![elsewhere], ..options(1.0) }).unwrap_err().to_string();
    assert!(e.contains("fixed patch 0"), "{e}");
    let broken = FixedPatch { quads: vec![[0, 0, 1, 2]], ..patch_of(&m, 0) };
    assert!(project_mesh(&topo, &s, &t, &ProjectOptions { fixed: vec![broken], ..options(1.0) }).is_err());
}
