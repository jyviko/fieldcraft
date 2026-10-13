//! Mapped meshing of four-sided regions by transfinite interpolation.
//!
//! W. J. Gordon, C. A. Hall, *Construction of curvilinear co-ordinate systems and applications to
//! mesh generation*, IJNME 7(4):461–477, 1973: the bilinearly blended Coons map
//! `X(u, v) = (1−v) B + v T + (1−u) L + u R − [(1−u)(1−v) P00 + u(1−v) P10 + u v P11 + (1−u) v P01]`
//! from the four sides `B` (bottom), `R` (right), `T` (top), `L` (left) and their corners. The
//! discrete form uses the side nodes directly; the blending parameters of node `(i, j)` follow the
//! arc-length distribution of the sides, from the intersection of the straight parameter lines
//! `u = (1−v) u_B(i) + v u_T(i)` and `v = (1−u) v_L(j) + u v_R(j)` (the standard treatment of
//! unequal spacing on opposite sides, e.g. Thompson, Soni, Weatherill, *Handbook of Grid
//! Generation*, CRC 1999, ch. 3). The map is dimension-generic: faces of a block in space use it
//! with `D = 3`.

use crate::{Error, Result};

/// The most nodes of one mapped grid.
pub const MAX_GRID_NODES: usize = 1 << 24;

fn dist<const D: usize>(a: &[f64; D], b: &[f64; D]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt()
}

/// Normalized cumulative arc length of a side, from 0 to 1.
fn params<const D: usize>(side: &[[f64; D]], name: &str) -> Result<Vec<f64>> {
    let mut acc = Vec::with_capacity(side.len());
    let mut s = 0.0;
    acc.push(0.0);
    for w in side.windows(2) {
        if let [a, b] = w {
            s += dist(a, b);
        }
        acc.push(s);
    }
    if !(s > 0.0) || !s.is_finite() {
        return Err(Error::Invalid(format!("the {name} side has zero or non-finite length")));
    }
    let mut out: Vec<f64> = acc.iter().map(|x| x / s).collect();
    if let Some(l) = out.last_mut() {
        *l = 1.0;
    }
    Ok(out)
}

/// The transfinite grid of a four-sided region: `(nu + 1) × (nv + 1)` points, node `(i, j)` at
/// index `j·(nu + 1) + i`. `bottom` and `top` run in `+u` (each `nu + 1` points), `left` and
/// `right` in `+v` (each `nv + 1` points); the corners must match exactly (`bottom[0] =
/// left[0]`, `bottom[nu] = right[0]`, `top[0] = left[nv]`, `top[nu] = right[nv]`). Side nodes are
/// copied exactly into the grid.
pub fn transfinite_grid<const D: usize>(bottom: &[[f64; D]], right: &[[f64; D]], top: &[[f64; D]], left: &[[f64; D]]) -> Result<Vec<[f64; D]>> {
    let (nu, nv) = (bottom.len().saturating_sub(1), left.len().saturating_sub(1));
    if nu == 0 || nv == 0 || top.len() != bottom.len() || right.len() != left.len() {
        return Err(Error::Invalid(format!(
            "opposite sides need the same number of divisions (at least 1): bottom {}, top {}, left {}, right {}",
            bottom.len().saturating_sub(1),
            top.len().saturating_sub(1),
            left.len().saturating_sub(1),
            right.len().saturating_sub(1)
        )));
    }
    let count = (nu + 1).checked_mul(nv + 1).filter(|&c| c <= MAX_GRID_NODES);
    let count = count.ok_or_else(|| Error::Limit(format!("a {nu} × {nv} grid exceeds {MAX_GRID_NODES} nodes")))?;
    for side in [bottom, right, top, left] {
        modelcraft_model::check_coordinates(side.as_flattened())?;
    }
    let corner = |s: &[[f64; D]], last: bool| if last { s.last().copied() } else { s.first().copied() };
    let (p00, p10, p01, p11) = match (corner(bottom, false), corner(bottom, true), corner(top, false), corner(top, true)) {
        (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
        _ => return Err(Error::Invalid("empty side".into())),
    };
    if corner(left, false) != Some(p00) || corner(right, false) != Some(p10) || corner(left, true) != Some(p01) || corner(right, true) != Some(p11) {
        return Err(Error::Invalid("the sides of a mapped region must share their corner points".into()));
    }
    let (ub, ut, vl, vr) = (params(bottom, "bottom")?, params(top, "top")?, params(left, "left")?, params(right, "right")?);
    let mut grid = Vec::with_capacity(count);
    for j in 0..=nv {
        for i in 0..=nu {
            let (Some(&b), Some(&t), Some(&l), Some(&r)) = (bottom.get(i), top.get(i), left.get(j), right.get(j)) else {
                return Err(Error::Failed("grid index".into()));
            };
            let point = if j == 0 {
                b
            } else if j == nv {
                t
            } else if i == 0 {
                l
            } else if i == nu {
                r
            } else {
                let (u0, u1, v0, v1) = (
                    ub.get(i).copied().unwrap_or(0.0),
                    ut.get(i).copied().unwrap_or(0.0),
                    vl.get(j).copied().unwrap_or(0.0),
                    vr.get(j).copied().unwrap_or(0.0),
                );
                let (du, dv) = (u1 - u0, v1 - v0);
                // |du|, |dv| < 1 at interior nodes, so the denominator is positive.
                let u = (u0 + v0 * du) / (1.0 - du * dv);
                let v = v0 + u * dv;
                let mut x = [0.0; D];
                for (k, xk) in x.iter_mut().enumerate() {
                    *xk = (1.0 - v) * b[k] + v * t[k] + (1.0 - u) * l[k] + u * r[k]
                        - ((1.0 - u) * (1.0 - v) * p00[k] + u * (1.0 - v) * p10[k] + u * v * p11[k] + (1.0 - u) * v * p01[k]);
                }
                x
            };
            grid.push(point);
        }
    }
    Ok(grid)
}
