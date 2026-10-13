//! Vertex welding in input order: each vertex joins the earliest representative within the
//! tolerance, or becomes one. Lookups go through a hash map, but nothing is ever iterated out of
//! it, so the result does not depend on hashing.

use std::collections::HashMap;

use super::{Box3, MAX_RELATIVE_WELD, index};
use crate::predicates::MIN_MAGNITUDE;
use crate::vec3::{dot, sub};
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
    let mut bounds = Box3::EMPTY;
    for (_, p) in referenced() {
        bounds.add_point(p);
    }
    let diagonal = bounds.diagonal();
    let tol = (relative * diagonal).max(absolute);
    if tol > MAX_RELATIVE_WELD * diagonal {
        return Err(Error::Invalid(format!(
            "weld distance {tol} is more than {MAX_RELATIVE_WELD} of the surface's size ({diagonal}): it would merge distinct vertices"
        )));
    }

    let mut lookup = if tol >= 1e-15 * diagonal && tol > 0.0 {
        Lookup::Grid { tol, lo: bounds.lo, heads: HashMap::new(), next: Vec::new() }
    } else {
        Lookup::Exact(HashMap::new())
    };
    let mut map = vec![u32::MAX; positions.len()];
    let mut points: Vec<[f64; 3]> = Vec::new();
    let mut source_vertex = Vec::new();
    for (v, p) in referenced() {
        let w = match lookup.find(p, &points) {
            Some(w) => w,
            None => {
                let w = index(points.len());
                points.push(p);
                source_vertex.push(index(v));
                lookup.insert(p, w);
                w
            }
        };
        if let Some(m) = map.get_mut(v) {
            *m = w;
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

/// Representatives found so far.
enum Lookup {
    /// Identical coordinates only (keyed by bits; `+ 0.0` turns -0 into +0).
    Exact(HashMap<[u64; 3], u32>),
    /// Cells of side `tol`: a vertex's representatives are in its cell or the 26 around it.
    /// Coordinates over the cell size stay below 1e15 (the distance is at least 1e-15 of the
    /// diagonal), exact in i64. Each cell keeps the head of a chain through `next`.
    Grid { tol: f64, lo: [f64; 3], heads: HashMap<[i64; 3], u32>, next: Vec<u32> },
}

impl Lookup {
    fn cell(tol: f64, lo: [f64; 3], p: [f64; 3]) -> [i64; 3] {
        [0, 1, 2].map(|k| ((p[k] - lo[k]) / tol).floor() as i64)
    }

    /// The earliest representative within the distance of `p`.
    fn find(&self, p: [f64; 3], points: &[[f64; 3]]) -> Option<u32> {
        match self {
            Lookup::Exact(seen) => seen.get(&p.map(|x| (x + 0.0).to_bits())).copied(),
            Lookup::Grid { tol, lo, heads, next } => {
                let c = Self::cell(*tol, *lo, p);
                let mut best: Option<u32> = None;
                for dx in -1..=1_i64 {
                    for dy in -1..=1_i64 {
                        for dz in -1..=1_i64 {
                            let key = [c[0].saturating_add(dx), c[1].saturating_add(dy), c[2].saturating_add(dz)];
                            let mut w = heads.get(&key).copied().unwrap_or(u32::MAX);
                            while let Some(&q) = points.get(w as usize) {
                                let d = sub(p, q);
                                if dot(d, d) <= tol * tol && best.is_none_or(|b| w < b) {
                                    best = Some(w);
                                }
                                w = next.get(w as usize).copied().unwrap_or(u32::MAX);
                            }
                        }
                    }
                }
                best
            }
        }
    }

    /// Record `p` as representative `w` (the next index).
    fn insert(&mut self, p: [f64; 3], w: u32) {
        match self {
            Lookup::Exact(seen) => {
                seen.insert(p.map(|x| (x + 0.0).to_bits()), w);
            }
            Lookup::Grid { tol, lo, heads, next } => {
                let head = heads.insert(Self::cell(*tol, *lo, p), w);
                next.push(head.unwrap_or(u32::MAX));
            }
        }
    }
}
