//! Insertion order: biased randomized insertion order (BRIO) with Hilbert-curve order inside
//! each round.
//!
//! - N. Amenta, S. Choi, G. Rote, *Incremental constructions con BRIO*, SoCG 2003, 211–219:
//!   points go into rounds of geometrically growing size (each point independently lands in round
//!   `k` from the end with probability 2⁻⁽ᵏ⁺¹⁾). Randomness between rounds keeps the expected
//!   cost of a randomized incremental construction; order inside a round is free.
//! - Inside a round the points follow a Hilbert curve, so consecutive points are close and the
//!   point-location walk from the previous insertion is short. The index is computed with
//!   J. Skilling, *Programming the Hilbert curve*, AIP Conf. Proc. 707:381–387, 2004
//!   (axes → transposed index, then bit interleaving).
//!
//! The "random" round of a point is a fixed hash of its index, so the order, and with it every
//! later step, is a deterministic function of the input. Floating point is used only to quantize
//! coordinates for the curve; it never decides any topology.

use crate::predicates::Point3;

/// Bits per axis of the Hilbert grid (3 × 21 = 63-bit keys).
const BITS: u32 = 21;
/// The deepest BRIO round; deeper hashes are clamped (2⁻²⁰ of the points share it).
const MAX_LEVEL: u32 = 20;

/// SplitMix64 finalizer (Steele, Lea, Flood, *Fast splittable pseudorandom number generators*,
/// OOPSLA 2014): a fixed, well-mixed hash of the vertex index.
pub(super) fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hilbert index of a point on the `2^BITS`-per-axis grid (Skilling 2004).
fn hilbert(mut x: [u32; 3]) -> u64 {
    let m = 1u32 << (BITS - 1);
    // Inverse undo excess work.
    let mut q = m;
    while q > 1 {
        let p = q - 1;
        for i in 0..3 {
            let xi = x[i];
            if xi & q != 0 {
                x[0] ^= p;
            } else {
                let t = (x[0] ^ xi) & p;
                x[0] ^= t;
                x[i] = xi ^ t;
            }
        }
        q >>= 1;
    }
    // Gray encode.
    x[1] ^= x[0];
    x[2] ^= x[1];
    let mut t = 0;
    q = m;
    while q > 1 {
        if x[2] & q != 0 {
            t ^= q - 1;
        }
        q >>= 1;
    }
    for xi in &mut x {
        *xi ^= t;
    }
    // Interleave the transposed bits, most significant first.
    let mut key = 0u64;
    for b in (0..BITS).rev() {
        for xi in x {
            key = (key << 1) | u64::from((xi >> b) & 1);
        }
    }
    key
}

/// The vertices in BRIO order, Hilbert order inside each round, ties by vertex index.
pub(super) fn brio_hilbert(points: &[Point3], vertices: &[u32]) -> Vec<u32> {
    let coords = |v: u32| points.get(v as usize).map(Point3::coords);
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for c in vertices.iter().filter_map(|&v| coords(v)) {
        for k in 0..3 {
            lo[k] = lo[k].min(c[k]);
            hi[k] = hi[k].max(c[k]);
        }
    }
    let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f64, f64::max);
    let top = f64::from((1u32 << BITS) - 1);
    let scale = if extent > 0.0 && extent.is_finite() { top / extent } else { 0.0 };
    let mut keyed: Vec<(u32, u64, u32)> = vertices
        .iter()
        .map(|&v| {
            let c = coords(v).unwrap_or_default();
            // `as` saturates; the clamp guards the last rounding step.
            let q = std::array::from_fn(|k| (((c[k] - lo[k]) * scale) as u32).min((1 << BITS) - 1));
            let level = mix(u64::from(v)).trailing_ones().min(MAX_LEVEL);
            (MAX_LEVEL - level, hilbert(q), v)
        })
        .collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, _, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hilbert_visits_neighbouring_cells_in_order() {
        // On a 2^BITS grid the curve over the first 8³ block is a walk of unit steps.
        let mut cells: Vec<(u64, [u32; 3])> = Vec::new();
        for x in 0..8 {
            for y in 0..8 {
                for z in 0..8 {
                    cells.push((hilbert([x, y, z]), [x, y, z]));
                }
            }
        }
        cells.sort_unstable();
        for w in cells.windows(2) {
            assert_eq!(w[1].0, w[0].0 + 1, "the 8³ block is a prefix of the curve");
            let d: u32 = (0..3).map(|k| w[0].1[k].abs_diff(w[1].1[k])).sum();
            assert_eq!(d, 1, "{:?} -> {:?}", w[0].1, w[1].1);
        }
    }

    #[test]
    fn order_is_a_permutation_with_geometric_rounds() {
        let points: Vec<Point3> = (0..10_000).map(|i| Point3::new(f64::from(i % 97), f64::from(i % 89), f64::from(i % 83)).unwrap()).collect();
        let vertices: Vec<u32> = (0..10_000).collect();
        let order = brio_hilbert(&points, &vertices);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vertices);
        let last_round = vertices.iter().filter(|&&v| mix(u64::from(v)).trailing_ones() == 0).count();
        assert!((4_500..5_500).contains(&last_round), "{last_round}");
        assert_eq!(order, brio_hilbert(&points, &vertices), "deterministic");
    }
}
