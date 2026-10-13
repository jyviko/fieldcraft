//! Rounding the Steiner points to doubles without inverting tetrahedra.
//!
//! The exact tetrahedralization is valid, but a Steiner point rounded to its nearest doubles is
//! off its segment by up to half an ulp per coordinate, which flips a tetrahedron whose volume is
//! below that scale (Diazzi et al. 2023, §5.1, §6.2: the nearly flat ones, typically four Steiner
//! points on nearly parallel boundary edges). For each tetrahedron that is not positive after
//! rounding, each of its Steiner points tries the doubles within [`REACH`] ulps of its rounded
//! coordinates, nearest first, and takes the first with which every tetrahedron around it is
//! positive (exact `orient3d` on the doubles). Surface vertices never move. The moves are a few
//! ulps, so boundary faces stay on their triangles to rounding precision. The number of passes is
//! capped; what is left is for the validity gate to report.
//!
//! This is the interim answer. The paper's own (§5.1) improves the exact mesh by flips that never
//! cross a facet before rounding, which removes most nearly flat tetrahedra instead of nudging
//! their vertices; it belongs with the flips of the quality stage.

use crate::predicates::{Point3, Sign, tet_volume_sign};
use crate::{Error, Result};

/// How far, in ulps per coordinate, a rounded Steiner point may move.
const REACH: i32 = 2;
/// Passes over the tetrahedra that are not positive.
const MAX_PASSES: usize = 4;

fn positive(points: &[[f64; 3]], t: &[u32; 4]) -> Result<bool> {
    let p = |v: u32| -> Result<Point3> {
        let c = points.get(v as usize).copied().ok_or_else(|| Error::Failed("rounding: vertex id out of range".into()))?;
        Point3::try_from(c)
    };
    Ok(tet_volume_sign(&p(t[0])?, &p(t[1])?, &p(t[2])?, &p(t[3])?) == Sign::Positive)
}

/// `x` moved by `k` ulps.
fn step(mut x: f64, k: i32) -> f64 {
    for _ in 0..k.unsigned_abs() {
        x = if k > 0 { x.next_up() } else { x.next_down() };
    }
    x
}

/// Offsets within [`REACH`], nearest (smallest total) first, in a fixed order.
fn offsets() -> Vec<[i32; 3]> {
    let r = -REACH..=REACH;
    let mut o: Vec<[i32; 3]> = r.clone().flat_map(|a| r.clone().flat_map(move |b| (-REACH..=REACH).map(move |c| [a, b, c]))).collect();
    o.retain(|d| d != &[0, 0, 0]);
    o.sort_by_key(|d| (d.iter().map(|k| k.abs()).sum::<i32>(), *d));
    o
}

/// Move Steiner points (ids `>= surface`) so every tetrahedron is positive where a few-ulp move
/// does it. The number of points moved.
pub(super) fn repair(points: &mut [[f64; 3]], tets: &[[u32; 4]], surface: usize) -> Result<usize> {
    let mut bad = Vec::new();
    for (i, t) in tets.iter().enumerate() {
        if !positive(points, t)? {
            bad.push(i);
        }
    }
    if bad.is_empty() {
        return Ok(0);
    }
    let mut around: Vec<Vec<u32>> = vec![Vec::new(); points.len()];
    for (i, t) in tets.iter().enumerate() {
        for &v in t {
            if (v as usize) >= surface
                && let Some(a) = around.get_mut(v as usize)
            {
                a.push(i as u32);
            }
        }
    }
    let offsets = offsets();
    let mut moved = vec![false; points.len()];
    for _ in 0..MAX_PASSES {
        let mut still = Vec::new();
        for &i in &bad {
            let t = *tets.get(i).ok_or_else(|| Error::Failed("rounding: tetrahedron id".into()))?;
            if positive(points, &t)? {
                continue;
            }
            let mut fixed = false;
            for v in t.into_iter().filter(|&v| (v as usize) >= surface) {
                let Some(origin) = points.get(v as usize).copied() else { continue };
                let incident = around.get(v as usize).cloned().unwrap_or_default();
                for d in &offsets {
                    let candidate: [f64; 3] = std::array::from_fn(|k| step(origin[k], d[k]));
                    if candidate.iter().any(|x| !crate::predicates::is_admissible(*x)) {
                        continue;
                    }
                    if let Some(p) = points.get_mut(v as usize) {
                        *p = candidate;
                    }
                    let mut ok = true;
                    for &j in &incident {
                        let u = tets.get(j as usize).ok_or_else(|| Error::Failed("rounding: tetrahedron id".into()))?;
                        if !positive(points, u)? {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        fixed = true;
                        if let Some(m) = moved.get_mut(v as usize) {
                            *m = true;
                        }
                        break;
                    }
                    if let Some(p) = points.get_mut(v as usize) {
                        *p = origin;
                    }
                }
                if fixed {
                    break;
                }
            }
            if !fixed {
                still.push(i);
            }
        }
        if still.is_empty() || still.len() == bad.len() {
            break;
        }
        bad = still;
    }
    Ok(moved.iter().filter(|m| **m).count())
}
