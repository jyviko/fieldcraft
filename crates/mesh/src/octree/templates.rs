//! The 3-refinement transition templates: how a leaf of the balanced tree is cut into
//! hexahedra so that it conforms to finer neighbours.
//!
//! Every vertex of a leaf is *marked* when a finer leaf touches it. Under strong balance the
//! marked set of a leaf is one of five patterns (after the closure in [`super::grid`] has split
//! every leaf that had another one), each with one template:
//!
//! | marks | template | hexahedra |
//! |---|---|---|
//! | none | the leaf itself | 1 |
//! | one corner | corner cube and three blocks fanning to the far corner | 4 |
//! | the two ends of an edge | three cubes along the edge and two four-hex slabs | 11 |
//! | the four corners of a face | nine cubes on the face, nine above them, four on top | 22 |
//! | all eight | the 3 × 3 × 3 split | 27 |
//!
//! A face of a leaf is cut according to its own marks only (none, one corner, one edge, all
//! four), and an edge into thirds at its marked ends, so two leaves sharing a face cut it the same
//! way: the mesh is conforming by construction. This is the node-marking form of Schneiders'
//! 3-refinement (R. Schneiders, *Refining quadrilateral and hexahedral element meshes*, NUMGRID
//! 1996; Y. Ito, A. M. Shih, B. K. Soni, *Octree-based reasonable-quality hexahedral mesh
//! generation using a new set of refinement templates*, IJNME 77(13):1809–1833, 2009). The
//! templates are our own construction from those papers' description: the face template's top
//! block is the 3 × 3 → 1 transition traced from its dual sheets, the edge template is two
//! copies of the planar 3 → 1 transition swept across the leaf, and the tests prove every
//! template's faces, volume and validity.
//!
//! Coordinates are integers in sixths of the leaf's edge, so every vertex shared with a neighbour
//! lies on the global integer lattice of [`super::grid`] and coincident vertices merge exactly.

/// A point of a leaf in sixths of its edge, `0..=6` per axis.
pub(crate) type Local = [i64; 3];
/// A hexahedron in the model's node order (corners 1-2-3-4 then 5-6-7-8 above them).
pub(crate) type Hex = [Local; 8];

/// The marked-corner pattern of a leaf, up to symmetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pattern {
    None,
    Corner,
    Edge,
    Face,
    Full,
}

/// A symmetry of the cube: coordinate `k` of the image is coordinate `perm[k]` of the point,
/// reflected (`6 − v`) when `flip[k]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sym {
    perm: [usize; 3],
    flip: [bool; 3],
}

impl Sym {
    pub(crate) fn apply(&self, p: Local) -> Local {
        let mut q = [0; 3];
        for (k, out) in q.iter_mut().enumerate() {
            let v = p[self.perm[k]];
            *out = if self.flip[k] { 6 - v } else { v };
        }
        q
    }
}

/// The 48 symmetries in a fixed order.
pub(crate) fn symmetries() -> Vec<Sym> {
    let perms = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    let mut out = Vec::with_capacity(48);
    for perm in perms {
        for bits in 0..8u8 {
            out.push(Sym { perm, flip: [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0] });
        }
    }
    out
}

/// The leaf corner `c` (bit 0: x, bit 1: y, bit 2: z) in sixths.
pub(crate) fn corner(c: usize) -> Local {
    [(c & 1) as i64 * 6, ((c >> 1) & 1) as i64 * 6, ((c >> 2) & 1) as i64 * 6]
}

/// The corner index of a corner point.
fn corner_index(p: Local) -> usize {
    usize::from(p[0] == 6) | (usize::from(p[1] == 6) << 1) | (usize::from(p[2] == 6) << 2)
}

/// The image of a corner mask under a symmetry.
fn map_mask(sym: &Sym, mask: u8) -> u8 {
    (0..8).filter(|c| mask & (1 << c) != 0).fold(0u8, |m, c| m | (1 << corner_index(sym.apply(corner(c)))))
}

/// The pattern of a corner mask, or `None` when no template fits it.
pub(crate) fn classify(mask: u8) -> Option<Pattern> {
    let corners: Vec<Local> = (0..8).filter(|c| mask & (1 << c) != 0).map(corner).collect();
    let shared = |n: usize| (0..3).filter(|&k| corners.iter().all(|p| p[k] == corners[0][k])).count() == n;
    match corners.len() {
        0 => Some(Pattern::None),
        1 => Some(Pattern::Corner),
        2 if shared(2) => Some(Pattern::Edge),
        4 if shared(1) => Some(Pattern::Face),
        8 => Some(Pattern::Full),
        _ => None,
    }
}

/// The canonical mask of a pattern: corner 0; edge 0–1 (along x); face z = 0.
fn canonical_mask(p: Pattern) -> u8 {
    match p {
        Pattern::None => 0,
        Pattern::Corner => 0b1,
        Pattern::Edge => 0b11,
        Pattern::Face => 0b1111,
        Pattern::Full => 0xff,
    }
}

/// The first symmetry (in [`symmetries`] order) taking the pattern's canonical mask to `mask`.
pub(crate) fn orientation(syms: &[Sym], pattern: Pattern, mask: u8) -> Option<Sym> {
    let c = canonical_mask(pattern);
    syms.iter().find(|s| map_mask(s, c) == mask).copied()
}

/// Thirds to sixths.
fn t(x: i64, y: i64, z: i64) -> Local {
    [2 * x, 2 * y, 2 * z]
}

/// A thirds-aligned cube with lower corner `(x, y, z)` (in thirds) and edge `e` thirds.
fn cube(x: i64, y: i64, z: i64, e: i64) -> Hex {
    let (a, b) = (t(x, y, z), t(x + e, y + e, z + e));
    [
        [a[0], a[1], a[2]],
        [b[0], a[1], a[2]],
        [b[0], b[1], a[2]],
        [a[0], b[1], a[2]],
        [a[0], a[1], b[2]],
        [b[0], a[1], b[2]],
        [b[0], b[1], b[2]],
        [a[0], b[1], b[2]],
    ]
}

/// The planar 3 → 1 transition in `(x, s)`: x in thirds, `s` in halves of the transition height,
/// three segments at `s = 0` and one at `s = 2`.
const THREE_TO_ONE: [[(i64, i64); 4]; 4] =
    [[(0, 0), (1, 0), (1, 1), (0, 2)], [(1, 0), (2, 0), (2, 1), (1, 1)], [(2, 0), (3, 0), (3, 2), (2, 1)], [(1, 1), (2, 1), (3, 2), (0, 2)]];

/// The canonical template of a pattern, in sixths of the leaf.
pub(crate) fn template(p: Pattern) -> Vec<Hex> {
    match p {
        Pattern::None => vec![cube(0, 0, 0, 3)],
        Pattern::Full => (0..27).map(|k| cube(k % 3, (k / 3) % 3, k / 9, 1)).collect(),
        Pattern::Corner => vec![
            cube(0, 0, 0, 1),
            [t(1, 0, 0), t(3, 0, 0), t(3, 3, 0), t(1, 1, 0), t(1, 0, 1), t(3, 0, 3), t(3, 3, 3), t(1, 1, 1)],
            [t(0, 1, 0), t(1, 1, 0), t(3, 3, 0), t(0, 3, 0), t(0, 1, 1), t(1, 1, 1), t(3, 3, 3), t(0, 3, 3)],
            [t(0, 0, 1), t(1, 0, 1), t(1, 1, 1), t(0, 1, 1), t(0, 0, 3), t(3, 0, 3), t(3, 3, 3), t(0, 3, 3)],
        ],
        Pattern::Edge => {
            let mut out: Vec<Hex> = (0..3).map(|i| cube(i, 0, 0, 1)).collect();
            // Two slabs over the edge column's faces y = 1 and z = 1, each the 3 → 1 transition
            // swept from the leaf face to the diagonal plane y = z.
            for q in THREE_TO_ONE {
                let b = |(x, s): (i64, i64), w: i64| t(x, 1 + s, w * (1 + s));
                let c = |(x, s): (i64, i64), w: i64| t(x, w * (1 + s), 1 + s);
                out.push([b(q[0], 0), b(q[1], 0), b(q[2], 0), b(q[3], 0), b(q[0], 1), b(q[1], 1), b(q[2], 1), b(q[3], 1)]);
                out.push([c(q[0], 0), c(q[1], 0), c(q[2], 0), c(q[3], 0), c(q[0], 1), c(q[1], 1), c(q[2], 1), c(q[3], 1)]);
            }
            out
        }
        Pattern::Face => {
            let mut out: Vec<Hex> = (0..9).map(|k| cube(k % 3, k / 3, 0, 1)).collect();
            // The layer above: its top is the corners raised to the leaf's top, the side nodes of
            // the 3 → 1 transitions at two thirds, the inside nodes at half height.
            let top = |a: i64, b: i64| -> Local {
                let edge = |v: i64| v == 0 || v == 3;
                match (edge(a), edge(b)) {
                    (true, true) => [2 * a, 2 * b, 6],
                    (false, false) => [2 * a, 2 * b, 3],
                    _ => [2 * a, 2 * b, 4],
                }
            };
            for k in 0..9 {
                let (i, j) = (k % 3, k / 3);
                out.push([
                    t(i, j, 1),
                    t(i + 1, j, 1),
                    t(i + 1, j + 1, 1),
                    t(i, j + 1, 1),
                    top(i, j),
                    top(i + 1, j),
                    top(i + 1, j + 1),
                    top(i, j + 1),
                ]);
            }
            // The four on top: the y-arch (low), the x-arch (high) and the two side blocks.
            out.push([[2, 2, 3], [4, 2, 3], [4, 4, 3], [2, 4, 3], [2, 0, 4], [4, 0, 4], [4, 6, 4], [2, 6, 4]]);
            out.push([[2, 0, 4], [4, 0, 4], [4, 6, 4], [2, 6, 4], [0, 0, 6], [6, 0, 6], [6, 6, 6], [0, 6, 6]]);
            out.push([[2, 2, 3], [2, 4, 3], [2, 6, 4], [2, 0, 4], [0, 2, 4], [0, 4, 4], [0, 6, 6], [0, 0, 6]]);
            out.push([[4, 2, 3], [4, 4, 3], [4, 6, 4], [4, 0, 4], [6, 2, 4], [6, 4, 4], [6, 6, 6], [6, 0, 6]]);
            out
        }
    }
}

/// Six times the signed volume of the corner tetrahedron at node 0 (exact on the lattice).
pub(crate) fn corner0_orientation(h: &[[i64; 3]; 8]) -> i128 {
    let e = |j: usize| [0, 1, 2].map(|k| i128::from(h[j][k] - h[0][k]));
    let (a, b, c) = (e(1), e(3), e(4));
    a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])
}

/// The hexahedron with its orientation reversed (the same cell, mirrored node order).
pub(crate) fn reversed<T: Copy>(h: [T; 8]) -> [T; 8] {
    [h[0], h[3], h[2], h[1], h[4], h[7], h[6], h[5]]
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::quality::{Validity, element_validity};
    use fieldcraft_model::ElementKind;

    /// The face of the leaf a quad lies on (axis, value), if any.
    fn on_leaf_face(q: &[Local; 4]) -> Option<(usize, i64)> {
        (0..3).find_map(|k| [0, 6].into_iter().find(|&v| q.iter().all(|p| p[k] == v)).map(|v| (k, v)))
    }

    fn sorted(mut q: [Local; 4]) -> [Local; 4] {
        q.sort();
        q
    }

    /// The quads a leaf face must carry for its four corner marks, as sorted vertex sets, on the
    /// face `(axis, value)`.
    fn face_template(axis: usize, value: i64, marks: [bool; 4]) -> Vec<[Local; 4]> {
        // Face coordinates (u, v) are the other two axes in increasing order; corners in order
        // (0,0), (6,0), (0,6), (6,6).
        let (ua, va) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let lift = |u: i64, v: i64| {
            let mut p = [0; 3];
            p[axis] = value;
            p[ua] = u;
            p[va] = v;
            p
        };
        // Build in a canonical frame, then map the 2D symmetry that takes the canonical marks to
        // these marks.
        let count = marks.iter().filter(|m| **m).count();
        let canon: Vec<[(i64, i64); 4]> = match count {
            0 => vec![[(0, 0), (6, 0), (6, 6), (0, 6)]],
            1 => vec![[(0, 0), (2, 0), (2, 2), (0, 2)], [(2, 0), (6, 0), (6, 6), (2, 2)], [(0, 2), (2, 2), (6, 6), (0, 6)]],
            2 => {
                let mut v: Vec<[(i64, i64); 4]> = (0..3).map(|i| [(2 * i, 0), (2 * i + 2, 0), (2 * i + 2, 2), (2 * i, 2)]).collect();
                v.extend([
                    [(0, 2), (2, 2), (2, 4), (0, 6)],
                    [(2, 2), (4, 2), (4, 4), (2, 4)],
                    [(4, 2), (6, 2), (6, 6), (4, 4)],
                    [(2, 4), (4, 4), (6, 6), (0, 6)],
                ]);
                v
            }
            _ => (0..9)
                .map(|k| {
                    [(2 * (k % 3), 2 * (k / 3)), (2 * (k % 3) + 2, 2 * (k / 3)), (2 * (k % 3) + 2, 2 * (k / 3) + 2), (2 * (k % 3), 2 * (k / 3) + 2)]
                })
                .collect(),
        };
        let canon_marks: [bool; 4] = match count {
            0 => [false; 4],
            1 => [true, false, false, false],
            2 => [true, true, false, false],
            _ => [true; 4],
        };
        let maps: [fn(i64, i64) -> (i64, i64); 8] = [
            |u, v| (u, v),
            |u, v| (6 - u, v),
            |u, v| (u, 6 - v),
            |u, v| (6 - u, 6 - v),
            |u, v| (v, u),
            |u, v| (6 - v, u),
            |u, v| (v, 6 - u),
            |u, v| (6 - v, 6 - u),
        ];
        let corners = [(0, 0), (6, 0), (0, 6), (6, 6)];
        let idx = |c: (i64, i64)| corners.iter().position(|&x| x == c).unwrap();
        let m =
            maps.iter().find(|m| (0..4).all(|c| canon_marks[c] == marks[idx(m(corners[c].0, corners[c].1))])).expect("a face symmetry for the marks");
        canon.iter().map(|q| sorted(q.map(|(u, v)| m(u, v)).map(|(u, v)| lift(u, v)))).collect()
    }

    fn face_corner_marks(axis: usize, value: i64, mask: u8) -> [bool; 4] {
        let (ua, va) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        [(0, 0), (6, 0), (0, 6), (6, 6)].map(|(u, v)| {
            let mut p = [0; 3];
            p[axis] = value;
            p[ua] = u;
            p[va] = v;
            mask & (1 << corner_index(p)) != 0
        })
    }

    /// Every template under every symmetry: valid elements, the leaf's volume, closed (each inner
    /// face shared by two elements) and, on each leaf face, exactly the face template of that
    /// face's marks.
    #[test]
    fn templates_are_valid_closed_and_conform_to_their_face_marks() {
        let syms = symmetries();
        for pattern in [Pattern::None, Pattern::Corner, Pattern::Edge, Pattern::Face, Pattern::Full] {
            for sym in &syms {
                let mask = map_mask(sym, canonical_mask(pattern));
                assert_eq!(classify(mask), Some(pattern));
                let hexes: Vec<Hex> = template(pattern)
                    .into_iter()
                    .map(|h| {
                        let h = h.map(|p| sym.apply(p));
                        if corner0_orientation(&h) < 0 { reversed(h) } else { h }
                    })
                    .collect();
                let mut volume = 0.0;
                let mut faces: HashMap<[Local; 4], usize> = HashMap::new();
                for h in &hexes {
                    let xyz: Vec<[f64; 3]> = h.iter().map(|p| p.map(|v| v as f64)).collect();
                    let check = element_validity(ElementKind::Hex8, &xyz).unwrap();
                    assert_eq!(check.validity, Validity::Valid, "{pattern:?} {sym:?} {h:?}");
                    volume += check.volume;
                    for f in ElementKind::Hex8.faces() {
                        *faces.entry(sorted([h[f[0]], h[f[1]], h[f[2]], h[f[3]]])).or_default() += 1;
                    }
                }
                assert!((volume - 216.0).abs() < 1e-9, "{pattern:?}: volume {volume}");
                let mut outer: HashMap<(usize, i64), Vec<[Local; 4]>> = HashMap::new();
                for (q, n) in &faces {
                    match on_leaf_face(q) {
                        Some(f) => {
                            assert_eq!(*n, 1);
                            outer.entry(f).or_default().push(*q);
                        }
                        None => assert_eq!(*n, 2, "{pattern:?} {sym:?}: inner face {q:?} used {n} times"),
                    }
                }
                for axis in 0..3 {
                    for value in [0, 6] {
                        let mut got = outer.remove(&(axis, value)).unwrap_or_default();
                        let mut want = face_template(axis, value, face_corner_marks(axis, value, mask));
                        got.sort();
                        want.sort();
                        assert_eq!(got, want, "{pattern:?} {sym:?} face {axis}={value}");
                    }
                }
            }
        }
    }

    #[test]
    fn patterns_without_a_template_are_recognised() {
        assert_eq!(classify(0b1001), None, "diagonal of a face");
        assert_eq!(classify(0b1000_0001), None, "opposite corners");
        assert_eq!(classify(0b0111), None, "three of a face");
        assert_eq!(classify(0b0110_1001), None, "four not on a face");
        assert_eq!(classify(0b0111_1111), None);
        assert_eq!(classify(0b0011_0011), Some(Pattern::Face));
        assert_eq!(classify(0b0101_0000), Some(Pattern::Edge));
    }
}
