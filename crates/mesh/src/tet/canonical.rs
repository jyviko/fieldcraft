//! The canonical form of a boundary triangle soup: the tetrahedral mesh of a part depends on its
//! geometry only, never on the order in which the CAD kernel lists its faces, triangles and
//! vertices.
//!
//! A kernel may enumerate the faces of one solid in a different order from run to run (its
//! internal maps are hash maps), which reorders the triangle soup and renumbers the face tags
//! while every coordinate stays the same. Welding keeps the first copy of a point it meets, and
//! every later stage visits vertices and faces in id order, so a reordered soup gives a different
//! mesh. [`CanonicalSoup`] sorts the triangles by their coordinates (rotated to start at their
//! smallest corner, which keeps their orientation), numbers the vertices by first use in that
//! order and the face tags by first appearance, and maps the tags back afterwards.

use std::collections::BTreeMap;

use crate::{Error, Result};

use super::QualityMesh;
use super::complex::NONE;
use super::geometry::AnalyticSurface;

/// A triangle soup in canonical order, and the original face tag of each canonical one.
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalSoup {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub tri_face: Vec<u32>,
    /// `faces[c]`: the original tag of canonical tag `c`.
    pub faces: Vec<u32>,
}

/// A coordinate as an integer whose order is the numeric order of the coordinate (−0 before +0;
/// the coordinates of a valid soup are finite).
fn ordered(x: f64) -> u64 {
    let b = x.to_bits();
    if b >> 63 == 1 { !b } else { b | (1 << 63) }
}

fn point_key(p: [f64; 3]) -> [u64; 3] {
    p.map(ordered)
}

impl CanonicalSoup {
    /// The canonical form of `triangles` over `positions`, each triangle on face `tri_face[i]`
    /// (all on face 0 when `tri_face` is empty).
    pub fn new(positions: &[[f64; 3]], triangles: &[[u32; 3]], tri_face: &[u32]) -> Result<CanonicalSoup> {
        if !tri_face.is_empty() && tri_face.len() != triangles.len() {
            return Err(Error::Invalid(format!("{} face tags for {} triangles", tri_face.len(), triangles.len())));
        }
        let point = |v: u32| positions.get(v as usize).copied().ok_or_else(|| Error::Invalid(format!("triangle vertex {v} does not exist")));
        // Each triangle rotated to start at its smallest corner, keyed by its corners' coordinates.
        let mut order: Vec<([[u64; 3]; 3], [u32; 3], u32)> = Vec::with_capacity(triangles.len());
        for (i, t) in triangles.iter().enumerate() {
            let keys = [point_key(point(t[0])?), point_key(point(t[1])?), point_key(point(t[2])?)];
            let first = (0..3).min_by(|&a, &b| keys[a].cmp(&keys[b])).unwrap_or(0);
            let r = |k: usize| (first + k) % 3;
            let tag = tri_face.get(i).copied().unwrap_or(0);
            order.push(([keys[r(0)], keys[r(1)], keys[r(2)]], [t[r(0)], t[r(1)], t[r(2)]], tag));
        }
        // Equal keys are the same triangle twice; the original tag breaks the tie only between
        // copies on different faces, which no valid soup has.
        order.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.2.cmp(&b.2)));
        let mut id = vec![NONE; positions.len()];
        let mut face_id: BTreeMap<u32, u32> = BTreeMap::new();
        let mut out = CanonicalSoup {
            positions: Vec::with_capacity(positions.len()),
            triangles: Vec::with_capacity(triangles.len()),
            tri_face: Vec::with_capacity(triangles.len()),
            faces: Vec::new(),
        };
        for (_, t, tag) in order {
            let mut nt = [0u32; 3];
            for (slot, v) in nt.iter_mut().zip(t) {
                let s = id.get_mut(v as usize).ok_or_else(|| Error::Invalid(format!("triangle vertex {v} does not exist")))?;
                if *s == NONE {
                    *s = out.positions.len() as u32;
                    out.positions.push(point(v)?);
                }
                *slot = *s;
            }
            let c = *face_id.entry(tag).or_insert_with(|| {
                out.faces.push(tag);
                out.faces.len() as u32 - 1
            });
            out.triangles.push(nt);
            out.tri_face.push(c);
        }
        Ok(out)
    }

    /// The canonical tag of an original face tag.
    pub fn canonical_tag(&self, original: u32) -> Option<u32> {
        self.faces.iter().position(|&f| f == original).map(|c| c as u32)
    }

    /// The exact surfaces by original tag, reordered by canonical tag.
    pub fn analytic(&self, by_original: &[Option<AnalyticSurface>]) -> Vec<Option<AnalyticSurface>> {
        self.faces.iter().map(|&f| by_original.get(f as usize).copied().flatten()).collect()
    }

    /// Give a mesh of the canonical soup its original face tags back.
    pub fn restore_tags(&self, mesh: &mut QualityMesh) {
        for b in &mut mesh.boundary {
            if let Some(&f) = self.faces.get(b.face as usize) {
                b.face = f;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tetrahedron's surface listed in two orders, with its vertices and faces renumbered,
    /// has one canonical form, and the tags map back.
    #[test]
    fn reordered_soups_have_one_canonical_form() {
        let p = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let t = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        let a = CanonicalSoup::new(&p, &t, &[10, 11, 12, 13]).unwrap();
        // Vertices reversed, triangles reversed and rotated, tags permuted.
        let q: Vec<[f64; 3]> = p.iter().rev().copied().collect();
        let map = |v: u32| 3 - v;
        let u: Vec<[u32; 3]> = t.iter().rev().map(|f| [map(f[1]), map(f[2]), map(f[0])]).collect();
        let b = CanonicalSoup::new(&q, &u, &[3, 2, 1, 0]).unwrap();
        assert_eq!((&a.positions, &a.triangles, &a.tri_face), (&b.positions, &b.triangles, &b.tri_face));
        for c in 0..4u32 {
            // Triangle k of `t` is on face 10 + k in a and, listed as u[3 − k], on face k in b.
            assert_eq!(a.faces[c as usize] - 10, b.faces[c as usize]);
            assert_eq!(a.canonical_tag(a.faces[c as usize]), Some(c));
        }
        // Orientation is kept: the enclosed volume has the same sign.
        let vol = |s: &CanonicalSoup| -> f64 {
            s.triangles
                .iter()
                .map(|f| {
                    let [x, y, z] = f.map(|v| s.positions[v as usize]);
                    x[0] * (y[1] * z[2] - y[2] * z[1]) - x[1] * (y[0] * z[2] - y[2] * z[0]) + x[2] * (y[0] * z[1] - y[1] * z[0])
                })
                .sum()
        };
        assert!(vol(&a) != 0.0 && vol(&a) == vol(&b));
    }

    #[test]
    fn hostile_soups_are_refused() {
        let p = [[0.0, 0.0, 0.0]];
        assert!(CanonicalSoup::new(&p, &[[0, 0, 7]], &[]).is_err());
        assert!(CanonicalSoup::new(&p, &[[0, 0, 0]], &[1, 2]).is_err());
        assert_eq!(CanonicalSoup::new(&p, &[], &[]).unwrap().triangles.len(), 0);
    }
}
