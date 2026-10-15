use super::*;
use crate::tet::geometry::AnalyticSurface;

const R_IN: f64 = 1.0;
const R_OUT: f64 = 2.0;
const HEIGHT: f64 = 1.0;

/// A quarter annulus (r 1..2, θ 0..90°, z 0..1) on its analytic faces: 0 the outer cylinder, 1
/// the inner, 2 and 3 the planes z = 0 and z = 1, 4 and 5 the planes θ = 0 and θ = 90°.
struct Annulus;

impl Annulus {
    fn surface(f: u32) -> AnalyticSurface {
        match f {
            0 => AnalyticSurface::Cylinder { origin: [0.0; 3], axis: [0.0, 0.0, 1.0], radius: R_OUT },
            1 => AnalyticSurface::Cylinder { origin: [0.0; 3], axis: [0.0, 0.0, 1.0], radius: R_IN },
            2 => AnalyticSurface::Plane { normal: [0.0, 0.0, -1.0], point: [0.0; 3] },
            3 => AnalyticSurface::Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, HEIGHT] },
            4 => AnalyticSurface::Plane { normal: [0.0, -1.0, 0.0], point: [0.0; 3] },
            _ => AnalyticSurface::Plane { normal: [-1.0, 0.0, 0.0], point: [0.0; 3] },
        }
    }

    /// The faces a point lies on.
    fn faces(p: P) -> Vec<u32> {
        let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
        let tol = 1e-9;
        let mut f = Vec::new();
        for (i, on) in
            [(r - R_OUT).abs() < tol, (r - R_IN).abs() < tol, p[2].abs() < tol, (p[2] - HEIGHT).abs() < tol, p[1].abs() < tol, p[0].abs() < tol]
                .into_iter()
                .enumerate()
        {
            if on {
                f.push(i as u32);
            }
        }
        f
    }

    fn on(p: P, faces: &[u32]) -> bool {
        faces.iter().all(|&f| Annulus::surface(f).project(p).is_some_and(|q| dist(p, q) < 1e-9))
    }
}

/// Curves are encoded as `a * 8 + b`.
struct Geo {
    model: Model,
}

impl Constraints for Geo {
    fn freedom(&self, node: u32) -> Option<Freedom> {
        let p = self.model.node(node)?.xyz;
        let f = Annulus::faces(p);
        Some(match f.as_slice() {
            [] => Freedom::Free,
            [a] => Freedom::Face(*a),
            [a, b] => Freedom::Curve(a * 8 + b),
            _ => Freedom::Fixed,
        })
    }
    fn slide_face(&self, face: u32, p: P, d: P) -> Option<P> {
        Annulus::surface(face).slide(p, d)
    }
    fn slide_curve(&self, curve: u32, p: P, d: P) -> Option<P> {
        let (a, b) = (Annulus::surface(curve / 8), Annulus::surface(curve % 8));
        let mut q = add(p, d);
        for _ in 0..100 {
            q = b.project(a.project(q)?)?;
        }
        Some(q)
    }
}

fn polar(r: f64, t: f64, z: f64) -> P {
    let (s, c) = crate::tet::geometry::sin_cos(t);
    [r * c, r * s, z]
}

/// A hex8 mesh of the annulus with `n` divisions each way; node label `1 + i + (n+1)(j + (n+1)k)`.
fn hex_annulus(n: usize) -> Model {
    let mut m = Model::default();
    let id = |i: usize, j: usize, k: usize| (1 + i + (n + 1) * (j + (n + 1) * k)) as u32;
    for k in 0..=n {
        for j in 0..=n {
            for i in 0..=n {
                let p = polar(
                    R_IN + (R_OUT - R_IN) * i as f64 / n as f64,
                    std::f64::consts::FRAC_PI_2 * j as f64 / n as f64,
                    HEIGHT * k as f64 / n as f64,
                );
                m.set_node(id(i, j, k), p).unwrap();
            }
        }
    }
    let mut label = 1;
    for k in 0..n {
        for j in 0..n {
            for i in 0..n {
                let nodes = vec![
                    id(i, j, k),
                    id(i + 1, j, k),
                    id(i + 1, j + 1, k),
                    id(i, j + 1, k),
                    id(i, j, k + 1),
                    id(i + 1, j, k + 1),
                    id(i + 1, j + 1, k + 1),
                    id(i, j + 1, k + 1),
                ];
                m.set_element(label, ElementKind::Hex8, nodes).unwrap();
                label += 1;
            }
        }
    }
    m
}

/// Move some nodes near `centre`, keeping each on its faces.
fn perturb(m: &mut Model, centre: P, radius: f64, amount: f64) -> BTreeSet<u32> {
    let geo = Geo { model: m.clone() };
    let mut moved = BTreeSet::new();
    let labels: Vec<u32> = m.nodes().keys().copied().collect();
    for (k, n) in labels.into_iter().enumerate() {
        let p = m.node(n).unwrap().xyz;
        if dist(p, centre) > radius {
            continue;
        }
        let d = [amount * ((k * 7 % 5) as f64 - 2.0) / 2.0, amount * ((k * 3 % 5) as f64 - 2.0) / 2.0, amount * ((k * 11 % 5) as f64 - 2.0) / 2.0];
        let q = match geo.freedom(n).unwrap() {
            Freedom::Free => Some(add(p, d)),
            Freedom::Face(f) => geo.slide_face(f, p, d),
            _ => None,
        };
        if let Some(q) = q {
            m.set_node(n, q).unwrap();
            moved.insert(n);
        }
    }
    moved
}

fn region_min(m: &Model, labels: &[u32]) -> (f64, usize) {
    let mut worst = f64::INFINITY;
    let mut invalid = 0;
    for l in labels {
        let e = m.element(*l).unwrap();
        let x: Vec<P> = e.nodes.iter().map(|n| m.node(*n).unwrap().xyz).collect();
        let el = Elem { label: *l, kind: e.kind, nodes: e.nodes.clone(), normal: None };
        let (q, v) = measure(&el, &x);
        worst = worst.min(q);
        invalid += usize::from(!v);
    }
    (worst, invalid)
}

fn applied(m: &Model, r: &Relaxed) -> Model {
    let mut out = m.clone();
    for (n, p) in &r.positions {
        out.set_node(*n, *p).unwrap();
    }
    out
}

const CENTRE: P = [1.414, 1.414, 0.5];

/// The issue's test: a perturbed hex mesh of a cylindrical part relaxes inside the sphere — every
/// surface node on its face, quality up, nothing inverted, nothing outside the sphere moved.
#[test]
fn hex_spikes_relax_with_surface_nodes_on_the_cylinder() {
    let mut m = hex_annulus(6);
    let original = m.clone();
    perturb(&mut m, CENTRE, 0.5, 0.08);
    let elements: Vec<u32> = m.elements().keys().copied().collect();
    let (before, _) = region_min(&m, &elements);
    let weights = sphere_weights(&m, CENTRE, 0.7, 1.0, Falloff::Smooth, None).unwrap();
    let geo = Geo { model: original.clone() };
    let r = relax(&m, &weights, 10, &geo).unwrap();
    assert!(r.moved > 0 && r.after.min_quality > r.before.min_quality, "{:?} → {:?}", r.before, r.after);
    assert_eq!(r.after.invalid, 0);
    let out = applied(&m, &r);
    let (after, invalid) = region_min(&out, &elements);
    assert!(after > before + 0.02 && invalid == 0, "{before} → {after}");
    for (n, p) in &r.positions {
        assert!(dist(m.node(*n).unwrap().xyz, CENTRE) < 0.7, "node {n} outside the sphere moved");
        let faces = Annulus::faces(original.node(*n).unwrap().xyz);
        assert!(Annulus::on(*p, &faces), "node {n} left its faces {faces:?}: {p:?}");
    }
}

/// tet10 with curved mid-side nodes on the cylinder: after relaxing, corner and mid-side surface
/// nodes are still on the surface and no element is invalid.
#[test]
fn tet10_mid_side_nodes_follow_on_the_geometry() {
    let hex = hex_annulus(4);
    let mut m = Model::default();
    for (l, n) in hex.nodes() {
        m.set_node(*l, n.xyz).unwrap();
    }
    // Six tetrahedra per hexahedron around its 1-7 diagonal (Kuhn), consistent across faces.
    const KUHN: [[usize; 4]; 6] = [[0, 1, 2, 6], [0, 2, 3, 6], [0, 3, 7, 6], [0, 7, 4, 6], [0, 4, 5, 6], [0, 5, 1, 6]];
    let mut mids: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut next = 100_000;
    let mut label = 1;
    for e in hex.elements().values() {
        for t in KUHN {
            let mut c: Vec<u32> = t.iter().map(|&i| e.nodes[i]).collect();
            let x: Vec<P> = c.iter().map(|n| m.node(*n).unwrap().xyz).collect();
            if dot(cross(sub(x[1], x[0]), sub(x[2], x[0])), sub(x[3], x[0])) < 0.0 {
                c.swap(1, 2);
            }
            let mut nodes = c.clone();
            for [a, b] in [[0, 1], [1, 2], [2, 0], [0, 3], [1, 3], [2, 3]] {
                let key = (c[a].min(c[b]), c[a].max(c[b]));
                let mid = *mids.entry(key).or_insert_with(|| {
                    let (pa, pb) = (m.node(key.0).unwrap().xyz, m.node(key.1).unwrap().xyz);
                    let mut p = scale(add(pa, pb), 0.5);
                    let shared: Vec<u32> = Annulus::faces(pa).into_iter().filter(|f| Annulus::faces(pb).contains(f)).collect();
                    if let [f] = shared.as_slice() {
                        p = Annulus::surface(*f).project(p).unwrap();
                    }
                    next += 1;
                    m.set_node(next, p).unwrap();
                    next
                });
                nodes.push(mid);
            }
            m.set_element(label, ElementKind::Tet10, nodes).unwrap();
            label += 1;
        }
    }
    let original = m.clone();
    // Perturb the corners only, then put the mid-side nodes at their edges' (curved) middles.
    let corners = corner_nodes(&m);
    perturb(&mut m, CENTRE, 0.6, 0.06);
    for (&(a, b), &mid) in &mids {
        let p = scale(add(m.node(a).unwrap().xyz, m.node(b).unwrap().xyz), 0.5);
        let shared: Vec<u32> =
            Annulus::faces(original.node(a).unwrap().xyz).into_iter().filter(|f| Annulus::faces(original.node(b).unwrap().xyz).contains(f)).collect();
        let p = if let [f] = shared.as_slice() { Annulus::surface(*f).project(p).unwrap() } else { p };
        m.set_node(mid, p).unwrap();
    }
    let _ = corners;
    let weights = sphere_weights(&m, CENTRE, 0.8, 1.0, Falloff::Smooth, None).unwrap();
    let r = relax(&m, &weights, 8, &Geo { model: original.clone() }).unwrap();
    assert!(r.after.min_quality >= r.before.min_quality && r.after.invalid == 0, "{:?} → {:?}", r.before, r.after);
    assert!(r.after.min_quality > r.before.min_quality + 0.01, "{:?} → {:?}", r.before, r.after);
    let mut mid_on_surface = 0;
    for (n, p) in &r.positions {
        let faces = Annulus::faces(original.node(*n).unwrap().xyz);
        assert!(Annulus::on(*p, &faces), "node {n} left {faces:?}");
        mid_on_surface += usize::from(*n > 100_000 && faces.contains(&0));
    }
    assert!(mid_on_surface > 0, "some curved mid-side nodes moved on the cylinder");
}

/// A cut between two cells is not geometry: its shared nodes move freely, and both cells keep
/// them (conforming).
#[test]
fn partition_cut_nodes_move_freely_and_stay_shared() {
    let mut m = hex_annulus(6);
    let original = m.clone();
    perturb(&mut m, CENTRE, 0.5, 0.08);
    // The cut: the plane θ = 45° through the middle row of nodes (j = 3) — reported free.
    struct Cut(Geo);
    impl Constraints for Cut {
        fn freedom(&self, node: u32) -> Option<Freedom> {
            let f = self.0.freedom(node)?;
            Some(if f == Freedom::Free { Freedom::Free } else { f })
        }
        fn slide_face(&self, f: u32, p: P, d: P) -> Option<P> {
            self.0.slide_face(f, p, d)
        }
        fn slide_curve(&self, c: u32, p: P, d: P) -> Option<P> {
            self.0.slide_curve(c, p, d)
        }
    }
    let weights = sphere_weights(&m, CENTRE, 0.7, 1.0, Falloff::Smooth, None).unwrap();
    let r = relax(&m, &weights, 10, &Cut(Geo { model: original.clone() })).unwrap();
    let on_cut = |p: P| (p[0] - p[1]).abs() < 1e-9;
    let cut_moved = r.positions.iter().filter(|(n, p)| on_cut(original.node(**n).unwrap().xyz) && !on_cut(**p)).count();
    assert!(cut_moved > 0, "cut nodes are free to leave the cut plane");
    assert_eq!(r.after.invalid, 0);
}

/// The issue's field test: relax a coarse mesh, mesh 2× finer, apply the field: the fine mesh
/// takes the relaxed shape (its nodes where the relaxed coarse mesh interpolates them) and
/// nothing inverts.
#[test]
fn a_relaxation_field_carries_over_to_a_finer_mesh() {
    let mut coarse = hex_annulus(4);
    let original = coarse.clone();
    // A deliberate interior shape change: pull the interior nodes toward the centre.
    for (n, p) in original.nodes() {
        if Annulus::faces(p.xyz).is_empty() && dist(p.xyz, CENTRE) < 0.6 {
            coarse.set_node(*n, add(p.xyz, [0.03, -0.02, 0.04])).unwrap();
        }
    }
    let weights = sphere_weights(&coarse, CENTRE, 0.9, 1.0, Falloff::Smooth, None).unwrap();
    let r = relax(&coarse, &weights, 5, &Geo { model: coarse.clone() }).unwrap();
    // The field is the change from the original coarse mesh to the relaxed one.
    let relaxed = applied(&coarse, &r);
    let total = Relaxed {
        positions: relaxed.nodes().iter().filter(|(n, p)| original.node(**n).unwrap().xyz != p.xyz).map(|(n, p)| (*n, p.xyz)).collect(),
        ..Default::default()
    };
    let cells = field_cells(&original, &total);
    assert!(!cells.is_empty());
    let fine = hex_annulus(8);
    let out = apply_field(&fine, &cells, None, &Geo { model: fine.clone() }, 0.0).unwrap();
    assert!(out.moved > 0);
    let result = applied(&fine, &out);
    let elements: Vec<u32> = fine.elements().keys().copied().collect();
    assert_eq!(region_min(&result, &elements).1, 0, "nothing inverts");
    for (n, p) in fine.nodes() {
        let d = field_displacement(&cells, p.xyz).unwrap_or([0.0; 3]);
        let got = result.node(*n).unwrap().xyz;
        if Annulus::faces(p.xyz).is_empty() {
            assert!(dist(got, add(p.xyz, d)) < 1e-9, "node {n}: {got:?} vs {:?}", add(p.xyz, d));
        } else {
            assert!(Annulus::on(got, &Annulus::faces(p.xyz)), "node {n} stays on its faces");
        }
    }
    // A fine node at a coarse node takes exactly that node's displacement.
    let coarse_node = original.nodes().iter().find(|(n, p)| r.positions.contains_key(n) && Annulus::faces(p.xyz).is_empty()).unwrap();
    let fine_twin = fine.nodes().values().find(|p| dist(p.xyz, coarse_node.1.xyz) < 1e-9).unwrap();
    let d = field_displacement(&cells, fine_twin.xyz).unwrap();
    assert!(dist(add(fine_twin.xyz, d), relaxed.node(*coarse_node.0).unwrap().xyz) < 1e-8);
}

#[test]
fn hostile_parameters_are_refused() {
    let m = hex_annulus(2);
    assert!(sphere_weights(&m, [f64::NAN, 0.0, 0.0], 1.0, 1.0, Falloff::Smooth, None).is_err());
    assert!(sphere_weights(&m, CENTRE, -1.0, 1.0, Falloff::Smooth, None).is_err());
    assert!(sphere_weights(&m, CENTRE, 1.0, 1.5, Falloff::Smooth, None).is_err());
    let w = sphere_weights(&m, CENTRE, 1.0, 1.0, Falloff::Smooth, None).unwrap();
    assert!(relax(&m, &w, 0, &NoGeometry).is_err());
    assert!(relax(&m, &w, MAX_ITERATIONS + 1, &NoGeometry).is_err());
    let bad: BTreeMap<u32, f64> = [(1, f64::NAN)].into();
    assert!(relax(&m, &bad, 1, &NoGeometry).is_err());
    // Without geometry, skin nodes stay put.
    let r = relax(&m, &w, 3, &NoGeometry).unwrap();
    assert!(r.positions.is_empty() || r.positions.keys().all(|n| Annulus::faces(m.node(*n).unwrap().xyz).is_empty()));
}
