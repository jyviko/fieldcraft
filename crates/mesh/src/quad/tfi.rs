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
//! with `D = 3`, and [`transfinite_volume`] fills a block from its six face grids.

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
    grid(bottom, right, top, left, None)
}

/// [`transfinite_grid`] blended by given parameters (`u` along bottom and top, `v` along left and
/// right, each from 0 to 1) instead of the sides' arc lengths: the Coons patch of a map whose
/// sides are parametrised by them (a graded mapped block's lattice).
pub fn transfinite_grid_blended<const D: usize>(
    bottom: &[[f64; D]],
    right: &[[f64; D]],
    top: &[[f64; D]],
    left: &[[f64; D]],
    u: &[f64],
    v: &[f64],
) -> Result<Vec<[f64; D]>> {
    if u.len() != bottom.len() || v.len() != left.len() {
        return Err(Error::Invalid("one blending parameter per side node is needed".into()));
    }
    grid(bottom, right, top, left, Some((u, v)))
}

fn grid<const D: usize>(
    bottom: &[[f64; D]],
    right: &[[f64; D]],
    top: &[[f64; D]],
    left: &[[f64; D]],
    blend: Option<(&[f64], &[f64])>,
) -> Result<Vec<[f64; D]>> {
    let (nu, nv) = (bottom.len().saturating_sub(1), left.len().saturating_sub(1));
    if nu == 0 || nv == 0 || top.len() != bottom.len() || right.len() != left.len() {
        return Err(Error::Invalid(format!(
            "opposite sides need the same number of divisions (at least 1): bottom {nu}, top {}, left {nv}, right {}",
            top.len().saturating_sub(1),
            right.len().saturating_sub(1)
        )));
    }
    let count = (nu + 1).checked_mul(nv + 1).filter(|&c| c <= MAX_GRID_NODES);
    let count = count.ok_or_else(|| Error::Limit(format!("a {nu} × {nv} grid exceeds {MAX_GRID_NODES} nodes")))?;
    for side in [bottom, right, top, left] {
        fieldcraft_model::check_coordinates(side.as_flattened())?;
    }
    let corner = |s: &[[f64; D]], last: bool| if last { s.last().copied() } else { s.first().copied() };
    let (p00, p10, p01, p11) = match (corner(bottom, false), corner(bottom, true), corner(top, false), corner(top, true)) {
        (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
        _ => return Err(Error::Invalid("empty side".into())),
    };
    if corner(left, false) != Some(p00) || corner(right, false) != Some(p10) || corner(left, true) != Some(p01) || corner(right, true) != Some(p11) {
        return Err(Error::Invalid("the sides of a mapped region must share their corner points".into()));
    }
    let (ub, ut, vl, vr) = match blend {
        Some((u, v)) => (u.to_vec(), u.to_vec(), v.to_vec(), v.to_vec()),
        None => (params(bottom, "bottom")?, params(top, "top")?, params(left, "left")?, params(right, "right")?),
    };
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

/// The index of lattice node `c = [i, j, k]` in a block lattice of `n[d] + 1` nodes per direction:
/// `(k·(n[1] + 1) + j)·(n[0] + 1) + i`.
pub fn lattice_index(n: [usize; 3], c: [usize; 3]) -> usize {
    (c[2] * (n[1] + 1) + c[1]) * (n[0] + 1) + c[0]
}

/// Fill the six faces of a block lattice ([`lattice_index`]) whose twelve edges are set: each face
/// is the transfinite grid of its four edges, blended by `blend` (one parameter list per
/// direction) or, without it, by the edges' arc lengths. `finish(d, side, grid)` sees each face's
/// grid before its interior nodes are stored (face `d = side · n[d]`, the grid running along the
/// other two directions in increasing order, the lower one fastest): a mesher can project them
/// onto its geometry. Edge nodes already set are kept.
pub fn transfinite_faces(
    x: &mut [Option<[f64; 3]>],
    n: [usize; 3],
    blend: Option<[&[f64]; 3]>,
    mut finish: impl FnMut(usize, usize, &mut [[f64; 3]]) -> Result<()>,
) -> Result<()> {
    for d in 0..3 {
        let (da, db) = match d {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let (na, nb) = (n[da], n[db]);
        for side in 0..2 {
            let at = |a: usize, b: usize| {
                let mut c = [0; 3];
                c[d] = side * n[d];
                c[da] = a;
                c[db] = b;
                lattice_index(n, c)
            };
            let line = |x: &[Option<[f64; 3]>], f: &dyn Fn(usize) -> usize, m: usize| -> Result<Vec<[f64; 3]>> {
                (0..=m)
                    .map(|q| x.get(f(q)).copied().flatten().ok_or_else(|| Error::Failed("transfinite faces: an edge node is missing".into())))
                    .collect()
            };
            let (bottom, top) = (line(x, &|a| at(a, 0), na)?, line(x, &|a| at(a, nb), na)?);
            let (left, right) = (line(x, &|b| at(0, b), nb)?, line(x, &|b| at(na, b), nb)?);
            let mut grid = match blend {
                Some(p) => transfinite_grid_blended(&bottom, &right, &top, &left, p[da], p[db])?,
                None => transfinite_grid(&bottom, &right, &top, &left)?,
            };
            finish(d, side, &mut grid)?;
            for b in 1..nb {
                for a in 1..na {
                    if let (Some(slot), Some(p)) = (x.get_mut(at(a, b)), grid.get(b * (na + 1) + a)) {
                        *slot = Some(*p);
                    }
                }
            }
        }
    }
    Ok(())
}

/// The trivariate transfinite interpolation of a block's interior from its boundary (W. J.
/// Gordon, L. C. Thiel, *Transfinite mappings and their application to grid generation*, Applied
/// Mathematics and Computation 10–11:171–233, 1982): faces blended linearly, minus edges blended
/// bilinearly, plus corners blended trilinearly. `x` is the `(n[0] + 1) × (n[1] + 1) × (n[2] + 1)`
/// lattice, node `(i, j, k)` at `(k·(n[1] + 1) + j)·(n[0] + 1) + i`, with every boundary node set;
/// `blend[d]` holds the blending parameter of each lattice index along direction `d`, from 0 to 1
/// (`i/n` for uniform blending). Interior nodes are set; boundary nodes are left as they are.
pub fn transfinite_volume(x: &mut [Option<[f64; 3]>], n: [usize; 3], blend: [&[f64]; 3]) -> Result<()> {
    let [ni, nj, nk] = n;
    let total = (ni + 1).checked_mul(nj + 1).and_then(|c| c.checked_mul(nk + 1));
    if total != Some(x.len()) || (0..3).any(|d| blend[d].len() != n[d] + 1) {
        return Err(Error::Invalid("a transfinite volume needs a full lattice and one blending parameter per index".into()));
    }
    let idx = |i: usize, j: usize, k: usize| lattice_index(n, [i, j, k]);
    let get = |x: &[Option<[f64; 3]>], i: usize, j: usize, k: usize| -> Result<[f64; 3]> {
        x.get(idx(i, j, k)).copied().flatten().ok_or_else(|| Error::Failed("transfinite volume: a boundary node is missing".into()))
    };
    let lin = |t: f64, a: [f64; 3], b: [f64; 3]| [0, 1, 2].map(|c| (1.0 - t) * a[c] + t * b[c]);
    let bil = |s: f64, t: f64, p00, p10, p01, p11| lin(t, lin(s, p00, p10), lin(s, p01, p11));
    for k in 1..nk {
        for j in 1..nj {
            for i in 1..ni {
                let at = |d: usize, q: usize| blend[d].get(q).copied().unwrap_or(0.0);
                let (u, v, w) = (at(0, i), at(1, j), at(2, k));
                let g = |i: usize, j: usize, k: usize| get(x, i, j, k);
                let faces = [lin(u, g(0, j, k)?, g(ni, j, k)?), lin(v, g(i, 0, k)?, g(i, nj, k)?), lin(w, g(i, j, 0)?, g(i, j, nk)?)];
                let edges = [
                    bil(v, w, g(i, 0, 0)?, g(i, nj, 0)?, g(i, 0, nk)?, g(i, nj, nk)?),
                    bil(u, w, g(0, j, 0)?, g(ni, j, 0)?, g(0, j, nk)?, g(ni, j, nk)?),
                    bil(u, v, g(0, 0, k)?, g(ni, 0, k)?, g(0, nj, k)?, g(ni, nj, k)?),
                ];
                let corners = lin(
                    w,
                    bil(u, v, g(0, 0, 0)?, g(ni, 0, 0)?, g(0, nj, 0)?, g(ni, nj, 0)?),
                    bil(u, v, g(0, 0, nk)?, g(ni, 0, nk)?, g(0, nj, nk)?, g(ni, nj, nk)?),
                );
                let p = [0, 1, 2].map(|c| faces.iter().map(|f| f[c]).sum::<f64>() - edges.iter().map(|e| e[c]).sum::<f64>() + corners[c]);
                if let Some(slot) = x.get_mut(idx(i, j, k)) {
                    *slot = Some(p);
                }
            }
        }
    }
    Ok(())
}
