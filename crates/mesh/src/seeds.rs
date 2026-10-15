//! Edge seeds as the meshers take them: a design edge's polyline with the count, bias and
//! constraint the model's seed gives it (`fieldcraft_model::seeds`).
//!
//! The meshers recover their own curves from the face-tagged boundary surface, so a seed is
//! matched to a mesher's curve by position: the curve's end points lie on the seed's end points
//! (either way round, which gives the orientation the bias is measured in) and each one's middle
//! lies on the other, within the tolerance of the two tessellations. The seed's node positions
//! are placed by arc length along the mesher's curve ([`place`]).

use fieldcraft_model::{Bias, SeedConstraint, seeds::spacing};

use crate::vec3::{P, dist, lerp, sub};
use crate::{Error, Result};

/// A seed on one design edge.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeSeed {
    /// The edge's name, for messages.
    pub name: String,
    /// The edge's polyline, start to end.
    pub points: Vec<[f64; 3]>,
    /// How far the polyline may lie from the exact edge (its tessellation's chordal tolerance).
    pub tolerance: f64,
    /// The divisions the seed asks for.
    pub count: usize,
    pub bias: Option<Bias>,
    pub constraint: SeedConstraint,
}

impl EdgeSeed {
    /// `n + 1` node positions as fractions of the arc length from the seed's start (reversed:
    /// from its end).
    pub fn fractions(&self, n: usize, reversed: bool) -> Vec<f64> {
        let f = spacing(u32::try_from(n).unwrap_or(u32::MAX), self.bias);
        if reversed { f.iter().rev().map(|x| 1.0 - x).collect() } else { f }
    }

    /// The seed in words: `F2:a|F2:b (8 exact)`.
    pub fn describe(&self) -> String {
        format!("{} ({} {})", self.name, self.count, self.constraint.name())
    }
}

fn length(poly: &[P]) -> f64 {
    poly.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// The point at fraction `t` of a polyline's arc length.
fn at_fraction(poly: &[P], t: f64) -> Option<P> {
    let total = length(poly);
    let target = t.clamp(0.0, 1.0) * total;
    let mut walked = 0.0;
    for w in poly.windows(2) {
        let l = dist(w[0], w[1]);
        if walked + l >= target && l > 0.0 {
            return Some(lerp(w[0], w[1], ((target - walked) / l).clamp(0.0, 1.0)));
        }
        walked += l;
    }
    poly.last().copied()
}

fn point_segment(p: P, a: P, b: P) -> f64 {
    let ab = sub(b, a);
    let l2 = crate::vec3::dot(ab, ab);
    let t = if l2 > 0.0 { (crate::vec3::dot(sub(p, a), ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, lerp(a, b, t))
}

fn point_polyline(p: P, poly: &[P]) -> f64 {
    match poly {
        [] => f64::INFINITY,
        [a] => dist(p, *a),
        _ => poly.windows(2).map(|w| point_segment(p, w[0], w[1])).fold(f64::INFINITY, f64::min),
    }
}

/// Whether polyline `curve` is the seed's edge within `tol`: `Some(reversed)` when it runs from
/// the seed's end to its start.
pub(crate) fn matches(seed: &EdgeSeed, curve: &[P], tol: f64) -> Option<bool> {
    let tol = tol + seed.tolerance;
    let (Some(&a), Some(&b), Some(&s0), Some(&s1)) = (curve.first(), curve.last(), seed.points.first(), seed.points.last()) else {
        return None;
    };
    let forward = dist(a, s0) <= tol && dist(b, s1) <= tol;
    let backward = dist(a, s1) <= tol && dist(b, s0) <= tol;
    if !(forward || backward) {
        return None;
    }
    // A closed edge (both ends at one point) is oriented by its first quarter.
    let reversed = if forward && backward {
        let q = at_fraction(curve, 0.25)?;
        let (f, r) = (at_fraction(&seed.points, 0.25)?, at_fraction(&seed.points, 0.75)?);
        dist(q, r) < dist(q, f)
    } else {
        backward
    };
    let (cm, sm) = (at_fraction(curve, 0.5)?, at_fraction(&seed.points, 0.5)?);
    (point_polyline(cm, &seed.points) <= tol && point_polyline(sm, curve) <= tol).then_some(reversed)
}

/// The seed (index, reversed) of each curve, the first that matches; two seeds of different
/// counts on one curve are an error naming both.
pub(crate) fn match_curves(seeds: &[EdgeSeed], curves: &[Vec<P>], tol: f64) -> Result<Vec<Option<(usize, bool)>>> {
    let mut out = vec![None; curves.len()];
    if seeds.is_empty() {
        return Ok(out);
    }
    for (c, curve) in curves.iter().enumerate() {
        for (i, s) in seeds.iter().enumerate() {
            let Some(rev) = matches(s, curve, tol) else { continue };
            match out.get(c).copied().flatten() {
                None => {
                    if let Some(slot) = out.get_mut(c) {
                        *slot = Some((i, rev));
                    }
                }
                Some((j, _)) => {
                    if let Some(o) = seeds.get(j)
                        && (o.count, o.bias, o.constraint) != (s.count, s.bias, s.constraint)
                    {
                        return Err(Error::Invalid(format!("the edge seeds {} and {} are on the same edge", o.describe(), s.describe())));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The points at the given arc-length fractions along `poly` (its end points exact).
pub(crate) fn place(poly: &[P], fractions: &[f64]) -> Result<Vec<P>> {
    let (Some(&first), Some(&last)) = (poly.first(), poly.last()) else {
        return Err(Error::Invalid("empty polyline".into()));
    };
    let n = fractions.len();
    fractions
        .iter()
        .enumerate()
        .map(|(k, &t)| {
            if k == 0 {
                Ok(first)
            } else if k + 1 == n {
                Ok(last)
            } else {
                at_fraction(poly, t).ok_or_else(|| Error::Failed("seed spacing walked off the curve".into()))
            }
        })
        .collect()
}

/// The element size a seed asks for at each of its nodes (the mean length of the elements on
/// either side), for size fields.
pub fn node_sizes(seed: &EdgeSeed) -> Vec<(P, f64)> {
    let f = seed.fractions(seed.count, false);
    let total = length(&seed.points);
    let Ok(pts) = place(&seed.points, &f) else { return Vec::new() };
    pts.iter()
        .enumerate()
        .filter_map(|(k, p)| {
            let before = k.checked_sub(1).and_then(|j| Some(f.get(k)? - f.get(j)?));
            let after = f.get(k + 1).zip(f.get(k)).map(|(b, a)| b - a);
            let h = match (before, after) {
                (Some(x), Some(y)) => 0.5 * (x + y),
                (Some(x), None) | (None, Some(x)) => x,
                (None, None) => return None,
            } * total;
            (h > 0.0 && h.is_finite()).then_some((*p, h))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use fieldcraft_model::BiasToward;

    use super::*;

    fn seed(points: Vec<P>) -> EdgeSeed {
        EdgeSeed {
            name: "e".into(),
            points,
            tolerance: 0.0,
            count: 3,
            bias: Some(Bias { ratio: 4.0, toward: BiasToward::Start }),
            constraint: SeedConstraint::Exact,
        }
    }

    #[test]
    fn curves_match_either_way_round_and_carry_the_orientation() {
        let s = seed(vec![[0.0; 3], [5.0, 0.0, 0.0], [10.0, 0.0, 0.0]]);
        assert_eq!(matches(&s, &[[0.0; 3], [10.0, 0.0, 0.0]], 1e-6), Some(false));
        assert_eq!(matches(&s, &[[10.0, 0.0, 0.0], [0.0; 3]], 1e-6), Some(true));
        // Same ends, another path.
        assert_eq!(matches(&s, &[[0.0; 3], [5.0, 3.0, 0.0], [10.0, 0.0, 0.0]], 1e-6), None);
        assert_eq!(matches(&s, &[[0.0; 3], [9.0, 0.0, 0.0]], 1e-6), None);
        // Reversed fractions mirror the spacing.
        let f = s.fractions(3, false);
        let r = s.fractions(3, true);
        assert!(f.iter().zip(r.iter().rev()).all(|(a, b)| (a - (1.0 - b)).abs() < 1e-12));
        let pts = place(&[[0.0; 3], [10.0, 0.0, 0.0]], &f).unwrap();
        assert!((pts[1][0] - 10.0 * f[1]).abs() < 1e-12);
        // Sizes: the mean of the neighbouring elements.
        let h = node_sizes(&s);
        assert_eq!(h.len(), 4);
        assert!((h[0].1 - 10.0 * f[1]).abs() < 1e-9);
    }
}
