//! Symbolic perturbation of the in-sphere and in-circle tests (Devillers & Teillaud 2011, after
//! Edelsbrunner & Mücke's Simulation of Simplicity).
//!
//! `insphere(a, b, c, d, e)` is the 5×5 determinant with rows `[p, |p|², 1]` (translate by `e`
//! and expand along the last column to see it). Lift every point by an infinitesimal
//! `|p|² + ε^k(p)`, where `k(p)` is the rank of `p` in decreasing lexicographic order
//! (`(x, y, z)`; the largest point gets the largest perturbation `ε¹`). The determinant is linear
//! in the lift column, so the perturbed value is
//!
//! `insphere + Σ_p ε^k(p) · C_p`,  `C_p = (-1)^(row(p) + 4) · orient3d(the other four points)`
//!
//! (the cofactor of `p`'s lift is a determinant with rows `[q, 1]`, which is `orient3d` of the
//! other points in their order). Its sign is the sign of the first nonzero term in order of
//! decreasing rank. Lifting by these weights is a regular triangulation of the same points, so
//! the decisions are consistent across calls, and a Bowyer–Watson cavity stays a star-shaped
//! ball. When `a, b, c, d` is a proper tetrahedron the term of `e` (`-orient3d(a, b, c, d)`) is
//! nonzero, so the result is never zero.
//!
//! The 2D version replaces `orient3d` by `orient2d` and the 5×5 by the 4×4 determinant.

use std::cmp::Ordering;

use super::indirect::{Axis, GenericPoint, generic_compare, generic_insphere, generic_orient3d};
use super::{Point2, Point3, Sign, incircle, insphere, orient2d, orient3d};

/// Lexicographic order on coordinates (total: `f64::total_cmp`).
fn lex<const N: usize>(p: &[f64; N], q: &[f64; N]) -> Ordering {
    p.iter().zip(q).map(|(a, b)| a.total_cmp(b)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
}

/// Argument positions sorted by decreasing lexicographic order of their points, ties (equal
/// points, which a triangulation never passes) by position.
fn ranked<const N: usize, const M: usize>(points: &[[f64; N]; M]) -> [usize; M] {
    let mut order: [usize; M] = std::array::from_fn(|i| i);
    order.sort_by(|&i, &j| match (points.get(i), points.get(j)) {
        (Some(p), Some(q)) => lex(q, p).then(i.cmp(&j)),
        _ => i.cmp(&j),
    });
    order
}

/// [`insphere`] with symbolic perturbation: never [`Sign::Zero`] when `a, b, c, d` is not
/// coplanar. Points must be pairwise distinct for the perturbation to be consistent (a
/// Delaunay triangulation never passes duplicates); with all five points coplanar it returns
/// `Zero`.
#[inline]
pub fn insphere_sos(a: &Point3, b: &Point3, c: &Point3, d: &Point3, e: &Point3) -> Sign {
    let exact = insphere(a, b, c, d, e);
    if exact.is_zero() { insphere_perturbed(a, b, c, d, e) } else { exact }
}

#[cold]
fn insphere_perturbed(a: &Point3, b: &Point3, c: &Point3, d: &Point3, e: &Point3) -> Sign {
    perturbed_cofactors([a, b, c, d, e], ranked(&[a.0, b.0, c.0, d.0, e.0]), orient3d)
}

/// The sign of the first nonzero lift cofactor, in rank order (`order`: argument positions by
/// decreasing rank). The single statement of the perturbation rule for every point type.
fn perturbed_cofactors<P>(points: [&P; 5], order: [usize; 5], orient: impl Fn(&P, &P, &P, &P) -> Sign) -> Sign {
    let [a, b, c, d, e] = points;
    for i in order {
        let [p, q, r, s] = match i {
            0 => [b, c, d, e],
            1 => [a, c, d, e],
            2 => [a, b, d, e],
            3 => [a, b, c, e],
            _ => [a, b, c, d],
        };
        let minor = orient(p, q, r, s);
        if !minor.is_zero() {
            // Row i + 1 (1-based), lift column 4: cofactor sign (-1)^(i + 5).
            return if i % 2 == 1 { minor } else { -minor };
        }
    }
    Sign::Zero
}

/// The lexicographic order of two points' exact coordinates (`x`, then `y`, then `z`): the rank
/// order of the perturbation, for explicit and implicit points alike. On explicit points it is
/// the order [`insphere_sos`] uses (callers admit `+0.0`, never `-0.0`).
pub fn generic_lex(p: &GenericPoint, q: &GenericPoint) -> Ordering {
    if let (Some(p), Some(q)) = (p.explicit(), q.explicit()) {
        return lex(&p.coords(), &q.coords());
    }
    Axis::ALL.into_iter().map(|a| generic_compare(p, q, a)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
}

/// [`insphere_sos`] on explicit or implicit points: the same lift by lexicographic rank, so a
/// triangulation built with [`insphere_sos`] stays consistent when implicit points are added.
/// Never [`Sign::Zero`] when `a, b, c, d` is not coplanar; points must be pairwise distinct.
pub fn generic_insphere_sos(a: &GenericPoint, b: &GenericPoint, c: &GenericPoint, d: &GenericPoint, e: &GenericPoint) -> Sign {
    if let (Some(a), Some(b), Some(c), Some(d), Some(e)) = (a.explicit(), b.explicit(), c.explicit(), d.explicit(), e.explicit()) {
        return insphere_sos(a, b, c, d, e);
    }
    let exact = generic_insphere(a, b, c, d, e);
    if !exact.is_zero() {
        return exact;
    }
    let points = [a, b, c, d, e];
    // Positions by decreasing rank, ties by position: an insertion sort, which needs no total
    // order guarantee from the comparator to stay in bounds.
    let mut order = [0usize, 1, 2, 3, 4];
    for i in 1..order.len() {
        let mut j = i;
        while j > 0 {
            let (Some(&x), Some(&y)) = (order.get(j - 1), order.get(j)) else { break };
            let (Some(px), Some(py)) = (points.get(x), points.get(y)) else { break };
            if generic_lex(py, px) != Ordering::Greater {
                break;
            }
            order.swap(j - 1, j);
            j -= 1;
        }
    }
    perturbed_cofactors(points, order, generic_orient3d)
}

/// [`incircle`] with symbolic perturbation: never [`Sign::Zero`] when `a, b, c` is not
/// collinear. Same conventions as [`insphere_sos`].
#[inline]
pub fn incircle_sos(a: &Point2, b: &Point2, c: &Point2, d: &Point2) -> Sign {
    let exact = incircle(a, b, c, d);
    if exact.is_zero() { incircle_perturbed(a, b, c, d) } else { exact }
}

#[cold]
fn incircle_perturbed(a: &Point2, b: &Point2, c: &Point2, d: &Point2) -> Sign {
    for i in ranked(&[a.0, b.0, c.0, d.0]) {
        let [p, q, r] = match i {
            0 => [b, c, d],
            1 => [a, c, d],
            2 => [a, b, d],
            _ => [a, b, c],
        };
        let minor = orient2d(p, q, r);
        if !minor.is_zero() {
            // Row i + 1 (1-based), lift column 3: cofactor sign (-1)^(i + 4).
            return if i % 2 == 0 { minor } else { -minor };
        }
    }
    Sign::Zero
}
