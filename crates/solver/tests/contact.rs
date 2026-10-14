//! Contact in nonlinear static steps against closed forms: the contact patch test on
//! non-matching meshes, Newton's convergence with contact, and what implicit contact refuses.
//! Run with `--nocapture` to see the iterations.

use std::collections::BTreeMap;

use fieldcraft_model::*;
use fieldcraft_solver::contact::Status;
use fieldcraft_solver::{Limits, StaticResult, solve_nonlinear};

/// A model built block by block.
#[derive(Default)]
struct Blocks {
    model: Model,
    xyz: BTreeMap<u32, [f64; 3]>,
    next_node: u32,
    next_element: u32,
}

impl Blocks {
    /// A structured block of `n` cells over the unit cube mapped by `map`, hex8 or tet4 (six
    /// Kuhn tetrahedra per cell); its elements' labels.
    fn block(&mut self, kind: ElementKind, n: [usize; 3], map: impl Fn([f64; 3]) -> [f64; 3]) -> Vec<u32> {
        let first = self.next_node + 1;
        let label = |i: [usize; 3]| first + (i[0] + (n[0] + 1) * (i[1] + (n[1] + 1) * i[2])) as u32;
        for k in 0..=n[2] {
            for j in 0..=n[1] {
                for i in 0..=n[0] {
                    let x = map([i as f64 / n[0] as f64, j as f64 / n[1] as f64, k as f64 / n[2] as f64]);
                    let l = label([i, j, k]);
                    self.model.set_node(l, x).unwrap();
                    self.xyz.insert(l, x);
                    self.next_node = self.next_node.max(l);
                }
            }
        }
        let mut out = Vec::new();
        for k in 0..n[2] {
            for j in 0..n[1] {
                for i in 0..n[0] {
                    let at = |d: [usize; 3]| label([i + d[0], j + d[1], k + d[2]]);
                    let cells: Vec<Vec<u32>> = match kind {
                        ElementKind::Hex8 => vec![
                            [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]].iter().map(|d| at(*d)).collect(),
                        ],
                        _ => [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]]
                            .iter()
                            .map(|perm| {
                                let mut v = [[0usize; 3]; 4];
                                for s in 0..3 {
                                    v[s + 1] = v[s];
                                    v[s + 1][perm[s]] = 1;
                                }
                                let p = v.map(|c| c.map(|x| x as f64));
                                let e = |i: usize| [0, 1, 2].map(|k| p[i][k] - p[0][k]);
                                let (a, b, c) = (e(1), e(2), e(3));
                                let vol =
                                    a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]);
                                if vol < 0.0 {
                                    v.swap(1, 2);
                                }
                                v.iter().map(|d| at(*d)).collect()
                            })
                            .collect(),
                    };
                    for nodes in cells {
                        self.next_element += 1;
                        self.model.set_element(self.next_element, kind, nodes).unwrap();
                        out.push(self.next_element);
                    }
                }
            }
        }
        out
    }

    /// The faces of `elements` whose nodes all satisfy `f`.
    fn faces(&self, elements: &[u32], f: impl Fn([f64; 3]) -> bool) -> Vec<ElementFace> {
        let mut out = Vec::new();
        for l in elements {
            let e = self.model.element(*l).unwrap();
            for (i, face) in e.kind.faces().iter().enumerate() {
                if face.iter().all(|k| f(self.xyz[&e.nodes[*k]])) {
                    out.push(ElementFace { element: *l, face: i as u8 + 1 });
                }
            }
        }
        out
    }

    fn nodes(&self, f: impl Fn([f64; 3]) -> bool) -> Vec<u32> {
        self.xyz.iter().filter(|(_, x)| f(**x)).map(|(l, _)| *l).collect()
    }

    fn node_set(&mut self, name: &str, f: impl Fn([f64; 3]) -> bool) {
        let nodes = self.nodes(f);
        self.model.add_set(name, Set { nodes, ..Set::default() }).unwrap();
    }

    fn face_set(&mut self, name: &str, elements: &[u32], f: impl Fn([f64; 3]) -> bool) {
        let faces = self.faces(elements, f);
        assert!(!faces.is_empty(), "no faces for `{name}`");
        self.model.add_set(name, Set { faces, ..Set::default() }).unwrap();
    }

    fn bc(&mut self, name: &str, values: [Option<f64>; 3], f: impl Fn([f64; 3]) -> bool) {
        self.node_set(name, f);
        let [a, b, c] = values;
        self.model
            .add_boundary_condition(name, BoundaryCondition::Displacement { target: Target::Set(name.into()), values: [a, b, c, None, None, None] })
            .unwrap();
        self.model.set_boundary_condition_active("Step", name, true).unwrap();
    }

    /// One elastic material and section over every element, and the nonlinear step.
    fn define(&mut self, young: f64, poisson: f64, settings: NonlinearStatic) {
        let all: Vec<u32> = self.model.elements().keys().copied().collect();
        self.model.add_set("all", Set { elements: all, ..Set::default() }).unwrap();
        self.model.add_material("Mat", Material { elastic: Some(Elastic::Isotropic { young, poisson }), ..Material::default() }).unwrap();
        self.model
            .add_section(
                "solid",
                Section { set: "all".into(), material: "Mat".into(), kind: SectionKind::Solid {}, formulation: Formulation::default() },
            )
            .unwrap();
        self.model.add_step(Step::new("Step", Procedure::NonlinearStatic(settings)), None).unwrap();
    }

    fn pair(&mut self, name: &str, main: &str, secondary: &str) {
        let c = Contact::new(ContactSurfaces::Pair { main: main.into(), secondary: secondary.into() });
        self.model.add_contact(name, c).unwrap();
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn report(r: &StaticResult) {
    let h = r.nonlinear.as_ref().unwrap();
    for inc in &h.increments {
        let forces: Vec<String> = inc.iterations.iter().map(|i| format!("{:.1e}", i.force)).collect();
        eprintln!("increment {} λ {:.3} aug {} [{}]", inc.number, inc.load_factor, inc.augmentations, forces.join(" "));
    }
}

/// Two blocks pressed together across non-matching meshes (`lower` × `upper` cells, each of
/// `lower_kind` / `upper_kind`), between frictionless symmetry planes: a uniaxial stress.
fn patch(lower_kind: ElementKind, upper_kind: ElementKind, geometry: bool, distorted: bool) -> (Blocks, StaticResult) {
    let mut b = Blocks::default();
    // A graded lower block, its cells boxes of different sizes, or distorted (its faces stay the
    // planes of the cube either way).
    let low = if distorted {
        b.block(lower_kind, [3, 3, 2], |p| [p[0] + 0.3 * p[0] * (1.0 - p[0]) * (p[1] - 0.5), p[1] + 0.3 * p[1] * (1.0 - p[1]) * (p[0] - 0.4), p[2]])
    } else {
        b.block(lower_kind, [3, 3, 2], |p| [p[0].powf(1.4), p[1].powf(0.8), p[2]])
    };
    let upper = [2, 2, 2];
    let up = b.block(upper_kind, upper, |p| [p[0], p[1], 1.0 + p[2]]);
    b.define(
        1000.0,
        0.3,
        NonlinearStatic {
            nonlinear_geometry: geometry,
            initial_increment: 0.5,
            // Tight for small sliding (linear: exact); finite sliding converges to the default ones.
            tolerances: if geometry { Tolerances::default() } else { Tolerances { force: 1e-10, displacement: 1e-8, energy: 1e-16 } },
            ..NonlinearStatic::default()
        },
    );
    b.face_set("lower_top", &low, |x| near(x[2], 1.0));
    b.face_set("upper_bottom", &up, |x| near(x[2], 1.0));
    b.pair("interface", "lower_top", "upper_bottom");
    b.bc("x0", [Some(0.0), None, None], |x| near(x[0], 0.0));
    b.bc("y0", [None, Some(0.0), None], |x| near(x[1], 0.0));
    b.bc("z0", [None, None, Some(0.0)], |x| near(x[2], 0.0));
    b.bc("push", [None, None, Some(-0.02)], |x| near(x[2], 2.0));
    let r = solve_nonlinear(&b.model, Some("Step"), &Limits::default()).unwrap();
    report(&r);
    (b, r)
}

/// The contact patch test (Taylor & Papadopoulos 1991; Crisfield 2000): a uniform pressure
/// crosses a non-matching interface exactly, so every element of both blocks has the same
/// uniaxial stress, with nothing across.
#[test]
fn the_mortar_interface_passes_the_contact_patch_test() {
    for (lower, upper, name) in [
        (ElementKind::Hex8, ElementKind::Hex8, "hex8 on hex8"),
        (ElementKind::Hex8, ElementKind::Tet4, "tet4 on hex8"),
        (ElementKind::Tet4, ElementKind::Hex8, "hex8 on tet4"),
    ] {
        for geometry in [false, true] {
            // Small sliding is linear and exact; finite sliding is exact up to the gaps the
            // augmentation leaves (their geometric terms, GAP_TOLERANCE) and the convergence.
            let tol = if geometry { 1e-4 } else { 1e-9 };
            let (b, r) = patch(lower, upper, geometry, false);
            let szz: Vec<f64> = r.elements.iter().flat_map(|e| e.nodal.iter().map(|s| s[2])).collect();
            let mean = szz.iter().sum::<f64>() / szz.len() as f64;
            assert!(mean < 0.0, "{name}: {mean}");
            for (e, s) in r.elements.iter().flat_map(|e| e.nodal.iter().map(move |s| (e.label, s))) {
                assert!((s[2] - mean).abs() < tol * mean.abs(), "{name} ({geometry}) element {e}: σzz {} vs {mean}", s[2]);
                assert!(s[0].abs() < tol * mean.abs() && s[1].abs() < tol * mean.abs(), "{name} ({geometry}) element {e}: {s:?}");
            }
            let c = &r.contacts[0];
            // The pressure is uniform, equal to the stress, at every secondary node.
            assert!(c.pressure.iter().all(|p| (p + mean).abs() < tol * mean.abs()), "{name}: {:?} vs {mean}", c.pressure);
            // The contact force on the upper block balances the reaction at its top.
            let top: f64 = b.nodes(|x| near(x[2], 2.0)).iter().map(|l| r.reaction[r.nodes.binary_search(l).unwrap()][2]).sum();
            assert!((c.force[2] + top).abs() < tol * top.abs(), "{name}: {:?} vs {top}", c.force);
            assert!(c.force[0].abs() < tol * top.abs() && c.force[1].abs() < tol * top.abs(), "{name}: {:?}", c.force);
            assert!((c.normal_force + top).abs() < tol * top.abs(), "{name}: {} vs {top}", c.normal_force);
            // Augmented to a gap tolerance far below the penetration a penalty alone leaves.
            assert!(c.penetration < 1e-5, "{name}: {}", c.penetration);
        }
    }
}

/// Hertz contact of a cylinder on a flat (plane strain): a quarter model of an indenter whose
/// bottom is a circle of radius `r` on a block, symmetric about x = 0, one element thick in z
/// with z held (plane strain). Returns the model and the result.
fn hertz_cylinder(finite: bool, push: f64) -> (Blocks, StaticResult) {
    let r = 10.0;
    let mut b = Blocks::default();
    let grade = |s: f64| s.powf(1.6);
    let foundation = b.block(ElementKind::Hex8, [24, 14, 1], |p| [5.0 * grade(p[0]), -5.0 * grade(1.0 - p[1]), 0.2 * p[2]]);
    let indenter = b.block(ElementKind::Hex8, [20, 10, 1], |p| {
        let x = 2.5 * grade(p[0]);
        [x, r - (r * r - x * x).sqrt() + 2.5 * grade(p[1]), 0.2 * p[2]]
    });
    b.define(1000.0, 0.3, NonlinearStatic { nonlinear_geometry: finite, initial_increment: 0.25, ..NonlinearStatic::default() });
    b.face_set("flat", &foundation, |x| near(x[1], 0.0));
    let bottom = |x: [f64; 3]| (x[1] - (r - (r * r - x[0] * x[0]).sqrt())).abs() < 1e-9;
    b.face_set("round", &indenter, bottom);
    b.pair("hertz", "flat", "round");
    b.bc("symmetry", [Some(0.0), None, None], |x| near(x[0], 0.0));
    b.bc("plane", [None, None, Some(0.0)], |_| true);
    b.bc("base", [None, Some(0.0), None], |x| near(x[1], -5.0));
    let top = 2.5 + r - (r * r - 2.5 * 2.5f64).sqrt();
    b.bc("push", [None, Some(-push), None], move |x| {
        x[1] > 2.5 && (x[1] - (r - (r * r - x[0] * x[0]).sqrt()) - 2.5).abs() < 1e-9 && x[1] <= top + 1e-9
    });
    let r = solve_nonlinear(&b.model, Some("Step"), &Limits::default()).unwrap();
    report(&r);
    (b, r)
}

/// Hertz's line contact (H. Hertz, *Über die Berührung fester elastischer Körper*, J. reine
/// angew. Math. 92:156–171, 1882; K. L. Johnson, *Contact Mechanics*, Cambridge 1985, §4.3):
/// for a load P per length, b = √(4 P R / (π E*)) and p₀ = 2 P / (π b), E* = E / (2 (1 − ν²))
/// for two bodies of one material.
#[test]
fn hertz_cylinder_on_a_flat() {
    for finite in [false, true] {
        let (b, r) = hertz_cylinder(finite, 0.03);
        let c = &r.contacts[0];
        let thickness = 0.2;
        // The load per length: the reaction at the base over the slab's thickness, both halves.
        let base: f64 = b.nodes(|x| near(x[1], -5.0)).iter().map(|l| r.reaction[r.nodes.binary_search(l).unwrap()][1]).sum();
        let p = 2.0 * base / thickness;
        let e_star = 1000.0 / (2.0 * (1.0 - 0.09));
        let half_width = (4.0 * p * 10.0 / (std::f64::consts::PI * e_star)).sqrt();
        let peak = 2.0 * p / (std::f64::consts::PI * half_width);
        let found = c.pressure.iter().fold(0.0f64, |m, v| m.max(*v));
        let (before, next) = edge(&b, c, 2);
        eprintln!("finite {finite}: P {p:.4} b {half_width:.4} ({before:.4}..{next:.4}) p0 {peak:.4} found {found:.4}");
        assert!((found - peak).abs() < 0.03 * peak, "p0 {found} vs {peak}");
        assert!(before <= half_width && half_width <= next, "b {half_width} not in ({before}, {next})");
        if finite {
            quadratic(&r);
        }
    }
}

/// The contact's edge along the x axis (the nodes on the plane where coordinate `across` is 0):
/// the node before the last closed one and the first open one after it, so that the edge lies
/// within an element of the last closed node (whose status the augmentation's tolerance decides).
fn edge(b: &Blocks, c: &fieldcraft_solver::contact::ContactResult, across: usize) -> (f64, f64) {
    let mut along: Vec<(f64, Status)> =
        c.nodes.iter().zip(&c.status).filter(|(l, _)| near(b.xyz[l][across], 0.0)).map(|(l, s)| (b.xyz[l][0], *s)).collect();
    along.sort_by(|x, y| x.0.total_cmp(&y.0));
    let last = along.iter().rposition(|(_, s)| *s == Status::Closed).unwrap_or(0);
    let before = last.checked_sub(1).and_then(|i| along.get(i)).map_or(0.0, |p| p.0);
    let next = along.get(last + 1).map_or(f64::INFINITY, |p| p.0);
    (before, next)
}

/// Newton's rate with contact: within each solution of an increment (its first, and each
/// augmentation's), once the residual is small every correction squares it, up to a constant
/// (the consistent tangent of the mortar integrals, segmentation and normal included).
fn quadratic(r: &StaticResult) {
    let mut checked = 0;
    for inc in &r.nonlinear.as_ref().unwrap().increments {
        for w in inc.iterations.windows(2) {
            let (a, b) = (w[0], w[1]);
            // Below 1e-10 of the force scale the sums' round-off takes over.
            if a.augmentation != b.augmentation || a.force > 1e-3 || b.force < 1e-10 {
                continue;
            }
            assert!(b.force <= 100.0 * a.force * a.force, "increment {}: {:e} after {:e}", inc.number, b.force, a.force);
            checked += 1;
        }
    }
    assert!(checked >= 3, "too few iterations near the solution to judge ({checked})");
}

/// Two unit blocks, the upper one starting `overlap` into the lower one, pressed together.
fn stacked(overlap: f64, penetration: InitialPenetration) -> Blocks {
    let mut b = Blocks::default();
    let low = b.block(ElementKind::Hex8, [2, 2, 1], |p| p);
    let up = b.block(ElementKind::Hex8, [3, 3, 1], |p| [p[0], p[1], 1.0 - overlap + p[2]]);
    b.define(1000.0, 0.3, NonlinearStatic { nonlinear_geometry: true, initial_increment: 1.0, ..NonlinearStatic::default() });
    b.face_set("lower_top", &low, |x| near(x[2], 1.0));
    b.face_set("upper_bottom", &up, |x| near(x[2], 1.0 - overlap));
    let mut c = Contact::new(ContactSurfaces::Pair { main: "lower_top".into(), secondary: "upper_bottom".into() });
    c.initial_penetration = penetration;
    b.model.add_contact("interface", c).unwrap();
    b.bc("x0", [Some(0.0), None, None], |x| near(x[0], 0.0));
    b.bc("y0", [None, Some(0.0), None], |x| near(x[1], 0.0));
    b.bc("z0", [None, None, Some(0.0)], |x| near(x[2], 0.0));
    b.bc("push", [None, None, Some(-0.01)], move |x| near(x[2], 2.0 - overlap));
    b
}

#[test]
fn initial_overclosure_is_adjusted_or_refused() {
    let b = stacked(1e-3, InitialPenetration::Adjust);
    let r = solve_nonlinear(&b.model, Some("Step"), &Limits::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("start inside the main surface")), "{:?}", r.warnings);
    // Adjusted, the blocks behave as if they started touching: the same stress as without the
    // overlap.
    let touching = solve_nonlinear(&stacked(0.0, InitialPenetration::Adjust).model, Some("Step"), &Limits::default()).unwrap();
    let (a, t) = (r.elements[0].nodal[0][2], touching.elements[0].nodal[0][2]);
    assert!((a - t).abs() < 1e-4 * t.abs(), "{a} vs {t}");
    let e = solve_nonlinear(&stacked(1e-3, InitialPenetration::Refuse).model, Some("Step"), &Limits::default()).unwrap_err().to_string();
    assert!(e.contains("starts") && e.contains("inside"), "{e}");
}

#[test]
fn contact_results_do_not_depend_on_the_thread_count() {
    let b = stacked(0.0, InitialPenetration::Adjust);
    let run = |threads| fieldcraft_parallel::with_threads(threads, || solve_nonlinear(&b.model, Some("Step"), &Limits::default()).unwrap());
    let (one, many) = (run(1), run(4));
    assert_eq!(one.displacement, many.displacement);
    assert_eq!(one.contacts, many.contacts);
}

#[test]
fn what_implicit_contact_refuses() {
    let solve = |m: &Model| solve_nonlinear(m, Some("Step"), &Limits::default()).unwrap_err().to_string();
    // Linear analyses refuse every contact by name.
    let mut b = stacked(0.0, InitialPenetration::Adjust);
    b.model.add_step(Step::new("Linear", Procedure::Static {}), None).unwrap();
    let e = fieldcraft_solver::solve_static(&b.model, Some("Linear"), &Limits::default()).unwrap_err().to_string();
    assert!(e.contains("interface") && e.contains("linear static"), "{e}");
    // General contact.
    let mut b = stacked(0.0, InitialPenetration::Adjust);
    b.model.add_contact("all", Contact::new(ContactSurfaces::General { surface: None })).unwrap();
    assert!(solve(&b.model).contains("general"));
    // Arc-length control.
    let mut b = stacked(0.0, InitialPenetration::Adjust);
    let arc = NonlinearStatic { arc_length: Some(ArcLength::default()), ..NonlinearStatic::default() };
    b.model.replace_step(Step { procedure: Procedure::NonlinearStatic(arc), ..b.model.step("Step").unwrap().clone() }).unwrap();
    assert!(solve(&b.model).contains("arc-length"));
}

/// Hertz's point contact (Johnson 1985, §4.2): a sphere of radius R on a flat of the same
/// material under a load P has the contact radius a = (3 P R / (4 E*))^(1/3) and the peak
/// pressure p₀ = 3 P / (2 π a²), E* = E / (2 (1 − ν²)). A quarter model, displacement controlled;
/// the load is the reaction at the base.
#[test]
#[ignore = "heavy: cargo xtask test --heavy (CI heavy job)"]
fn hertz_sphere_on_a_flat() {
    let (r, e, nu) = (10.0, 1000.0, 0.3);
    let mut b = Blocks::default();
    let grade = |s: f64| s.powf(1.5);
    let foundation = b.block(ElementKind::Hex8, [16, 16, 10], |p| [5.0 * grade(p[0]), 5.0 * grade(p[1]), -5.0 * grade(1.0 - p[2])]);
    let cap = move |x: f64, y: f64| r - (r * r - x * x - y * y).sqrt();
    let indenter = b.block(ElementKind::Hex8, [14, 14, 8], |p| {
        let (x, y) = (2.5 * grade(p[0]), 2.5 * grade(p[1]));
        let bottom = cap(x, y);
        [x, y, bottom + (3.0 - bottom) * grade(p[2])]
    });
    b.define(e, nu, NonlinearStatic { nonlinear_geometry: false, initial_increment: 0.25, ..NonlinearStatic::default() });
    b.face_set("flat", &foundation, |x| near(x[2], 0.0));
    b.face_set("round", &indenter, move |x| (x[2] - cap(x[0], x[1])).abs() < 1e-9 && x[2] < 1.0);
    b.pair("hertz", "flat", "round");
    b.bc("x0", [Some(0.0), None, None], |x| near(x[0], 0.0));
    b.bc("y0", [None, Some(0.0), None], |x| near(x[1], 0.0));
    b.bc("base", [None, None, Some(0.0)], |x| near(x[2], -5.0));
    b.bc("push", [None, None, Some(-0.04)], |x| near(x[2], 3.0));
    let res = solve_nonlinear(&b.model, Some("Step"), &Limits::default()).unwrap();
    report(&res);
    let c = &res.contacts[0];
    let base: f64 = b.nodes(|x| near(x[2], -5.0)).iter().map(|l| res.reaction[res.nodes.binary_search(l).unwrap()][2]).sum();
    let p = 4.0 * base;
    let e_star = e / (2.0 * (1.0 - nu * nu));
    let a = (3.0 * p * r / (4.0 * e_star)).cbrt();
    let peak = 3.0 * p / (2.0 * std::f64::consts::PI * a * a);
    let found = c.pressure.iter().fold(0.0f64, |m, v| m.max(*v));
    let (before, next) = edge(&b, c, 1);
    eprintln!("P {p:.4} a {a:.4} ({before:.4}..{next:.4}) p0 {peak:.4} found {found:.4}");
    assert!((found - peak).abs() < 0.05 * peak, "p0 {found} vs {peak}");
    assert!(before <= a && a <= next, "a {a} not in ({before}, {next})");
}
