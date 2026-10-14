//! Throughput and filter hit rate of the exact predicates. Ignored by default; run with
//! `cargo test --release -p fieldcraft-mesh --test predicates_bench -- --ignored --nocapture`.

use std::hint::black_box;
use std::time::Instant;

use fieldcraft_mesh::predicates::{
    GenericPoint, Lpi, Point3, Sign, generic_orient3d, insphere, insphere_filter, insphere_sos, orient3d, orient3d_filter,
};

struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn index(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn k3(p: &Point3) -> robust::Coord3D<f64> {
    let [x, y, z] = p.coords();
    robust::Coord3D { x, y, z }
}

fn p3(x: f64, y: f64, z: f64) -> Point3 {
    Point3::new(x, y, z).unwrap()
}

/// Uniform points in the unit cube.
fn random_cloud(r: &mut Rng, n: usize) -> Vec<Point3> {
    (0..n).map(|_| p3(r.unit(), r.unit(), r.unit())).collect()
}

/// Vertices of a tessellated CAD part: a 100 × 60 × 40 mm box with 5 mm grid faces and a
/// Ø30 cylinder boss (64 segments, 8 rings) on top. Coplanar and cospherical subsets abound.
fn cad_cloud() -> Vec<Point3> {
    let mut v = Vec::new();
    let (lx, ly, lz, h) = (100.0, 60.0, 40.0, 5.0);
    let steps = |l: f64| (0..=(l / h) as usize).map(move |i| i as f64 * h);
    for x in steps(lx) {
        for y in steps(ly) {
            v.push(p3(x, y, 0.0));
            v.push(p3(x, y, lz));
        }
        for z in steps(lz) {
            v.push(p3(x, 0.0, z));
            v.push(p3(x, ly, z));
        }
    }
    for y in steps(ly) {
        for z in steps(lz) {
            v.push(p3(0.0, y, z));
            v.push(p3(lx, y, z));
        }
    }
    for ring in 0..8 {
        for k in 0..64 {
            let t = k as f64 * std::f64::consts::TAU / 64.0;
            v.push(p3(50.0 + 15.0 * t.cos(), 30.0 + 15.0 * t.sin(), lz + ring as f64 * 2.5));
        }
    }
    v
}

fn queries<const N: usize>(r: &mut Rng, cloud: &[Point3], n: usize) -> Vec<[Point3; N]> {
    (0..n).map(|_| std::array::from_fn(|_| cloud[r.index(cloud.len())])).collect()
}

fn time<T>(label: &str, items: &[T], mut f: impl FnMut(&T) -> Sign) {
    let start = Instant::now();
    let mut acc = 0i64;
    for _ in 0..5 {
        for it in items {
            acc += f(it) as i64;
        }
    }
    let ns = start.elapsed().as_nanos() as f64 / (5 * items.len()) as f64;
    println!("  {label:<34} {ns:7.1} ns/call");
    black_box(acc);
}

fn hit_rate<T>(items: &[T], f: impl Fn(&T) -> Option<Sign>) -> f64 {
    100.0 * items.iter().filter(|it| f(it).is_some()).count() as f64 / items.len() as f64
}

#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn predicate_throughput_and_filter_hit_rate() {
    let mut r = Rng(42);
    let n = 200_000;
    for (name, cloud) in [("random unit cube", random_cloud(&mut r, 10_000)), ("CAD tessellation", cad_cloud())] {
        let o: Vec<[Point3; 4]> = queries(&mut r, &cloud, n);
        let s: Vec<[Point3; 5]> = queries(&mut r, &cloud, n);
        println!("{name} ({} points, {n} queries)", cloud.len());
        println!(
            "  filter hit rate: orient3d {:.2}%, insphere {:.2}%",
            hit_rate(&o, |[a, b, c, d]| orient3d_filter(a, b, c, d)),
            hit_rate(&s, |[a, b, c, d, e]| insphere_filter(a, b, c, d, e))
        );
        time("orient3d", &o, |[a, b, c, d]| orient3d(a, b, c, d));
        time("orient3d (robust crate)", &o, |[a, b, c, d]| {
            let k = |p: &Point3| {
                let [x, y, z] = p.coords();
                robust::Coord3D { x, y, z }
            };
            Sign::of(robust::orient3d(k(a), k(b), k(c), k(d)))
        });
        time("orient3d naive f64 (inexact)", &o, |[a, b, c, d]| {
            let ([ax, ay, az], [bx, by, bz], [cx, cy, cz], [dx, dy, dz]) = (a.coords(), b.coords(), c.coords(), d.coords());
            let (adx, ady, adz, bdx, bdy, bdz, cdx, cdy, cdz) = (ax - dx, ay - dy, az - dz, bx - dx, by - dy, bz - dz, cx - dx, cy - dy, cz - dz);
            Sign::of(adz * (bdx * cdy - cdx * bdy) + bdz * (cdx * ady - adx * cdy) + cdz * (adx * bdy - bdx * ady))
        });
        time("insphere", &s, |[a, b, c, d, e]| insphere(a, b, c, d, e));
        time("insphere (robust crate)", &s, |[a, b, c, d, e]| {
            let k = |p: &Point3| {
                let [x, y, z] = p.coords();
                robust::Coord3D { x, y, z }
            };
            Sign::of(robust::insphere(k(a), k(b), k(c), k(d), k(e)))
        });
        time("insphere_sos", &s, |[a, b, c, d, e]| insphere_sos(a, b, c, d, e));
    }

    // Fully degenerate CAD queries: four vertices of one planar face (orient3d = 0), and five
    // grid vertices of one face (insphere = 0 when not all coplanar is impossible: use the
    // cylinder rings, which are cospherical in groups of points on two rings).
    let face: Vec<Point3> = (0..=20).flat_map(|i| (0..=12).map(move |j| p3(i as f64 * 5.0, j as f64 * 5.0, 40.0))).collect();
    let o: Vec<[Point3; 4]> = queries(&mut r, &face, n);
    let ring = |z: f64| -> Vec<Point3> {
        (0..64)
            .map(|k| {
                let t = k as f64 * std::f64::consts::TAU / 64.0;
                p3(50.0 + 15.0 * t.cos(), 30.0 + 15.0 * t.sin(), z)
            })
            .collect()
    };
    let (r0, r1) = (ring(40.0), ring(42.5));
    let s: Vec<[Point3; 5]> = (0..n)
        .map(|i| {
            let mut q: [Point3; 5] = std::array::from_fn(|k| if k < 3 { r0[r.index(64)] } else { r1[r.index(64)] });
            // Same angle on both rings: symmetric configurations that are exactly cospherical.
            if i % 2 == 0 {
                let k = r.index(64);
                q[3] = r1[k];
                q[4] = r1[(k + 32) % 64];
            }
            q
        })
        .collect();
    println!("degenerate CAD queries ({n} each)");
    println!(
        "  filter hit rate: coplanar orient3d {:.2}%, ring insphere {:.2}%",
        hit_rate(&o, |[a, b, c, d]| orient3d_filter(a, b, c, d)),
        hit_rate(&s, |[a, b, c, d, e]| insphere_filter(a, b, c, d, e))
    );
    time("orient3d (coplanar face)", &o, |[a, b, c, d]| orient3d(a, b, c, d));
    time("insphere (two cylinder rings)", &s, |[a, b, c, d, e]| insphere(a, b, c, d, e));
    time("insphere_sos (two cylinder rings)", &s, |[a, b, c, d, e]| insphere_sos(a, b, c, d, e));
    time("insphere (robust crate, rings)", &s, |[a, b, c, d, e]| Sign::of(robust::insphere(k3(a), k3(b), k3(c), k3(d), k3(e))));
    time("orient3d (robust crate, face)", &o, |[a, b, c, d]| Sign::of(robust::orient3d(k3(a), k3(b), k3(c), k3(d))));

    // Indirect: orient3d with one line-plane intersection and three explicit points.
    let cloud = random_cloud(&mut r, 1000);
    let mut items = Vec::new();
    while items.len() < 20_000 {
        let q: [Point3; 8] = std::array::from_fn(|_| cloud[r.index(cloud.len())]);
        if let Ok(l) = Lpi::new(&q[0], &q[1], &q[2], &q[3], &q[4]) {
            items.push([GenericPoint::Lpi(l), q[5].into(), q[6].into(), q[7].into()]);
        }
    }
    println!("indirect (20000 queries)");
    time("generic_orient3d (LPI + 3 explicit)", &items, |[a, b, c, d]| generic_orient3d(a, b, c, d));
}
