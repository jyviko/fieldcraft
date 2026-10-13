//! Throughput of the Delaunay tetrahedralization on 1M random points and on a 100³ grid (every
//! cell cospherical). Ignored by default; run with
//! `cargo test --release -p modelcraft-mesh --test delaunay_bench -- --ignored --nocapture`.

use std::time::Instant;

use modelcraft_mesh::delaunay::Triangulation;

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
}

fn run(name: &str, points: &[[f64; 3]]) {
    let start = Instant::now();
    let tri = Triangulation::new(points).unwrap();
    let build = start.elapsed().as_secs_f64();
    let start = Instant::now();
    tri.verify().unwrap();
    let verify = start.elapsed().as_secs_f64();
    let tets = tri.finite_tet_count();
    println!(
        "{name}: {} points -> {tets} tets in {build:.2} s ({:.0} points/s, {:.0} tets/s); verify {verify:.2} s",
        points.len(),
        points.len() as f64 / build,
        tets as f64 / build
    );
}

#[test]
#[ignore]
fn delaunay_throughput() {
    let mut r = Rng(1);
    let random: Vec<[f64; 3]> = (0..1_000_000).map(|_| [r.unit(), r.unit(), r.unit()]).collect();
    if std::env::var("BENCH_GRID_ONLY").is_err() {
        run("random 1M", &random);
    }

    let n = 100;
    let grid: Vec<[f64; 3]> = (0..n * n * n).map(|i| [(i % n) as f64, ((i / n) % n) as f64, (i / (n * n)) as f64]).collect();
    run("grid 100^3", &grid);
}
