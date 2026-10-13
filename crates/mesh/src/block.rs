//! Mapped blocks: a structured mesh of one six-sided region given by its eight corners and,
//! optionally, curved edges (circular and elliptical arcs).
//!
//! The interior follows the edge-based transfinite interpolation of Gordon and Hall (1973,
//! "Construction of curvilinear co-ordinate systems and applications to mesh generation",
//! Int. J. Numer. Meth. Engng 7, 461–477): the twelve edge curves blended linearly across the
//! block, minus twice the trilinear interpolation of the corners. It reproduces every edge
//! exactly, gives Coons patches on the faces and the trilinear map for straight edges.
//!
//! Elements: hex8 and hex20 per cell; tet4 and tet10 by Kuhn's split of each cell into six
//! tetrahedra along its main diagonal (conforming between the cells of a block); wedge6 by
//! splitting each cell's first-second-direction face along its diagonal and extruding along the
//! third. Quadratic elements take their mid-side nodes from the same map, so they lie on the
//! curved edges.
//!
//! Every element is checked with the certified Jacobian test ([`crate::quality`]); a block
//! whose corners are not in hex8 order or whose curves fold it is an error.

use modelcraft_model::{ElementKind, MAX_COORD};

use crate::quality::{Validity, element_validity};
use crate::{Error, Result};

type P = [f64; 3];

/// Most elements one block makes.
pub const MAX_BLOCK_ELEMENTS: usize = 4_000_000;

/// Most divisions along one direction.
pub const MAX_DIVISIONS: usize = 100_000;

/// The corners of the block's twelve edges, grouped by direction: four along 1→2, four along
/// 1→4, four along 1→5, each from the corner where its direction's parameter is 0 to the one
/// where it is 1. Block edge numbers in [`BlockMesh::edges`] index this table.
pub const EDGES: [[usize; 2]; 12] = [[0, 1], [3, 2], [4, 5], [7, 6], [0, 3], [1, 2], [4, 7], [5, 6], [0, 4], [1, 5], [2, 6], [3, 7]];

/// Corner `c` of the block in parameter space (hex8 order).
const CORNERS: [[usize; 3]; 8] = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]];

/// The shape of one block edge between its two corners.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    Line,
    /// The shorter circular arc about `center` (both corners at the same distance from it).
    Arc {
        center: P,
    },
    /// The shorter arc of the ellipse `center + a cos t + b sin t` (`a` and `b` perpendicular)
    /// between the two corners, which must lie on it.
    Ellipse {
        center: P,
        axes: [P; 2],
    },
}

/// One block to mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: ElementKind,
    /// The corners in hex8 order: 1-2-3-4 counter-clockwise seen from 5-6-7-8's side.
    pub corners: [P; 8],
    /// Elements along the directions 1→2, 1→4 and 1→5.
    pub divisions: [usize; 3],
    /// Last element size over first along each direction (geometric grading); 1 is uniform.
    pub bias: [f64; 3],
    /// The curve of each edge, by corner pair (0-based corners).
    pub curves: Vec<([usize; 2], Curve)>,
}

/// The mesh of a block: points, elements as point indices in the model's node order, and where
/// element faces and edges lie on the block's faces and edges.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockMesh {
    pub points: Vec<P>,
    pub elements: Vec<Vec<u32>>,
    /// `(element, face 1-based, block face 0..6)`: block faces in hex8 face order.
    pub faces: Vec<(u32, u8, u8)>,
    /// `(element, edge 1-based, block edge 0..12)`: block edges as in [`EDGES`].
    pub edges: Vec<(u32, u8, u8)>,
    /// The point at each block corner.
    pub corners: [u32; 8],
}

impl Block {
    /// The block edge ([`EDGES`] index) joining two 0-based corners, either way round.
    pub fn edge_index(corners: [usize; 2]) -> Option<usize> {
        EDGES.iter().position(|e| *e == corners || *e == [corners[1], corners[0]])
    }
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add_scaled(a: P, s: f64, b: P) -> P {
    [a[0] + s * b[0], a[1] + s * b[1], a[2] + s * b[2]]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: P, b: P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}

/// An edge curve resolved against its end points: `point(s)` for `s` in `[0, 1]`.
#[derive(Debug, Clone, Copy)]
enum Path {
    Line { from: P, to: P },
    Ellipse { center: P, a: P, b: P, start: f64, sweep: f64 },
}

impl Path {
    fn new(curve: Curve, from: P, to: P) -> Result<Path> {
        let (center, a, b) = match curve {
            Curve::Line => return Ok(Path::Line { from, to }),
            Curve::Arc { center } => {
                let (p, q) = (sub(from, center), sub(to, center));
                let r = norm(p);
                if !(r > 0.0) || (norm(q) - r).abs() > 1e-9 * r.max(norm(q)) {
                    return Err(Error::Invalid(format!(
                        "an arc's corners {from:?} and {to:?} must be at the same distance from its centre {center:?}"
                    )));
                }
                let n = cross(p, q);
                if !(norm(n) > 1e-12 * r * r) {
                    return Err(Error::Invalid(format!(
                        "the arc from {from:?} to {to:?} about {center:?} has no plane (it spans 0 or 180 degrees); split it"
                    )));
                }
                let b = cross(n, p);
                (center, p, [b[0] * r / norm(b), b[1] * r / norm(b), b[2] * r / norm(b)])
            }
            Curve::Ellipse { center, axes: [a, b] } => (center, a, b),
        };
        let (la, lb) = (norm(a), norm(b));
        if !(la > 0.0 && lb > 0.0) || dot(a, b).abs() > 1e-9 * la * lb {
            return Err(Error::Invalid("an ellipse's two axes must be non-zero and perpendicular".into()));
        }
        let angle = |p: P| -> Result<f64> {
            let d = sub(p, center);
            let (c, s) = (dot(d, a) / (la * la), dot(d, b) / (lb * lb));
            let back = add_scaled(add_scaled(center, c, a), s, b);
            if norm(sub(back, p)) > 1e-9 * la.max(lb) || ((c * c + s * s).sqrt() - 1.0).abs() > 1e-9 {
                return Err(Error::Invalid(format!("corner {p:?} is not on the edge's ellipse or arc about {center:?}")));
            }
            Ok(s.atan2(c))
        };
        let start = angle(from)?;
        let mut sweep = angle(to)? - start;
        if sweep > std::f64::consts::PI {
            sweep -= 2.0 * std::f64::consts::PI;
        } else if sweep <= -std::f64::consts::PI {
            sweep += 2.0 * std::f64::consts::PI;
        }
        if !(sweep.abs() > 1e-12) || std::f64::consts::PI - sweep.abs() < 1e-9 {
            return Err(Error::Invalid(format!("the arc from {from:?} to {to:?} spans 0 or 180 degrees; split it")));
        }
        Ok(Path::Ellipse { center, a, b, start, sweep })
    }

    fn point(&self, s: f64) -> P {
        match *self {
            Path::Line { from, to } => add_scaled(from, s, sub(to, from)),
            Path::Ellipse { center, a, b, start, sweep } => {
                let t = start + s * sweep;
                add_scaled(add_scaled(center, t.cos(), a), t.sin(), b)
            }
        }
    }
}

/// The parameters of `n` elements along one direction with last/first size ratio `bias`, at
/// `order` points per element (`order · n + 1` values from 0 to 1; mid-side values halfway).
fn parameters(n: usize, bias: f64, order: usize) -> Vec<f64> {
    let q = if n > 1 { bias.powf(1.0 / (n - 1) as f64) } else { 1.0 };
    let mut ends = Vec::with_capacity(n + 1);
    let (mut sum, mut size) = (0.0, 1.0);
    ends.push(0.0);
    for _ in 0..n {
        sum += size;
        ends.push(sum);
        size *= q;
    }
    let ends: Vec<f64> = ends.iter().map(|e| e / sum).collect();
    let mut out = Vec::with_capacity(order * n + 1);
    for w in ends.windows(2) {
        let (a, b) = (w[0], w[1]);
        out.push(a);
        if order == 2 {
            out.push(0.5 * (a + b));
        }
    }
    out.push(1.0);
    out
}

/// The map from the unit cube onto the block.
struct Map {
    corners: [P; 8],
    edges: [Path; 12],
}

impl Map {
    fn point(&self, u: f64, v: f64, w: f64) -> P {
        let e = |k: usize, s: f64| self.edges[k].point(s);
        let terms: [(f64, P); 12] = [
            ((1.0 - v) * (1.0 - w), e(0, u)),
            (v * (1.0 - w), e(1, u)),
            ((1.0 - v) * w, e(2, u)),
            (v * w, e(3, u)),
            ((1.0 - u) * (1.0 - w), e(4, v)),
            (u * (1.0 - w), e(5, v)),
            ((1.0 - u) * w, e(6, v)),
            (u * w, e(7, v)),
            ((1.0 - u) * (1.0 - v), e(8, w)),
            (u * (1.0 - v), e(9, w)),
            (u * v, e(10, w)),
            ((1.0 - u) * v, e(11, w)),
        ];
        let mut x = [0.0; 3];
        for (f, p) in terms {
            x = add_scaled(x, f, p);
        }
        for (c, at) in self.corners.iter().zip(CORNERS) {
            let f = |bit: usize, t: f64| if bit == 1 { t } else { 1.0 - t };
            x = add_scaled(x, -2.0 * f(at[0], u) * f(at[1], v) * f(at[2], w), *c);
        }
        x
    }
}

/// Lattice offsets of each element of one cell, in units of the lattice (two per cell for
/// quadratic kinds), in the model's node order.
fn cell_elements(kind: ElementKind) -> Vec<Vec<[usize; 3]>> {
    let scale = |c: [usize; 3], m: usize| [c[0] * m, c[1] * m, c[2] * m];
    match kind {
        ElementKind::Hex8 => vec![CORNERS.to_vec()],
        ElementKind::Hex20 => {
            let mut nodes: Vec<[usize; 3]> = CORNERS.iter().map(|c| scale(*c, 2)).collect();
            for edge in kind.edges() {
                if let (Some(&a), Some(&b)) = (edge.first(), edge.get(1))
                    && let (Some(pa), Some(pb)) = (CORNERS.get(a), CORNERS.get(b))
                {
                    nodes.push([pa[0] + pb[0], pa[1] + pb[1], pa[2] + pb[2]]);
                }
            }
            vec![nodes]
        }
        ElementKind::Tet4 | ElementKind::Tet10 => {
            let m = if kind == ElementKind::Tet10 { 2 } else { 1 };
            [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]]
                .iter()
                .map(|perm| {
                    let mut v = [[0usize; 3]; 4];
                    for s in 0..3 {
                        v[s + 1] = v[s];
                        v[s + 1][perm[s]] = 1;
                    }
                    // Even permutations are positively oriented; swap two nodes of the others.
                    let odd = matches!(perm, [0, 2, 1] | [1, 0, 2] | [2, 1, 0]);
                    if odd {
                        v.swap(1, 2);
                    }
                    let mut nodes: Vec<[usize; 3]> = v.iter().map(|c| scale(*c, m)).collect();
                    if m == 2 {
                        for [a, b] in [[0, 1], [1, 2], [2, 0], [0, 3], [1, 3], [2, 3]] {
                            nodes.push([v[a][0] + v[b][0], v[a][1] + v[b][1], v[a][2] + v[b][2]]);
                        }
                    }
                    nodes
                })
                .collect()
        }
        ElementKind::Wedge6 => vec![
            vec![[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1]],
            vec![[0, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 1, 1], [0, 1, 1]],
        ],
        _ => Vec::new(),
    }
}

/// Mesh one block.
pub fn block(b: &Block) -> Result<BlockMesh> {
    let kind = b.kind;
    if !matches!(kind, ElementKind::Hex8 | ElementKind::Hex20 | ElementKind::Tet4 | ElementKind::Tet10 | ElementKind::Wedge6) {
        return Err(Error::Invalid(format!("a block is meshed with hex8, hex20, tet4, tet10 or wedge6 elements, not {}", kind.name())));
    }
    if b.corners.iter().flatten().any(|c| !c.is_finite() || c.abs() > MAX_COORD) {
        return Err(Error::Invalid(format!("block corners must be finite and within ±{MAX_COORD}")));
    }
    if b.divisions.iter().any(|&n| n == 0 || n > MAX_DIVISIONS) {
        return Err(Error::Invalid(format!("divisions must be 1 to {MAX_DIVISIONS} in each direction")));
    }
    if b.bias.iter().any(|&r| !(r.is_finite() && (1e-6..=1e6).contains(&r))) {
        return Err(Error::Invalid("bias must be between 1e-6 and 1e6 (1 is uniform)".into()));
    }
    let per_cell = cell_elements(kind).len();
    let cells = b.divisions.iter().try_fold(1usize, |acc, &n| acc.checked_mul(n));
    let count = cells.and_then(|c| c.checked_mul(per_cell)).filter(|&n| n <= MAX_BLOCK_ELEMENTS).ok_or_else(|| {
        Error::Limit(format!("a block makes at most {MAX_BLOCK_ELEMENTS} elements; {:?} divisions of {} are more", b.divisions, kind.name()))
    })?;

    let mut curves = [Curve::Line; 12];
    let mut seen = [false; 12];
    for (pair, curve) in &b.curves {
        let k = Block::edge_index(*pair)
            .ok_or_else(|| Error::Invalid(format!("corners {} and {} are not joined by a block edge", pair[0] + 1, pair[1] + 1)))?;
        if std::mem::replace(&mut seen[k], true) {
            return Err(Error::Invalid(format!("the edge from corner {} to {} is given twice", pair[0] + 1, pair[1] + 1)));
        }
        if let Some(slot) = curves.get_mut(k) {
            *slot = *curve;
        }
    }
    let mut paths = [Path::Line { from: [0.0; 3], to: [0.0; 3] }; 12];
    for (k, [from, to]) in EDGES.iter().enumerate() {
        paths[k] = Path::new(curves[k], b.corners[*from], b.corners[*to])?;
    }
    let map = Map { corners: b.corners, edges: paths };

    let order = if kind.is_quadratic() { 2 } else { 1 };
    let params: [Vec<f64>; 3] = [0, 1, 2].map(|d| parameters(b.divisions[d], b.bias[d], order));
    let size = [0, 1, 2].map(|d| params[d].len());
    let index = |c: [usize; 3]| c[0] + size[0] * (c[1] + size[1] * c[2]);

    // Elements as lattice coordinates; then number the lattice points they use, in lattice order.
    let pattern = cell_elements(kind);
    let mut lattice: Vec<Vec<[usize; 3]>> = Vec::with_capacity(count);
    for k in 0..b.divisions[2] {
        for j in 0..b.divisions[1] {
            for i in 0..b.divisions[0] {
                let base = [i * order, j * order, k * order];
                for e in &pattern {
                    lattice.push(e.iter().map(|o| [base[0] + o[0], base[1] + o[1], base[2] + o[2]]).collect());
                }
            }
        }
    }
    let total = size.iter().product::<usize>();
    let mut number: Vec<u32> = vec![u32::MAX; total];
    for e in &lattice {
        for c in e {
            if let Some(slot) = number.get_mut(index(*c)) {
                *slot = 0;
            }
        }
    }
    let mut points = Vec::new();
    for k in 0..size[2] {
        for j in 0..size[1] {
            for i in 0..size[0] {
                if let Some(slot) = number.get_mut(index([i, j, k]))
                    && *slot == 0
                {
                    *slot = u32::try_from(points.len()).map_err(|_| Error::Limit("too many points in one block".into()))?;
                    points.push(map.point(params[0][i], params[1][j], params[2][k]));
                }
            }
        }
    }
    let point_of = |c: [usize; 3]| number.get(index(c)).copied().filter(|&n| n != u32::MAX);
    let last = [0, 1, 2].map(|d| size[d] - 1);
    let mut corners = [0u32; 8];
    for (slot, c) in corners.iter_mut().zip(CORNERS) {
        *slot = point_of([c[0] * last[0], c[1] * last[1], c[2] * last[2]]).ok_or_else(|| Error::Failed("a block corner has no point".into()))?;
    }

    // Block faces in hex8 face order: (direction, at the end?).
    const FACES: [(usize, bool); 6] = [(2, false), (2, true), (1, false), (0, true), (1, true), (0, false)];
    let on_face = |c: &[usize; 3], f: (usize, bool)| c[f.0] == if f.1 { last[f.0] } else { 0 };
    let corner_count = kind.corner_count();
    let mut elements = Vec::with_capacity(lattice.len());
    let mut faces = Vec::new();
    let mut edges = Vec::new();
    for (e, nodes) in lattice.iter().enumerate() {
        let label = u32::try_from(e).map_err(|_| Error::Limit("too many elements in one block".into()))?;
        let ids: Vec<u32> =
            nodes.iter().map(|c| point_of(*c)).collect::<Option<_>>().ok_or_else(|| Error::Failed("an element node has no point".into()))?;
        let xyz: Vec<P> = ids.iter().map(|&i| points.get(i as usize).copied().unwrap_or([f64::NAN; 3])).collect();
        let check = element_validity(kind, &xyz)?;
        if check.validity != Validity::Valid {
            let cell = nodes.first().map(|c| [c[0] / order, c[1] / order, c[2] / order]).unwrap_or_default();
            return Err(Error::Invalid(format!(
                "the block's {} element in cell {cell:?} is inverted or degenerate: give the corners in hex8 order (1-2-3-4 counter-clockwise seen from 5-6-7-8) and curves that do not fold the block",
                kind.name()
            )));
        }
        for (f, local) in kind.faces().iter().enumerate() {
            let cs: Vec<&[usize; 3]> = local.iter().filter(|&&i| i < corner_count).filter_map(|&i| nodes.get(i)).collect();
            for (bf, side) in FACES.iter().enumerate() {
                if cs.iter().all(|c| on_face(c, *side)) {
                    faces.push((label, f as u8 + 1, bf as u8));
                }
            }
        }
        for (k, local) in kind.edges().iter().enumerate() {
            let (Some(a), Some(c)) = (local.first().and_then(|&i| nodes.get(i)), local.get(1).and_then(|&i| nodes.get(i))) else { continue };
            for (be, [from, to]) in EDGES.iter().enumerate() {
                // The block edge is where two of the three lattice coordinates are fixed.
                let along = (0..3).find(|&d| CORNERS[*from][d] != CORNERS[*to][d]).unwrap_or(0);
                let fixed = |p: &[usize; 3]| (0..3).filter(|&d| d != along).all(|d| p[d] == CORNERS[*from][d] * last[d]);
                if fixed(a) && fixed(c) {
                    edges.push((label, k as u8 + 1, be as u8));
                }
            }
        }
        elements.push(ids);
    }
    Ok(BlockMesh { points, elements, faces, edges, corners })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(kind: ElementKind, divisions: [usize; 3]) -> Block {
        let corners = CORNERS.map(|c| [c[0] as f64, c[1] as f64, c[2] as f64]);
        Block { kind, corners, divisions, bias: [1.0; 3], curves: Vec::new() }
    }

    fn volume(kind: ElementKind, m: &BlockMesh) -> f64 {
        m.elements
            .iter()
            .map(|e| {
                let xyz: Vec<P> = e.iter().map(|&i| m.points[i as usize]).collect();
                element_validity(kind, &xyz).unwrap().volume
            })
            .sum()
    }

    #[test]
    fn every_kind_fills_a_cube_with_valid_elements_and_shared_nodes() {
        for (kind, per_cell, points) in [
            (ElementKind::Hex8, 1, 27),
            (ElementKind::Hex20, 1, 81),
            (ElementKind::Tet4, 6, 27),
            (ElementKind::Tet10, 6, 125),
            (ElementKind::Wedge6, 2, 27),
        ] {
            let m = block(&unit(kind, [2, 2, 2])).unwrap();
            assert_eq!(m.elements.len(), 8 * per_cell, "{kind:?}");
            assert_eq!(m.points.len(), points, "{kind:?}");
            assert!((volume(kind, &m) - 1.0).abs() < 1e-12, "{kind:?}");
            // Each block face is covered by element faces of total area 1: count them instead.
            let per_face = |bf: u8| m.faces.iter().filter(|f| f.2 == bf).count();
            let expect = if matches!(kind, ElementKind::Hex8 | ElementKind::Hex20) { 4 } else { 8 };
            for bf in 0..6 {
                let n = per_face(bf);
                // Wedges have quads on the sides and triangles top and bottom.
                let want = if kind == ElementKind::Wedge6 && bf >= 2 { 4 } else { expect };
                assert_eq!(n, want, "{kind:?} face {bf}");
            }
            for be in 0..12 {
                // Two segments per block edge (an edge segment may belong to several elements).
                let mut segments: Vec<[u32; 2]> = m
                    .edges
                    .iter()
                    .filter(|e| e.2 == be)
                    .map(|&(e, k, _)| {
                        let local = kind.edges()[k as usize - 1];
                        let el = &m.elements[e as usize];
                        let mut s = [el[local[0]], el[local[1]]];
                        s.sort();
                        s
                    })
                    .collect();
                segments.sort();
                segments.dedup();
                assert_eq!(segments.len(), 2, "{kind:?} edge {be}");
            }
            assert_eq!(m.points[m.corners[6] as usize], [1.0, 1.0, 1.0]);
        }
    }

    #[test]
    fn arcs_and_ellipses_put_every_edge_node_on_the_curve() {
        // A quarter annulus a = 1, b = 2, extruded 0.5, hex20 so mid-side nodes are on the arcs.
        let (a, b) = (1.0, 2.0);
        let corners = [[a, 0.0, 0.0], [b, 0.0, 0.0], [0.0, b, 0.0], [0.0, a, 0.0], [a, 0.0, 0.5], [b, 0.0, 0.5], [0.0, b, 0.5], [0.0, a, 0.5]];
        let o = |z: f64| [0.0, 0.0, z];
        let curves = vec![
            ([1, 2], Curve::Arc { center: o(0.0) }),
            ([0, 3], Curve::Arc { center: o(0.0) }),
            ([5, 6], Curve::Arc { center: o(0.5) }),
            ([4, 7], Curve::Ellipse { center: o(0.5), axes: [[a, 0.0, 0.0], [0.0, a, 0.0]] }),
        ];
        let m = block(&Block { kind: ElementKind::Hex20, corners, divisions: [3, 6, 2], bias: [1.0; 3], curves }).unwrap();
        for p in &m.points {
            let r = p[0].hypot(p[1]);
            assert!(r > a - 1e-12 && r < b + 1e-12, "{p:?}");
        }
        let exact = std::f64::consts::PI / 4.0 * (b * b - a * a) * 0.5;
        let v = volume(ElementKind::Hex20, &m);
        assert!((v - exact).abs() < 1e-4 * exact, "{v} vs {exact}");
        // The inner face's nodes are on r = a.
        let elements = &m.elements;
        let inner: Vec<u32> = m
            .faces
            .iter()
            .filter(|f| f.2 == 5)
            .flat_map(|&(e, f, _)| ElementKind::Hex20.faces()[f as usize - 1].iter().map(move |&i| elements[e as usize][i]))
            .collect();
        assert!(!inner.is_empty());
        assert!(inner.iter().all(|&i| (m.points[i as usize][0].hypot(m.points[i as usize][1]) - a).abs() < 1e-12));
    }

    #[test]
    fn grading_and_bad_blocks() {
        let mut g = unit(ElementKind::Hex8, [4, 1, 1]);
        g.bias = [8.0, 1.0, 1.0];
        let m = block(&g).unwrap();
        let xs: Vec<f64> = (0..5).map(|i| m.points[i][0]).collect();
        let (first, last) = (xs[1] - xs[0], xs[4] - xs[3]);
        assert!((last / first - 8.0).abs() < 1e-9, "{xs:?}");

        let mut inverted = unit(ElementKind::Hex8, [1, 1, 1]);
        inverted.corners.swap(1, 3);
        assert!(block(&inverted).is_err());
        let mut off = unit(ElementKind::Hex8, [1, 1, 1]);
        off.curves = vec![([0, 1], Curve::Arc { center: [0.3, 0.0, 0.0] })];
        assert!(block(&off).is_err(), "corners at different distances from the centre");
        let mut half = unit(ElementKind::Hex8, [1, 1, 1]);
        half.curves = vec![([0, 1], Curve::Arc { center: [0.5, 0.0, 0.0] })];
        assert!(block(&half).is_err(), "180 degrees has no plane");
        let mut diagonal = unit(ElementKind::Hex8, [1, 1, 1]);
        diagonal.curves = vec![([0, 6], Curve::Line)];
        assert!(block(&diagonal).is_err(), "not an edge");
        for divisions in [[0, 1, 1], [MAX_DIVISIONS + 1, 1, 1], [MAX_DIVISIONS, MAX_DIVISIONS, MAX_DIVISIONS]] {
            assert!(block(&unit(ElementKind::Hex8, divisions)).is_err(), "{divisions:?}");
        }
        assert!(block(&unit(ElementKind::Quad4, [1, 1, 1])).is_err());
        let mut nan = unit(ElementKind::Hex8, [1, 1, 1]);
        nan.corners[0][0] = f64::NAN;
        assert!(block(&nan).is_err());
        let mut bias = unit(ElementKind::Hex8, [2, 1, 1]);
        bias.bias[0] = 0.0;
        assert!(block(&bias).is_err());
    }
}
