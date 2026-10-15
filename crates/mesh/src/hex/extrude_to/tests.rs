use super::*;
use crate::tet::geometry::AnalyticSurface;

/// Untrimmed analytic faces, by index.
struct Faces(Vec<AnalyticSurface>);

impl Geometry for Faces {
    fn project(&self, face: u32, p: P) -> Option<P> {
        self.0.get(face as usize)?.project(p)
    }
    fn normal(&self, face: u32, p: P) -> Option<P> {
        self.0.get(face as usize)?.normal_at(p)
    }
}

/// An `nx` × `ny` grid of quadrilaterals over `[x0, x1] × [y0, y1]` at `z`.
fn grid(x: [f64; 2], y: [f64; 2], z: f64, nx: u32, ny: u32) -> (Vec<P>, Vec<[u32; 4]>) {
    let mut nodes = Vec::new();
    for j in 0..=ny {
        for i in 0..=nx {
            let (u, v) = (f64::from(i) / f64::from(nx), f64::from(j) / f64::from(ny));
            nodes.push([x[0] + u * (x[1] - x[0]), y[0] + v * (y[1] - y[0]), z]);
        }
    }
    let id = |i: u32, j: u32| j * (nx + 1) + i;
    let quads = (0..ny).flat_map(|j| (0..nx).map(move |i| [id(i, j), id(i + 1, j), id(i + 1, j + 1), id(i, j + 1)])).collect();
    (nodes, quads)
}

fn spec<'a>(nodes: &'a [P], quads: &'a [[u32; 4]], targets: &'a [u32], sides: &'a [u32]) -> ExtrudeSpec<'a> {
    ExtrudeSpec {
        nodes,
        quads,
        path: ExtrudePath::Direction([0.0, 0.0, 1.0]),
        targets,
        sides,
        layers: ExtrudeLayers::Count(8),
        bias: 1.0,
        tolerance: 1e-6,
        max_elements: 1_000_000,
    }
}

fn last_layer(m: &Extruded, n_src: usize) -> &[P] {
    &m.nodes[m.layers * n_src..]
}

#[test]
fn a_square_section_reaches_a_cylinder() {
    let g = Faces(vec![AnalyticSurface::Cylinder { origin: [0.0, 0.0, 30.0], axis: [1.0, 0.0, 0.0], radius: 20.0 }]);
    let (nodes, quads) = grid([-4.0, 4.0], [-4.0, 4.0], 0.0, 4, 4);
    let m = extrude_to_surface(&spec(&nodes, &quads, &[0], &[]), &g).unwrap();
    assert_eq!(m.hexes.len(), 16 * 8);
    assert_eq!(&m.nodes[..nodes.len()], &nodes[..], "the source nodes are kept");
    for p in last_layer(&m, nodes.len()) {
        let r = (p[1].powi(2) + (p[2] - 30.0).powi(2)).sqrt();
        assert!((r - 20.0).abs() < 1e-9, "{p:?} is {r} from the axis");
    }
    assert!(m.max_target_gap < 1e-9);
    assert!(m.min_scaled_jacobian >= 0.5, "{}", m.min_scaled_jacobian);
    assert!(m.gate.passed());
    // Volume of the column under the arc: ∫∫ (30 − √(400 − y²)) dx dy over the square.
    let exact = 8.0 * (30.0 * 8.0 - (4.0 * (400.0f64 - 16.0).sqrt() + 400.0 * (0.2f64).asin()));
    assert!((m.gate.volume - exact).abs() < 1e-2 * exact, "{} vs {exact}", m.gate.volume);
    // Every boundary face is tagged: 16 source, 16 target, 4 × 4 × 8 sides.
    assert_eq!(m.boundary.iter().filter(|b| b.role == ExtrudeRole::Source).count(), 16);
    assert_eq!(m.boundary.iter().filter(|b| b.role == ExtrudeRole::Target(0)).count(), 16);
    assert_eq!(m.boundary.iter().filter(|b| b.role == ExtrudeRole::Side(None)).count(), 128);
}

#[test]
fn a_square_section_reaches_a_sphere_cap_with_graded_layers() {
    let g = Faces(vec![AnalyticSurface::Sphere { center: [0.0, 0.0, -5.0], radius: 25.0 }]);
    let (nodes, quads) = grid([-6.0, 6.0], [-6.0, 6.0], 0.0, 6, 6);
    let s = ExtrudeSpec { bias: 0.25, layers: ExtrudeLayers::Size(2.0), ..spec(&nodes, &quads, &[0], &[]) };
    let m = extrude_to_surface(&s, &g).unwrap();
    assert_eq!(m.layers, 10, "about 20 mm at 2 mm");
    for p in last_layer(&m, nodes.len()) {
        let r = (p[0].powi(2) + p[1].powi(2) + (p[2] + 5.0).powi(2)).sqrt();
        assert!((r - 25.0).abs() < 1e-9, "{p:?}");
    }
    // Graded: the last layer is a quarter of the first at the centre node.
    let c = 3 * 7 + 3;
    let z = |k: usize| m.nodes[k * nodes.len() + c][2];
    let (first, last) = (z(1) - z(0), z(10) - z(9));
    assert!((last / first - 0.25).abs() < 1e-9, "{first} {last}");
    assert!(m.min_scaled_jacobian >= 0.5, "{}", m.min_scaled_jacobian);
}

#[test]
fn layers_follow_side_faces_that_are_not_straight() {
    // Waisted sides x = ±(24 − √(400 − (z − 5)²)), flat sides y = ±4, a tilted target plane.
    let x0 = 24.0 - 375.0f64.sqrt();
    let g = Faces(vec![
        AnalyticSurface::Plane { normal: [0.0, -0.1, 1.0], point: [0.0, 0.0, 10.0] },
        AnalyticSurface::Cylinder { origin: [24.0, 0.0, 5.0], axis: [0.0, 1.0, 0.0], radius: 20.0 },
        AnalyticSurface::Cylinder { origin: [-24.0, 0.0, 5.0], axis: [0.0, 1.0, 0.0], radius: 20.0 },
        AnalyticSurface::Plane { normal: [0.0, 1.0, 0.0], point: [0.0, 4.0, 0.0] },
        AnalyticSurface::Plane { normal: [0.0, 1.0, 0.0], point: [0.0, -4.0, 0.0] },
    ]);
    let (nodes, quads) = grid([-x0, x0], [-4.0, 4.0], 0.0, 6, 6);
    let m = extrude_to_surface(&spec(&nodes, &quads, &[0], &[1, 2, 3, 4]), &g).unwrap();
    let n = nodes.len();
    // Every boundary node of every layer is on its side face.
    let on_side = |p: P| {
        let waist = 24.0 - (400.0 - (p[2] - 5.0).powi(2)).sqrt();
        (p[0].abs() - waist).abs() < 1e-6 || (p[1].abs() - 4.0).abs() < 1e-9
    };
    for k in 0..=m.layers {
        for j in 0..=6u32 {
            for i in 0..=6u32 {
                if i == 0 || j == 0 || i == 6 || j == 6 {
                    let p = m.nodes[k * n + (j * 7 + i) as usize];
                    assert!(on_side(p), "layer {k} node {p:?}");
                }
            }
        }
    }
    for p in last_layer(&m, n) {
        assert!((p[2] - (10.0 + 0.1 * p[1])).abs() < 1e-9, "{p:?}");
    }
    assert!(m.boundary.iter().any(|b| b.role == ExtrudeRole::Side(Some(1))));
    assert!(m.boundary.iter().all(|b| b.role != ExtrudeRole::Side(None)));
    assert!(m.min_scaled_jacobian >= 0.5, "{}", m.min_scaled_jacobian);
}

#[test]
fn normal_paths_turn_toward_the_target() {
    let g = Faces(vec![AnalyticSurface::Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, -10.0] }]);
    // Counterclockwise from above: the normal is +Z, the target below.
    let (nodes, quads) = grid([0.0, 4.0], [0.0, 4.0], 0.0, 2, 2);
    let s = ExtrudeSpec { path: ExtrudePath::Normal, layers: ExtrudeLayers::Count(5), ..spec(&nodes, &quads, &[0], &[]) };
    let m = extrude_to_surface(&s, &g).unwrap();
    assert!(last_layer(&m, nodes.len()).iter().all(|p| (p[2] + 10.0).abs() < 1e-12));
    assert!((m.gate.volume - 160.0).abs() < 1e-9 && m.min_scaled_jacobian > 0.99);
}

#[test]
fn hostile_input_is_an_error() {
    let g = Faces(vec![AnalyticSurface::Plane { normal: [0.0, 0.0, 1.0], point: [0.0, 0.0, 10.0] }]);
    let (nodes, quads) = grid([0.0, 1.0], [0.0, 1.0], 0.0, 1, 1);
    let ok = spec(&nodes, &quads, &[0], &[]);
    assert!(extrude_to_surface(&ok, &g).is_ok());
    let bad = [
        ExtrudeSpec { bias: f64::NAN, ..ok.clone() },
        ExtrudeSpec { bias: 0.0, ..ok.clone() },
        ExtrudeSpec { layers: ExtrudeLayers::Count(0), ..ok.clone() },
        ExtrudeSpec { layers: ExtrudeLayers::Count(MAX_LAYERS + 1), ..ok.clone() },
        ExtrudeSpec { layers: ExtrudeLayers::Size(f64::INFINITY), ..ok.clone() },
        ExtrudeSpec { layers: ExtrudeLayers::Size(1e-12), ..ok.clone() },
        ExtrudeSpec { tolerance: -1.0, ..ok.clone() },
        ExtrudeSpec { targets: &[], ..ok.clone() },
        ExtrudeSpec { targets: &[7], ..ok.clone() },
        ExtrudeSpec { path: ExtrudePath::Direction([0.0; 3]), ..ok.clone() },
        // Away from the target.
        ExtrudeSpec { path: ExtrudePath::Direction([0.0, 0.0, -1.0]), ..ok.clone() },
        // Parallel to it.
        ExtrudeSpec { path: ExtrudePath::Direction([1.0, 0.0, 0.0]), ..ok.clone() },
        ExtrudeSpec { quads: &[[0, 1, 2, 9]], ..ok.clone() },
        ExtrudeSpec { quads: &[[0, 1, 1, 2]], ..ok.clone() },
        ExtrudeSpec { max_elements: 1, layers: ExtrudeLayers::Count(2), ..ok.clone() },
        // A side face the section is not on.
        ExtrudeSpec { sides: &[0], ..ok.clone() },
    ];
    for (i, s) in bad.iter().enumerate() {
        assert!(extrude_to_surface(s, &g).is_err(), "case {i}");
    }
}

#[test]
fn layer_parameters_grade_geometrically() {
    assert_eq!(parameters(1, 5.0), vec![0.0, 1.0]);
    let t = parameters(4, 1.0);
    assert!(t.iter().zip([0.0, 0.25, 0.5, 0.75, 1.0]).all(|(a, b)| (a - b).abs() < 1e-15));
    let t = parameters(3, 4.0);
    let h: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).collect();
    assert!((h[2] / h[0] - 4.0).abs() < 1e-12 && (h[1] / h[0] - 2.0).abs() < 1e-12, "{h:?}");
}
