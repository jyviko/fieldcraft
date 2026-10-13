//! Banded contour plots: a range of nodal values cut into equal bands, each drawn in one colour.
//!
//! A triangle with values at its corners is split exactly along the band boundaries (the
//! values vary linearly across it), so every piece lies in one band and the band edges are
//! sharp iso-lines on any renderer, with no shader support: the GPU viewport and the CPU
//! rasterizer draw the same flat-coloured pieces.

use crate::{Rgb, Vec3};

/// Equal bands from `min` to `max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bands {
    pub min: f64,
    pub max: f64,
    pub count: u32,
}

impl Bands {
    /// `count` bands over `[min, max]`; a range of one value (or none) is one band around it.
    pub fn new(min: f64, max: f64, count: u32) -> Bands {
        let count = count.max(1);
        if !(min.is_finite() && max.is_finite()) {
            return Bands { min: 0.0, max: 0.0, count };
        }
        Bands { min: min.min(max), max: max.max(min), count }
    }

    /// The band a value lies in (values outside the range go to the end bands); `None` for a
    /// value that is not a number.
    pub fn band(&self, v: f64) -> Option<u32> {
        if v.is_nan() {
            return None;
        }
        let span = self.max - self.min;
        if !(span > 0.0) {
            return Some(self.count / 2);
        }
        let k = ((v - self.min) / span * f64::from(self.count)).floor();
        Some(k.clamp(0.0, f64::from(self.count - 1)) as u32)
    }

    /// The value between band `k − 1` and band `k` (`level(0)` is `min`, `level(count)` `max`).
    pub fn level(&self, k: u32) -> f64 {
        self.min + (self.max - self.min) * f64::from(k) / f64::from(self.count)
    }
}

/// Split a triangle (corners `p`, values `v`) into pieces that each lie in one band, and hand
/// each piece (a convex polygon of 3 to 5 corners) to `out` with its band. A triangle with a
/// value that is not a number is handed whole, with no band.
pub fn split_triangle(p: [Vec3; 3], v: [f64; 3], bands: &Bands, out: &mut impl FnMut(&[Vec3], Option<u32>)) {
    let [Some(a), Some(b), Some(c)] = v.map(|x| bands.band(x)) else {
        out(&p, None);
        return;
    };
    let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
    if lo == hi {
        out(&p, Some(lo));
        return;
    }
    let poly: Vec<(Vec3, f64)> = p.into_iter().zip(v).collect();
    for k in lo..=hi {
        // The end bands also hold the values beyond the range.
        let below = if k == 0 { f64::NEG_INFINITY } else { bands.level(k) };
        let above = if k + 1 == bands.count { f64::INFINITY } else { bands.level(k + 1) };
        let piece = clip(&clip(&poly, below, true), above, false);
        if piece.len() >= 3 {
            let pts: Vec<Vec3> = piece.iter().map(|(q, _)| *q).collect();
            out(&pts, Some(k));
        }
    }
}

/// The part of a convex polygon where the value is at least (`keep_above`) or at most `level`
/// (Sutherland–Hodgman against one level).
fn clip(poly: &[(Vec3, f64)], level: f64, keep_above: bool) -> Vec<(Vec3, f64)> {
    if level.is_infinite() {
        return poly.to_vec();
    }
    let inside = |v: f64| if keep_above { v >= level } else { v <= level };
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (i, &(a, va)) in poly.iter().enumerate() {
        let Some(&(b, vb)) = poly.get((i + 1) % poly.len()) else { continue };
        if inside(va) {
            out.push((a, va));
        }
        if inside(va) != inside(vb) {
            let t = (level - va) / (vb - va);
            out.push((a + (b - a) * t, level));
        }
    }
    out
}

/// The colour of band `k` of `count`: the colour scale `stops` (low to high) sampled at the
/// band's middle.
pub fn band_color(stops: &[Rgb], k: u32, count: u32) -> Rgb {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else { return Rgb(128, 128, 128) };
    if stops.len() == 1 {
        return *first;
    }
    let count = count.max(1);
    let t = (f64::from(k.min(count - 1)) + 0.5) / f64::from(count) * (stops.len() - 1) as f64;
    let i = (t.floor() as usize).min(stops.len() - 2);
    let f = t - i as f64;
    let (Some(a), Some(b)) = (stops.get(i), stops.get(i + 1)) else { return *last };
    let mix = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * f).round().clamp(0.0, 255.0) as u8;
    Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(p: &[Vec3]) -> f64 {
        let mut n = Vec3::ZERO;
        for i in 1..p.len() - 1 {
            n = n + (p[i] - p[0]).cross(p[i + 1] - p[0]);
        }
        n.len() * 0.5
    }

    #[test]
    fn bands_and_levels() {
        let b = Bands::new(0.0, 10.0, 5);
        assert_eq!(
            (b.band(0.0), b.band(1.99), b.band(2.0), b.band(10.0), b.band(-5.0), b.band(99.0)),
            (Some(0), Some(0), Some(1), Some(4), Some(0), Some(4))
        );
        assert_eq!((b.level(0), b.level(5)), (0.0, 10.0));
        assert_eq!(b.band(f64::NAN), None);
        // One value: one band in the middle; reversed and non-finite ranges are made safe.
        assert_eq!(Bands::new(3.0, 3.0, 6).band(3.0), Some(3));
        assert_eq!(Bands::new(5.0, 1.0, 4), Bands::new(1.0, 5.0, 4));
        assert_eq!(Bands::new(f64::NAN, 1.0, 0).count, 1);
    }

    #[test]
    fn a_triangle_splits_along_the_levels_and_keeps_its_area() {
        let p = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 10.0, 0.0)];
        // The value is x: pieces are strips between x = 0, 2, 4, …
        let b = Bands::new(0.0, 10.0, 5);
        let mut pieces = Vec::new();
        split_triangle(p, [0.0, 10.0, 0.0], &b, &mut |q, k| pieces.push((q.to_vec(), k)));
        assert_eq!(pieces.iter().map(|(_, k)| k.unwrap()).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        let total: f64 = pieces.iter().map(|(q, _)| area(q)).sum();
        assert!((total - 50.0).abs() < 1e-9, "{total}");
        for (q, k) in &pieces {
            let k = k.unwrap() as f64;
            assert!(q.iter().all(|c| c.x >= 2.0 * k - 1e-9 && c.x <= 2.0 * k + 2.0 + 1e-9), "{q:?} in band {k}");
        }
        // A band-uniform triangle stays whole; a NaN value hands it over with no band.
        let mut n = 0;
        split_triangle(p, [0.5, 1.0, 1.5], &b, &mut |q, k| {
            n += 1;
            assert_eq!((q.len(), k), (3, Some(0)));
        });
        split_triangle(p, [0.5, f64::NAN, 1.5], &b, &mut |_, k| {
            n += 1;
            assert_eq!(k, None);
        });
        assert_eq!(n, 2);
    }

    #[test]
    fn band_colours_run_through_the_stops() {
        let stops = [Rgb(0, 0, 255), Rgb(0, 255, 0), Rgb(255, 0, 0)];
        assert_eq!(band_color(&stops, 0, 2), Rgb(0, 128, 128));
        assert_eq!(band_color(&stops, 1, 2), Rgb(128, 128, 0));
        let first = band_color(&stops, 0, 12);
        let last = band_color(&stops, 11, 12);
        assert!(first.2 > 200 && last.0 > 200, "{first:?} {last:?}");
        assert_eq!(band_color(&[], 0, 3), Rgb(128, 128, 128));
        assert_eq!(band_color(&stops, 99, 3), band_color(&stops, 2, 3));
    }
}
