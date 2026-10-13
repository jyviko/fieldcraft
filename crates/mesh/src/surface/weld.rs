//! Vertex welding in input order: each vertex joins the earliest representative within the
//! tolerance, or becomes one. Lookups go through a hash map, but nothing is ever iterated out of
//! it, so the result does not depend on hashing.

use std::collections::HashMap;

use super::{MAX_RELATIVE_WELD, index};
use crate::predicates::MIN_MAGNITUDE;
use crate::vec3::{dot, norm, sub};
use crate::{Error, Result};

pub(crate) struct Welded {
    pub points: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub source_vertex: Vec<u32>,
    pub source_triangle: Vec<u32>,
}

/// Weld the vertices `triangles` use (indices already checked) within the larger of `relative`
/// × the bounding-box diagonal and `absolute`, snap coordinates below [`MIN_MAGNITUDE`] to zero
/// and drop triangles that collapse. A distance above [`MAX_RELATIVE_WELD`] of the diagonal is an
/// error; one below 1e-15 of it welds identical coordinates only.
pub(crate) fn weld(positions: &[[f64; 3]], triangles: &[[u32; 3]], relative: f64, absolute: f64) -> Result<Welded> {
    let mut used = vec![false; positions.len()];
    for v in triangles.iter().flatten() {
        if let Some(u) = used.get_mut(*v as usize) {
            *u = true;
        }
    }
    let referenced = || positions.iter().zip(&used).enumerate().filter(|(_, (_, u))| **u).map(|(v, (p, _))| (v, *p));
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for (_, p) in referenced() {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let diagonal = norm(sub(hi, lo));
    let tol = (relative * diagonal).max(absolute);
    if tol > MAX_RELATIVE_WELD * diagonal {
        return Err(Error::Invalid(format!(
            "weld distance {tol} is more than {MAX_RELATIVE_WELD} of the surface's size ({diagonal}): it would merge distinct vertices"
        )));
    }

    let mut map = vec![u32::MAX; positions.len()];
    let mut points: Vec<[f64; 3]> = Vec::new();
    let mut source_vertex = Vec::new();
    let mut add = |v: usize, p: [f64; 3], points: &mut Vec<[f64; 3]>| {
        let w = index(points.len());
        points.push(p);
        source_vertex.push(index(v));
        w
    };
    if tol >= 1e-15 * diagonal && tol > 0.0 {
        // Cells of side `tol`: a vertex's representatives are in its cell or the 26 around it.
        // Coordinates over the cell size stay below 1e15, exact in i64.
        let cell = |p: [f64; 3]| [0, 1, 2].map(|k| ((p[k] - lo[k]) / tol).floor() as i64);
        let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
        for (v, p) in referenced() {
            let c = cell(p);
            let mut best: Option<u32> = None;
            for dx in -1..=1_i64 {
                for dy in -1..=1_i64 {
                    for dz in -1..=1_i64 {
                        let key = [c[0].saturating_add(dx), c[1].saturating_add(dy), c[2].saturating_add(dz)];
                        for &w in grid.get(&key).into_iter().flatten() {
                            let Some(&q) = points.get(w as usize) else { continue };
                            let d = sub(p, q);
                            if dot(d, d) <= tol * tol && best.is_none_or(|b| w < b) {
                                best = Some(w);
                            }
                        }
                    }
                }
            }
            let w = match best {
                Some(w) => w,
                None => {
                    let w = add(v, p, &mut points);
                    grid.entry(c).or_default().push(w);
                    w
                }
            };
            if let Some(m) = map.get_mut(v) {
                *m = w;
            }
        }
    } else {
        // Identical coordinates only (`+ 0.0` turns -0 into +0).
        let mut seen: HashMap<[u64; 3], u32> = HashMap::new();
        for (v, p) in referenced() {
            let key = p.map(|x| (x + 0.0).to_bits());
            let w = match seen.get(&key) {
                Some(&w) => w,
                None => {
                    let w = add(v, p, &mut points);
                    seen.insert(key, w);
                    w
                }
            };
            if let Some(m) = map.get_mut(v) {
                *m = w;
            }
        }
    }
    for x in points.iter_mut().flatten() {
        if x.abs() < MIN_MAGNITUDE {
            *x = 0.0;
        }
    }

    let mut out = Vec::with_capacity(triangles.len());
    let mut source_triangle = Vec::with_capacity(triangles.len());
    for (t, tri) in triangles.iter().enumerate() {
        let w = tri.map(|v| map.get(v as usize).copied().unwrap_or(u32::MAX));
        if w[0] != w[1] && w[1] != w[2] && w[2] != w[0] {
            out.push(w);
            source_triangle.push(index(t));
        }
    }
    Ok(Welded { points, triangles: out, source_vertex, source_triangle })
}
