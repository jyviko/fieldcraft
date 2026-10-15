use super::complex::*;
use super::measure;
use super::ops::*;
use crate::predicates::Point3;

/// A cube [0, 1]³ split into six tetrahedra around the diagonal 0–6, boundary faces tagged by
/// cube side (0..6).
pub(crate) fn cube() -> TetComplex {
    let p: Vec<[f64; 3]> = (0..8).map(|i| [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64]).collect();
    // Vertex i = (x, y, z) bits; diagonal 0 → 7.
    let raw = [[0, 1, 3, 7], [0, 3, 2, 7], [0, 2, 6, 7], [0, 6, 4, 7], [0, 4, 5, 7], [0, 5, 1, 7]];
    let pts: Vec<Point3> = p.iter().map(|c| Point3::try_from(*c).unwrap()).collect();
    let tets: Vec<[u32; 4]> = raw
        .iter()
        .map(|t| {
            let t = t.map(|v: usize| v as u32);
            let x = t.map(|v| pts[v as usize].coords());
            if measure::volume6(&x) > 0.0 { t } else { [t[1], t[0], t[2], t[3]] }
        })
        .collect();
    let side = |f: [u32; 3]| -> Option<u32> {
        let c = f.map(|v| p[v as usize]);
        for axis in 0..3 {
            for (s, val) in [(0, 0.0), (1, 1.0)] {
                if c.iter().all(|q| q[axis] == val) {
                    return Some((axis * 2 + s) as u32);
                }
            }
        }
        None
    };
    let mut boundary = Vec::new();
    for (t, vs) in tets.iter().enumerate() {
        for k in 0..4 {
            if let Some(tag) = side(face_toward(*vs, k)) {
                boundary.push((t as u32, k, tag));
            }
        }
    }
    let kinds = vec![VertexKind::Corner; 8];
    TetComplex::new(&p, kinds, &tets, &boundary, &[], 1000, 10_000).unwrap()
}

fn volume(cx: &TetComplex) -> f64 {
    cx.live_tets().map(|(_, vs)| measure::volume6(&cx.coords(vs).unwrap()) / 6.0).sum()
}

#[test]
fn cube_builds_and_rings() {
    let mut cx = cube();
    assert_eq!(cx.live_count(), 6);
    assert!((volume(&cx) - 1.0).abs() < 1e-15);
    let ring = cx.edge_ring(0, 7).unwrap().unwrap();
    assert!(ring.closed);
    assert_eq!(ring.verts.len(), 6);
    assert_eq!(ring.tets.len(), 6);
    let ring = cx.edge_ring(0, 1).unwrap().unwrap();
    assert!(!ring.closed);
    assert_eq!(ring.verts.len(), ring.tets.len() + 1);
}

#[test]
fn interior_insertion_and_rollback() {
    let mut cx = cube();
    cx.journal.recording = true;
    let start = cx.journal.steps.len();
    let p = Point3::try_from([0.5, 0.5, 0.5]).unwrap();
    let t = match cx.walk(0, &p, 100).unwrap().unwrap() {
        Walk::Inside(t) => t,
        w => panic!("{w:?}"),
    };
    let v = insert_delaunay(&mut cx, p, VertexKind::Interior, Site::Interior(t), 1000).unwrap().unwrap();
    assert!(cx.is_used(v));
    assert!((volume(&cx) - 1.0).abs() < 1e-15);
    assert!(cx.live_count() > 6);
    cx.rollback(start).unwrap();
    cx.validate().unwrap();
    assert_eq!(cx.live_count(), 6);
    assert_eq!(cx.point_count(), 8);
    assert!((volume(&cx) - 1.0).abs() < 1e-15);
}

#[test]
fn boundary_face_insertion_keeps_tags() {
    let mut cx = cube();
    // A point on the z = 0 side.
    let p = Point3::try_from([0.3, 0.6, 0.0]).unwrap();
    let mut site = None;
    for (t, _) in cx.live_tets().collect::<Vec<_>>() {
        let vs = cx.tet(t).unwrap();
        for k in 0..4 {
            if cx.adj(t).unwrap()[k] == NONE && cx.tags(t).unwrap()[k] == 4 {
                let f = face_toward(vs, k).map(|v| cx.xyz(v).unwrap());
                // inside the triangle in xy
                let s = |a: [f64; 3], b: [f64; 3]| (b[0] - a[0]) * (0.6 - a[1]) - (b[1] - a[1]) * (0.3 - a[0]);
                let signs = [s(f[0], f[1]), s(f[1], f[2]), s(f[2], f[0])];
                if signs.iter().all(|x| *x > 0.0) || signs.iter().all(|x| *x < 0.0) {
                    site = Some(Site::Face(t, k));
                }
            }
        }
    }
    let v = insert_delaunay(&mut cx, p, VertexKind::Face(4), site.unwrap(), 1000).unwrap().unwrap();
    assert!(cx.is_used(v));
    assert!((volume(&cx) - 1.0).abs() < 1e-15);
    let c = cx.compact().unwrap();
    let area: f64 = c
        .boundary
        .iter()
        .filter(|b| b.2 == 4)
        .map(|&(t, k, _)| {
            let f = face_toward(c.tets[t as usize], k).map(|v| c.points[v as usize]);
            0.5 * measure::norm(measure::cross(measure::sub(f[1], f[0]), measure::sub(f[2], f[0])))
        })
        .sum();
    assert!((area - 1.0).abs() < 1e-15, "{area}");
}

#[test]
fn edge_and_face_removal_keep_the_mesh_valid() {
    let mut cx = cube();
    let p = Point3::try_from([0.5, 0.45, 0.55]).unwrap();
    let Walk::Inside(t) = cx.walk(0, &p, 100).unwrap().unwrap() else { panic!() };
    insert_delaunay(&mut cx, p, VertexKind::Interior, Site::Interior(t), 1000).unwrap().unwrap();
    let mut changed = 0;
    for _ in 0..3 {
        let edges = edges_of(&cx, &cx.live_tets().map(|(t, _)| t).collect::<Vec<_>>()).unwrap();
        for (a, b) in edges {
            if cx.edge_tet(a, b).unwrap().is_some() && remove_edge(&mut cx, a, b).unwrap() {
                changed += 1;
            }
        }
        let tets: Vec<u32> = cx.live_tets().map(|(t, _)| t).collect();
        for t in tets {
            for k in 0..4 {
                if cx.is_live(t) && remove_face(&mut cx, t, k).unwrap() {
                    changed += 1;
                }
            }
        }
    }
    assert!(changed > 0);
    cx.validate().unwrap();
    assert!((volume(&cx) - 1.0).abs() < 1e-14);
    for (_, vs) in cx.live_tets() {
        assert_eq!(cx.orient(vs).unwrap(), crate::predicates::Sign::Positive);
    }
}

mod pipeline {
    use super::super::geometry::AnalyticSurface;
    use super::super::{Order, QualityOptions, quality_mesh};
    use crate::cdt::tests::{cube, surface, uv_sphere};
    use crate::cdt::{CdtOptions, tetrahedralize};
    use crate::surface::{BoundarySurface, SurfaceOptions};

    /// A 10 mm cube with one B-rep face per side.
    fn box_surface() -> BoundarySurface {
        let (p, t) = cube();
        let p: Vec<[f64; 3]> = p.iter().map(|c| c.map(|x| 10.0 * x)).collect();
        let s = surface(&p, &t, [5.0; 3]);
        // One face tag per side: the side's axis and end.
        let faces: Vec<u32> = s
            .triangles()
            .iter()
            .map(|t| {
                let c = t.map(|v| s.points()[v as usize]);
                (0..3).find_map(|k| [0.0, 10.0].iter().position(|&x| c.iter().all(|q| q[k] == x)).map(|e| (2 * k + e) as u32)).unwrap()
            })
            .collect();
        BoundarySurface::new(s.points(), s.triangles(), &faces, &SurfaceOptions::default()).unwrap()
    }

    #[test]
    fn a_box_is_refined_improved_and_deterministic() {
        let s = box_surface();
        let cdt = tetrahedralize(&s, &CdtOptions::default()).unwrap();
        let o = QualityOptions { size: Some(2.5), ..QualityOptions::default() };
        let a = quality_mesh(&s, &cdt, Vec::new(), &o).unwrap();
        let r = &a.report;
        assert!(r.output.tets > 100, "{r:?}");
        assert_eq!(r.output.inadmissible, 0);
        assert_eq!(r.output.slivers, 0, "{r:?}");
        assert!(r.output.min_dihedral > 25.0 && r.output.max_dihedral < 155.0, "{r:?}");
        // Flat faces: the volume is the box's, and each side keeps its tag on its own plane.
        assert!((r.volume - 1000.0).abs() < 1e-9, "{}", r.volume);
        for b in &a.boundary {
            let t: [u32; 4] = a.elements[b.tet as usize].clone().try_into().unwrap();
            let f = super::super::complex::face_toward(t, b.opposite as usize).map(|v| a.points[v as usize]);
            let (k, end) = ((b.face / 2) as usize, if b.face % 2 == 0 { 0.0 } else { 10.0 });
            assert!(f.iter().all(|p| p[k] == end), "face {} at {f:?}", b.face);
        }
        // The same input gives the same mesh, bit for bit.
        let b = quality_mesh(&s, &cdt, Vec::new(), &o).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_seeded_edge_gets_the_seeded_spacing_and_the_size_grows_from_it() {
        use crate::seeds::EdgeSeed;
        use fieldcraft_model::SeedConstraint;
        let s = box_surface();
        let cdt = tetrahedralize(&s, &CdtOptions::default()).unwrap();
        let edge = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]];
        let seed = EdgeSeed { name: "front".into(), points: edge, tolerance: 0.0, count: 16, bias: None, constraint: SeedConstraint::Exact };
        let plain = QualityOptions { size: Some(5.0), seeds: vec![EdgeSeed { count: 1, ..seed.clone() }], ..QualityOptions::default() };
        let seeded = QualityOptions { seeds: vec![seed], ..plain.clone() };
        let a = quality_mesh(&s, &cdt, Vec::new(), &plain).unwrap();
        let b = quality_mesh(&s, &cdt, Vec::new(), &seeded).unwrap();
        let (na, nb) = (a.report.seeds[0].2, b.report.seeds[0].2);
        // Unseeded, the 10 mm edge gets about two 5 mm elements; seeded, about sixteen.
        assert!(na <= 4, "{na}");
        assert_eq!(nb, 16);
        assert_eq!(b.report.seeds[0].1, 16);
        assert!(b.report.output.tets > a.report.output.tets && b.report.output.inadmissible == 0);
        // Biased: the mesh's nodes on the edge are the seed's.
        let bias = Some(fieldcraft_model::Bias { ratio: 4.0, toward: fieldcraft_model::BiasToward::Start });
        let biased = QualityOptions { seeds: vec![EdgeSeed { count: 8, bias, ..seeded.seeds[0].clone() }], ..plain.clone() };
        let c = quality_mesh(&s, &cdt, Vec::new(), &biased).unwrap();
        let mut x: Vec<f64> = c.points.iter().filter(|p| p[1].abs() < 1e-9 && p[2].abs() < 1e-9).map(|p| p[0]).collect();
        x.sort_by(f64::total_cmp);
        let want: Vec<f64> = biased.seeds[0].fractions(8, false).iter().map(|f| 10.0 * f).collect();
        assert_eq!(x.len(), want.len(), "{x:?}");
        assert!(x.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-9), "{x:?} {want:?}");
    }

    #[test]
    fn a_sphere_keeps_its_boundary_on_the_sphere_to_second_order() {
        let (p, t) = uv_sphere(10, 20);
        let p: Vec<[f64; 3]> = p.iter().map(|c| c.map(|x| 5.0 * x)).collect();
        let s = surface(&p, &t, [0.0; 3]);
        let faces = vec![0u32; s.triangles().len()];
        let s = BoundarySurface::new(s.points(), s.triangles(), &faces, &SurfaceOptions::default()).unwrap();
        let cdt = tetrahedralize(&s, &CdtOptions::default()).unwrap();
        let sphere = vec![Some(AnalyticSurface::Sphere { center: [0.0; 3], radius: 5.0 })];
        let o = QualityOptions { size: Some(1.5), order: Order::Quadratic, ..QualityOptions::default() };
        let m = quality_mesh(&s, &cdt, sphere, &o).unwrap();
        assert_eq!(m.kind, fieldcraft_model::ElementKind::Tet10);
        assert_eq!(m.report.straightened, 0);
        // Every node of a boundary face, corners and mid-edge nodes, lies on the sphere.
        let mut on = 0;
        for b in &m.boundary {
            let e = &m.elements[b.tet as usize];
            let k = b.opposite as usize;
            let corners: Vec<usize> = (0..4).filter(|&i| i != k).collect();
            let mids =
                [(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)].iter().enumerate().filter(|(_, (i, j))| *i != k && *j != k).map(|(n, _)| 4 + n);
            for i in corners.into_iter().chain(mids) {
                let q = m.points[e[i] as usize];
                let r = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
                assert!((r - 5.0).abs() < 1e-9, "a boundary node {i} of {e:?} at {q:?}, radius {r}; report {:?}", m.report);
                on += 1;
            }
        }
        assert!(on > 0);
        assert!(m.report.output.min_dihedral > 15.0, "{:?}", m.report.output);
        // Every element passes the certified check of the gate.
        for e in &m.elements {
            let xyz: Vec<[f64; 3]> = e.iter().map(|&v| m.points[v as usize]).collect();
            let v = crate::quality::element_validity(fieldcraft_model::ElementKind::Tet10, &xyz).unwrap();
            assert_eq!(v.validity, crate::quality::Validity::Valid);
        }
    }

    #[test]
    fn caps_stop_the_stage_without_panicking() {
        let s = box_surface();
        let cdt = tetrahedralize(&s, &CdtOptions::default()).unwrap();
        for o in [
            QualityOptions { size: Some(1e-3), max_points: 200, ..QualityOptions::default() },
            QualityOptions { size: Some(0.5), max_tets: 500, ..QualityOptions::default() },
            QualityOptions { size: Some(f64::NAN), ..QualityOptions::default() },
            QualityOptions { size: Some(-1.0), ..QualityOptions::default() },
        ] {
            match quality_mesh(&s, &cdt, Vec::new(), &o) {
                Ok(m) => assert!(!m.report.stopped.is_empty(), "{:?}", m.report),
                Err(e) => assert!(matches!(e, crate::Error::Limit(_) | crate::Error::Invalid(_)), "{e}"),
            }
        }
    }
}
