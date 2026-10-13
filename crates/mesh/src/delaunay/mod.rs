//! Exact, deterministic 3D Delaunay tetrahedralization of a point set.
//!
//! # Method
//! - **Incremental cavity insertion** (A. Bowyer, *Computing Dirichlet tessellations*, Comput. J.
//!   24(2):162–166, 1981; D. F. Watson, *Computing the n-dimensional Delaunay tessellation with
//!   application to Voronoi polytopes*, Comput. J. 24(2):167–172, 1981). Each point finds the
//!   tetrahedra whose circumsphere contains it (the cavity, grown by adjacency from one of them),
//!   deletes them and joins the cavity's boundary faces to the point.
//! - **Insertion order**: BRIO rounds with Hilbert order inside each round (Amenta, Choi, Rote,
//!   SoCG 2003; Skilling 2004). See [`order`].
//! - **Point location**: the stochastic visibility walk with memory (O. Devillers, S. Pion,
//!   M. Teillaud, *Walking in a triangulation*, IJFCS 13(2):181–199, 2002, §2–§3), starting at
//!   the last tetrahedron created. In a Delaunay triangulation the walk cannot cycle
//!   (H. Edelsbrunner, *An acyclicity theorem for cell complexes in d dimensions*, Combinatorica
//!   10(3):251–260, 1990); it is still capped at the number of tetrahedra, after which a linear
//!   scan locates the point.
//! - **The infinite vertex.** The convex hull is closed by *ghost* tetrahedra: each hull face is
//!   joined to the vertex [`GHOST`], so every face has exactly two tetrahedra and the adjacency is
//!   that of a triangulated 3-sphere (no boundary cases). A ghost tetrahedron's "circumsphere" is
//!   the open half-space beyond its hull face; a point on the plane of the face is decided by the
//!   finite tetrahedron on the other side of that face (the limit of spheres through the face
//!   whose centre runs off to infinity), so the decision is the same perturbed one
//!   (O. Devillers, M. Teillaud, *Perturbations for Delaunay and weighted Delaunay 3D
//!   triangulations*, CGTA 44(3):160–168, 2011, §3 and the infinite-vertex discussion).
//! - **Compact adjacency arrays**: per tetrahedron four vertex ids and four neighbour ids, where
//!   neighbour `i` is across the face opposite vertex `i`. Deleted slots are recycled LIFO.
//!
//! # Exactness and degeneracies
//! Every decision is an exact predicate from [`crate::predicates`]: `orient3d` for the walk and
//! for the hull, `insphere_sos` (Devillers–Teillaud symbolic perturbation) for the cavity. There
//! are no tolerances. The perturbation is a function of the coordinates only (it lifts each point
//! by an infinitesimal that depends on its lexicographic rank), so the result is *the* Delaunay
//! triangulation of the perturbed points: unique, independent of the insertion order, and with
//! no flat tetrahedra, even on grids where every eight neighbours are cospherical.
//!
//! - **Duplicates are merged.** Points with equal coordinates (`-0.0 == 0.0`) become one vertex:
//!   the one with the lowest input index. The others are reported by
//!   [`Triangulation::duplicate_of`] and appear in no tetrahedron.
//! - **Lower-dimensional inputs** (all points collinear or coplanar, a single point, none) have
//!   no tetrahedra: [`Triangulation::dimension`] reports their affine dimension and the
//!   tetrahedron lists are empty. Points arriving later that leave the plane start the 3D
//!   triangulation.
//! - **Hostile coordinates** are rejected at the boundary by [`Point3::new`] (NaN, infinities,
//!   magnitudes outside `[2⁻¹²⁰, 2¹⁶⁰]`); at most [`MAX_POINTS`] points.
//!
//! # Determinism
//! Same points in the same order → same tetrahedra in the same slots, bit for bit: the order is
//! a fixed function of the input, the walk's "random" face choice comes from a fixed-seed
//! generator, sorting has total keys, and nothing iterates a hash map.
//!
//! # Verification
//! [`Triangulation::verify`] checks the adjacency invariants, the orientation of every
//! tetrahedron, the convexity of the hull and the empty-circumsphere property of every interior
//! face (exact and perturbed), which by Delaunay's lemma makes every circumsphere empty. Tests
//! also use [`Triangulation::verify_empty_spheres`], the brute-force check of every point against
//! every circumsphere.

mod order;
mod verify;

#[cfg(test)]
mod tests;

use crate::predicates::{Point2, Point3, Sign, insphere_sos, orient2d, orient3d};
use crate::{Error, Result};

/// The infinite vertex of the ghost tetrahedra that close the convex hull.
pub const GHOST: u32 = u32::MAX;
/// Marks a deleted tetrahedron slot (first vertex).
const DEAD: u32 = u32::MAX - 1;
/// No tetrahedron / no vertex.
const NONE: u32 = u32::MAX;
/// An empty slot of the edge table (no edge has two equal endpoints).
const EMPTY_EDGE: u64 = u64::MAX;
/// The most points a triangulation takes (vertex ids stay far below [`GHOST`] and the ~6.7
/// tetrahedra per point fit `u32` slot ids).
pub const MAX_POINTS: usize = 1 << 24;

/// Where a point lies relative to the triangulation ([`Triangulation::locate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// In the closed finite tetrahedron `t` and not one of its vertices.
    Inside(u32),
    /// Strictly beyond the hull face of the ghost tetrahedron `t`.
    Outside(u32),
    /// Equal to this vertex.
    Vertex(u32),
}

/// The outcome of [`Triangulation::insert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insertion {
    /// A new vertex.
    Inserted(u32),
    /// The point equals this existing vertex; it was recorded as a duplicate (its own id is the
    /// last point index).
    Duplicate(u32),
}

/// A Delaunay tetrahedralization with ghost tetrahedra closing the hull.
///
/// Vertex ids are input point indices. Tetrahedron ids are slots: [`Triangulation::slots`]
/// counts them, and [`Triangulation::tet`] returns `None` for a recycled slot. A tetrahedron's
/// vertices `[a, b, c, d]` have positive volume, `det[b − a, c − a, d − a] > 0` (the tet4 node
/// order). A ghost tetrahedron has [`GHOST`] in one slot; putting a point in place of the ghost
/// gives positive volume exactly when that point is beyond the hull face.
#[derive(Debug, Clone)]
pub struct Triangulation {
    points: Vec<Point3>,
    /// `NONE`, or the vertex this point duplicates.
    duplicate_of: Vec<u32>,
    tets: Vec<[u32; 4]>,
    /// Neighbour `i` is across the face opposite vertex `i`.
    adj: Vec<[u32; 4]>,
    free: Vec<u32>,
    /// A live tetrahedron incident to each vertex (`NONE` until it is in one).
    vertex_tet: Vec<u32>,
    /// While the points span less than 3D: affinely independent vertices, and the others.
    basis: Vec<u32>,
    pending: Vec<u32>,
    /// Cavity marks: `epoch` = in the cavity, `epoch + 1` = tested and outside.
    mark: Vec<u32>,
    epoch: u32,
    /// Where the next walk starts.
    hint: u32,
    /// State of the walk's face-order generator.
    rng: u64,
    scratch: Scratch,
}

#[derive(Debug, Clone, Default)]
struct Scratch {
    cavity: Vec<u32>,
    stack: Vec<u32>,
    /// `(cavity tet, face slot, outside neighbour)`.
    boundary: Vec<(u32, usize, u32)>,
    /// `(new tet vertices, slot of the outside neighbour, neighbour, the neighbour's slot back)`.
    new_tets: Vec<([u32; 4], usize, u32, usize)>,
    /// Open-addressing table `(edge key, tet, slot)` linking the new tetrahedra to each other.
    edges: Vec<(u64, u32, usize)>,
}

/// A Delaunay triangulation is corrupt: this never happens with exact predicates, and is
/// reported instead of panicking.
fn corrupt(what: &str) -> Error {
    Error::Failed(format!("Delaunay triangulation is inconsistent: {what}"))
}

/// The sign of the volume `det[b − a, c − a, d − a]` (Shewchuk's `orient3d` has the opposite
/// sign).
#[inline]
fn volume_sign(p: [&Point3; 4]) -> Sign {
    let [a, b, c, d] = p;
    -orient3d(a, b, c, d)
}

fn collinear(a: &Point3, b: &Point3, c: &Point3) -> bool {
    let project = |p: &Point3, i: usize, j: usize| {
        let q = p.coords();
        // Projections of admissible points are admissible.
        Point2::new(q.get(i).copied().unwrap_or(0.0), q.get(j).copied().unwrap_or(0.0))
    };
    [(0, 1), (1, 2), (2, 0)].iter().all(|&(i, j)| match (project(a, i, j), project(b, i, j), project(c, i, j)) {
        (Ok(a), Ok(b), Ok(c)) => orient2d(&a, &b, &c).is_zero(),
        _ => false,
    })
}

/// The point coordinates as [`Point3`]s, with `-0.0` made `+0.0` so equal points have equal bits.
fn admit(coords: &[[f64; 3]]) -> Result<Vec<Point3>> {
    coords
        .iter()
        .enumerate()
        .map(|(i, &[x, y, z])| Point3::new(x + 0.0, y + 0.0, z + 0.0).map_err(|e| Error::Invalid(format!("point {i}: {e}"))))
        .collect()
}

impl Triangulation {
    fn empty() -> Self {
        Triangulation {
            points: Vec::new(),
            duplicate_of: Vec::new(),
            tets: Vec::new(),
            adj: Vec::new(),
            free: Vec::new(),
            vertex_tet: Vec::new(),
            basis: Vec::new(),
            pending: Vec::new(),
            mark: Vec::new(),
            epoch: 0,
            hint: NONE,
            rng: 0x2545_F491_4F6C_DD1D,
            scratch: Scratch::default(),
        }
    }

    /// The Delaunay tetrahedralization of `points`. Duplicates are merged into the lowest
    /// index; fewer than four affinely independent points give no tetrahedra.
    pub fn new(points: &[[f64; 3]]) -> Result<Self> {
        if points.len() > MAX_POINTS {
            return Err(Error::Limit(format!("{} points; a triangulation takes at most {MAX_POINTS}", points.len())));
        }
        let mut tri = Self::empty();
        tri.points = admit(points)?;
        let n = tri.points.len();
        tri.duplicate_of = vec![NONE; n];
        tri.vertex_tet = vec![NONE; n];

        // Merge duplicates: sort by coordinates, ties by index, keep the first of each run.
        let mut by_coords: Vec<u32> = (0..n as u32).collect();
        let key = |v: u32| tri.points.get(v as usize).map(|p| p.coords().map(f64::to_bits)).unwrap_or_default();
        // Equal values have equal bits after `admit`; the bit order is total and deterministic.
        by_coords.sort_unstable_by_key(|&v| (key(v), v));
        let mut first = NONE;
        for w in 0..by_coords.len() {
            let v = by_coords.get(w).copied().unwrap_or(NONE);
            if w > 0 && first != NONE && key(v) == key(first) {
                if let Some(d) = tri.duplicate_of.get_mut(v as usize) {
                    *d = first;
                }
            } else {
                first = v;
            }
        }
        let unique: Vec<u32> = (0..n as u32).filter(|&v| tri.duplicate_of.get(v as usize) == Some(&NONE)).collect();

        let expected = unique.len().saturating_mul(7);
        tri.tets.reserve(expected);
        tri.adj.reserve(expected);
        tri.mark.reserve(expected);
        for v in order::brio_hilbert(&tri.points, &unique) {
            tri.add(v)?;
        }
        Ok(tri)
    }

    /// Insert one more point (a Steiner point, say). Its vertex id is its index in
    /// [`Triangulation::points`]. A point equal to an existing vertex is recorded as a duplicate.
    pub fn insert(&mut self, p: [f64; 3]) -> Result<Insertion> {
        if self.points.len() >= MAX_POINTS {
            return Err(Error::Limit(format!("a triangulation takes at most {MAX_POINTS} points")));
        }
        let point = admit(&[p])?.into_iter().next().ok_or_else(|| corrupt("point"))?;
        if self.tets.is_empty() {
            // Below 3D: compare with the (few or degenerate) vertices directly.
            if let Some(&u) = self.basis.iter().chain(&self.pending).find(|&&u| self.points.get(u as usize) == Some(&point)) {
                self.push_point(point, u);
                return Ok(Insertion::Duplicate(u));
            }
        }
        let v = self.push_point(point, NONE);
        self.add(v)
    }

    fn push_point(&mut self, p: Point3, duplicate_of: u32) -> u32 {
        let v = self.points.len() as u32;
        self.points.push(p);
        self.duplicate_of.push(duplicate_of);
        self.vertex_tet.push(NONE);
        v
    }

    /// The points, by vertex id (including duplicates).
    pub fn points(&self) -> &[Point3] {
        &self.points
    }

    /// The vertex a duplicate point was merged into.
    pub fn duplicate_of(&self, v: u32) -> Option<u32> {
        self.duplicate_of.get(v as usize).copied().filter(|&d| d != NONE)
    }

    /// The affine dimension of the distinct points: 3 when there are tetrahedra, otherwise 0
    /// (none or one point), 1 (collinear) or 2 (coplanar).
    pub fn dimension(&self) -> usize {
        if self.tets.is_empty() { self.basis.len().saturating_sub(1) } else { 3 }
    }

    /// Tetrahedron slots, live or recycled, finite or ghost.
    pub fn slots(&self) -> usize {
        self.tets.len()
    }

    /// The vertices of the tetrahedron in slot `t` (one may be [`GHOST`]); `None` for a recycled
    /// or out-of-range slot.
    pub fn tet(&self, t: u32) -> Option<[u32; 4]> {
        self.tets.get(t as usize).copied().filter(|v| v[0] != DEAD)
    }

    /// The neighbours of tetrahedron `t`: neighbour `i` shares the face opposite vertex `i`.
    pub fn neighbors(&self, t: u32) -> Option<[u32; 4]> {
        self.tet(t)?;
        self.adj.get(t as usize).copied()
    }

    /// Whether `t` is a ghost tetrahedron (one vertex is [`GHOST`]).
    pub fn is_ghost(&self, t: u32) -> bool {
        self.tet(t).is_some_and(|v| v.contains(&GHOST))
    }

    /// A live tetrahedron incident to vertex `v` (finite or ghost), once `v` is triangulated.
    pub fn incident_tet(&self, v: u32) -> Option<u32> {
        self.vertex_tet.get(v as usize).copied().filter(|&t| t != NONE)
    }

    /// The finite tetrahedra with their slot ids, in slot order; vertices in tet4 order.
    pub fn finite_tets(&self) -> impl Iterator<Item = (u32, [u32; 4])> + '_ {
        self.tets.iter().enumerate().filter(|(_, v)| v[0] != DEAD && !v.contains(&GHOST)).map(|(t, v)| (t as u32, *v))
    }

    /// The number of finite tetrahedra.
    pub fn finite_tet_count(&self) -> usize {
        self.finite_tets().count()
    }

    /// Locate `p`: the walk from the last tetrahedron created.
    pub fn locate(&self, p: [f64; 3]) -> Result<Option<Location>> {
        if self.tets.is_empty() {
            return Ok(None);
        }
        let point = admit(&[p])?.into_iter().next().ok_or_else(|| corrupt("point"))?;
        let mut rng = self.rng;
        self.walk(&point, self.hint, &mut rng).map(Some)
    }

    // ---- internals ----

    fn pt(&self, v: u32) -> Result<&Point3> {
        self.points.get(v as usize).ok_or_else(|| corrupt("vertex id"))
    }

    fn tet_at(&self, t: u32) -> Result<[u32; 4]> {
        self.tet(t).ok_or_else(|| corrupt("tetrahedron id"))
    }

    fn adj_at(&self, t: u32) -> Result<[u32; 4]> {
        self.adj.get(t as usize).copied().ok_or_else(|| corrupt("tetrahedron id"))
    }

    /// The four points of a finite tetrahedron.
    fn pts(&self, [a, b, c, d]: [u32; 4]) -> Result<[&Point3; 4]> {
        Ok([self.pt(a)?, self.pt(b)?, self.pt(c)?, self.pt(d)?])
    }

    /// The four points of `vs` with slot `k` replaced by `p` (`k` may hold the ghost).
    fn with_point<'a>(&'a self, vs: [u32; 4], k: usize, p: &'a Point3) -> Result<[&'a Point3; 4]> {
        let mut out = [p; 4];
        for (i, (o, &v)) in out.iter_mut().zip(&vs).enumerate() {
            if i != k {
                *o = self.pt(v)?;
            }
        }
        Ok(out)
    }

    /// Add vertex `v` (already in `points`, not a known duplicate).
    fn add(&mut self, v: u32) -> Result<Insertion> {
        if !self.tets.is_empty() {
            return self.insert_vertex(v);
        }
        let p = *self.pt(v)?;
        match self.basis.as_slice() {
            [] | [_] => self.basis.push(v),
            &[a, b] => {
                if collinear(self.pt(a)?, self.pt(b)?, &p) {
                    self.pending.push(v);
                } else {
                    self.basis.push(v);
                }
            }
            &[a, b, c, ..] => {
                if orient3d(self.pt(a)?, self.pt(b)?, self.pt(c)?, &p).is_zero() {
                    self.pending.push(v);
                } else {
                    self.start([a, b, c, v])?;
                    for u in std::mem::take(&mut self.pending) {
                        self.insert_vertex(u)?;
                    }
                }
            }
        }
        Ok(Insertion::Inserted(v))
    }

    /// The first tetrahedron and its four ghosts.
    fn start(&mut self, [a, b, c, d]: [u32; 4]) -> Result<()> {
        let t0 = if volume_sign(self.pts([a, b, c, d])?) == Sign::Positive { [a, b, c, d] } else { [b, a, c, d] };
        let mut all = vec![t0];
        for j in 0..4 {
            let mut g = t0;
            g[j] = GHOST;
            // Swapping two other slots turns the ghost to face away from the hull.
            g.swap((j + 1) % 4, (j + 2) % 4);
            all.push(g);
        }
        let mut faces: Vec<([u32; 3], u32, usize)> = Vec::with_capacity(20);
        for vs in all {
            let t = self.alloc(vs)?;
            for k in 0..4 {
                let mut f = [0; 3];
                for (o, &v) in f.iter_mut().zip(vs.iter().enumerate().filter(|&(i, _)| i != k).map(|(_, v)| v)) {
                    *o = v;
                }
                f.sort_unstable();
                faces.push((f, t, k));
            }
            self.hint = t;
        }
        faces.sort_unstable();
        for pair in faces.chunks(2) {
            match pair {
                [(f, t, i), (g, u, j)] if f == g => {
                    self.set_adj(*t, *i, *u)?;
                    self.set_adj(*u, *j, *t)?;
                }
                _ => return Err(corrupt("initial tetrahedron faces")),
            }
        }
        self.basis.clear();
        Ok(())
    }

    fn set_adj(&mut self, t: u32, i: usize, n: u32) -> Result<()> {
        let slot = self.adj.get_mut(t as usize).and_then(|a| a.get_mut(i)).ok_or_else(|| corrupt("tetrahedron id"))?;
        *slot = n;
        Ok(())
    }

    fn alloc(&mut self, vs: [u32; 4]) -> Result<u32> {
        let t = match self.free.pop() {
            Some(t) => {
                *self.tets.get_mut(t as usize).ok_or_else(|| corrupt("free slot"))? = vs;
                *self.adj.get_mut(t as usize).ok_or_else(|| corrupt("free slot"))? = [NONE; 4];
                t
            }
            None => {
                if self.tets.len() >= (DEAD - 1) as usize {
                    return Err(Error::Limit("too many tetrahedra".into()));
                }
                self.tets.push(vs);
                self.adj.push([NONE; 4]);
                self.mark.push(0);
                (self.tets.len() - 1) as u32
            }
        };
        for v in vs {
            if let Some(s) = self.vertex_tet.get_mut(v as usize) {
                *s = t;
            }
        }
        Ok(t)
    }

    fn kill(&mut self, t: u32) -> Result<()> {
        *self.tets.get_mut(t as usize).ok_or_else(|| corrupt("tetrahedron id"))? = [DEAD; 4];
        self.free.push(t);
        Ok(())
    }

    /// Whether `p`'s (perturbed) circumsphere test puts it in conflict with tetrahedron `t`.
    fn conflict(&self, t: u32, p: &Point3) -> Result<bool> {
        let vs = self.tet_at(t)?;
        let Some(g) = vs.iter().position(|&v| v == GHOST) else {
            let [a, b, c, d] = self.pts(vs)?;
            // `insphere` wants Shewchuk-positive orientation: swap the first two vertices.
            return Ok(insphere_sos(b, a, c, d, p) == Sign::Positive);
        };
        Ok(match volume_sign(self.with_point(vs, g, p)?) {
            Sign::Positive => true,
            Sign::Negative => false,
            // On the hull face's plane: the finite tetrahedron across that face decides.
            Sign::Zero => {
                let o = self.adj_at(t)?.get(g).copied().ok_or_else(|| corrupt("slot"))?;
                let ws = self.tet_at(o)?;
                if ws.contains(&GHOST) {
                    return Err(corrupt("two ghost tetrahedra share a hull face"));
                }
                let [a, b, c, d] = self.pts(ws)?;
                insphere_sos(b, a, c, d, p) == Sign::Positive
            }
        })
    }

    /// The visibility walk towards `p` from `start` (Devillers, Pion, Teillaud 2002).
    fn walk(&self, p: &Point3, start: u32, rng: &mut u64) -> Result<Location> {
        let mut t = start;
        let vs = self.tet_at(t)?;
        if let Some(g) = vs.iter().position(|&v| v == GHOST) {
            t = self.adj_at(t)?.get(g).copied().ok_or_else(|| corrupt("slot"))?;
        }
        let mut prev = NONE;
        for _ in 0..self.tets.len().saturating_add(16) {
            let vs = self.tet_at(t)?;
            if vs.contains(&GHOST) {
                return Ok(Location::Outside(t));
            }
            let adj = self.adj_at(t)?;
            // xorshift64 (Marsaglia 2003): which face to try first.
            *rng ^= *rng << 13;
            *rng ^= *rng >> 7;
            *rng ^= *rng << 17;
            let first = (*rng >> 61) as usize;
            let mut next = NONE;
            for i in 0..4 {
                let k = (first + i) % 4;
                let n = adj.get(k).copied().unwrap_or(NONE);
                if n == prev {
                    continue;
                }
                if volume_sign(self.with_point(vs, k, p)?) == Sign::Negative {
                    next = n;
                    break;
                }
            }
            if next == NONE {
                for v in vs {
                    if self.pt(v)? == p {
                        return Ok(Location::Vertex(v));
                    }
                }
                return Ok(Location::Inside(t));
            }
            prev = t;
            t = next;
        }
        self.locate_by_scan(p)
    }

    /// Point location by testing every tetrahedron (the walk's fallback).
    fn locate_by_scan(&self, p: &Point3) -> Result<Location> {
        let mut outside = None;
        for (t, &vs) in self.tets.iter().enumerate() {
            if vs[0] == DEAD {
                continue;
            }
            match vs.iter().position(|&v| v == GHOST) {
                Some(g) => {
                    if outside.is_none() && volume_sign(self.with_point(vs, g, p)?) == Sign::Positive {
                        outside = Some(t as u32);
                    }
                }
                None => {
                    let mut inside = true;
                    for k in 0..4 {
                        inside &= volume_sign(self.with_point(vs, k, p)?) != Sign::Negative;
                    }
                    if inside {
                        for v in vs {
                            if self.pt(v)? == p {
                                return Ok(Location::Vertex(v));
                            }
                        }
                        return Ok(Location::Inside(t as u32));
                    }
                }
            }
        }
        outside.map(Location::Outside).ok_or_else(|| corrupt("a point is neither inside nor outside the hull"))
    }

    fn next_epoch(&mut self) {
        if self.epoch >= u32::MAX - 3 {
            self.mark.fill(0);
            self.epoch = 0;
        }
        self.epoch += 2;
    }

    fn mark_of(&self, t: u32) -> u32 {
        self.mark.get(t as usize).copied().unwrap_or(0)
    }

    fn set_mark(&mut self, t: u32, m: u32) {
        if let Some(s) = self.mark.get_mut(t as usize) {
            *s = m;
        }
    }

    /// Insert vertex `v` into the 3D triangulation.
    fn insert_vertex(&mut self, v: u32) -> Result<Insertion> {
        let p = *self.pt(v)?;
        let mut rng = self.rng;
        let located = self.walk(&p, self.hint, &mut rng)?;
        self.rng = rng;
        let seed = match located {
            Location::Vertex(u) => {
                if let Some(d) = self.duplicate_of.get_mut(v as usize) {
                    *d = u;
                }
                return Ok(Insertion::Duplicate(u));
            }
            Location::Inside(t) | Location::Outside(t) => t,
        };
        let mut s = std::mem::take(&mut self.scratch);
        let result = self.dig(seed, &p, &mut s).and_then(|()| self.fill(v, &mut s));
        self.scratch = s;
        result.map(|()| Insertion::Inserted(v))
    }

    /// The cavity of `p`: every tetrahedron in conflict, grown by adjacency from `seed`, and the
    /// faces between it and the rest.
    fn dig(&mut self, seed: u32, p: &Point3, s: &mut Scratch) -> Result<()> {
        self.next_epoch();
        let (inside, outside) = (self.epoch, self.epoch + 1);
        s.cavity.clear();
        s.stack.clear();
        s.boundary.clear();
        self.set_mark(seed, inside);
        s.cavity.push(seed);
        s.stack.push(seed);
        while let Some(t) = s.stack.pop() {
            for (j, o) in self.adj_at(t)?.into_iter().enumerate() {
                let m = self.mark_of(o);
                if m == inside {
                    continue;
                }
                if m != outside && self.conflict(o, p)? {
                    self.set_mark(o, inside);
                    s.cavity.push(o);
                    s.stack.push(o);
                } else {
                    self.set_mark(o, outside);
                    s.boundary.push((t, j, o));
                }
            }
        }
        Ok(())
    }

    /// Replace the cavity by the tetrahedra joining its boundary faces to `v`.
    fn fill(&mut self, v: u32, s: &mut Scratch) -> Result<()> {
        s.new_tets.clear();
        // Everything is read before any slot is recycled: a new tetrahedron may reuse the id of
        // a cavity tetrahedron that another outside neighbour still points to.
        for &(t, j, o) in &s.boundary {
            let mut vs = self.tet_at(t)?;
            *vs.get_mut(j).ok_or_else(|| corrupt("slot"))? = v;
            let back = self.adj_at(o)?.iter().position(|&n| n == t).ok_or_else(|| corrupt("one-sided adjacency"))?;
            s.new_tets.push((vs, j, o, back));
        }
        for &t in &s.cavity {
            self.kill(t)?;
        }
        // Each edge of the cavity boundary is shared by exactly two boundary faces; their new
        // tetrahedra meet across the face spanned by that edge and `v`. Pair them through a
        // small open-addressing table keyed by the edge (lookups only, so no order leaks out).
        let size = (s.new_tets.len() * 4).next_power_of_two().max(16);
        s.edges.clear();
        s.edges.resize(size, (EMPTY_EDGE, NONE, 0));
        let mask = size - 1;
        let mut open = 0usize;
        for &(vs, j, o, back) in &s.new_tets {
            let t = self.alloc(vs)?;
            self.set_adj(t, j, o)?;
            self.set_adj(o, back, t)?;
            for k in (0..4).filter(|&k| k != j) {
                let mut e = vs.iter().enumerate().filter(|&(i, _)| i != j && i != k).map(|(_, &u)| u);
                let (a, b) = (e.next().unwrap_or(NONE), e.next().unwrap_or(NONE));
                let key = (u64::from(a.min(b)) << 32) | u64::from(a.max(b));
                let mut h = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize & mask;
                let mut placed = false;
                for _ in 0..size {
                    let entry = s.edges.get_mut(h).ok_or_else(|| corrupt("edge table"))?;
                    if entry.0 == EMPTY_EDGE {
                        *entry = (key, t, k);
                        open += 1;
                        placed = true;
                        break;
                    }
                    if entry.0 == key {
                        let (_, u, l) = *entry;
                        if u == NONE {
                            return Err(corrupt("an edge of the cavity boundary has more than two faces"));
                        }
                        entry.1 = NONE;
                        open -= 1;
                        self.set_adj(t, k, u)?;
                        self.set_adj(u, l, t)?;
                        placed = true;
                        break;
                    }
                    h = (h + 1) & mask;
                }
                if !placed {
                    return Err(corrupt("edge table full"));
                }
            }
            self.hint = t;
        }
        if open != 0 {
            return Err(corrupt("the cavity boundary is not a closed surface"));
        }
        Ok(())
    }
}
