//! Planar constrained Delaunay triangulation with Delaunay refinement to a size field.
//!
//! - Insertion: Bowyer–Watson (Bowyer, Comput. J. 24(2), 1981; Watson, Comput. J. 24(2), 1981)
//!   inside a super-triangle, with exact `orient2d`/`incircle` (Shewchuk 1997). The cavity grows
//!   from the triangle containing the point and never crosses a constrained edge, which is the
//!   constrained Delaunay insertion of Shewchuk, *Delaunay refinement algorithms for triangular
//!   mesh generation*, CGTA 22:21–74, 2002, §2.
//! - Constraint recovery: Sloan, *A fast algorithm for generating constrained Delaunay
//!   triangulations*, Computers & Structures 47(3):441–450, 1993: flip the edges crossing a
//!   segment until it appears, then restore the Delaunay property with Lawson flips.
//! - Refinement: Ruppert, *A Delaunay refinement algorithm for quality 2-dimensional mesh
//!   generation*, J. Algorithms 18(3):548–585, 1995 (circumcentres of triangles that are too
//!   large for the size field or have a circumradius-to-shortest-edge ratio above √2), with one
//!   change: boundary segments carry prescribed nodes and are never split, so a circumcentre that
//!   falls outside the region, comes closer than a fraction of the local size to a vertex, or is
//!   refused by the caller's guard (clearance from the true boundary) is not inserted (a
//!   too-large triangle gets its centroid instead). Ruppert's encroachment rule (diametral
//!   circles) would forbid any refinement next to a boundary coarser than the size field; the
//!   clearances keep the packing argument for termination.
//!
//! Every loop is bounded (walk steps, cavity rounds, flips, refinement passes and the triangle
//! count), and a cavity is checked before it is retriangulated: the new triangles all have
//! positive exact orientation and no constrained edge is removed.

use std::collections::VecDeque;

use super::domain::Region;
use crate::predicates::{Point2, Sign, incircle, orient2d, snap_to_domain};
use crate::size::{SizeField, checked_size};
use crate::{Error, Result};

const NONE: u32 = u32::MAX;
/// Rounds of cavity shrinking before an insertion is rejected.
const MAX_CAVITY_ROUNDS: usize = 32;
/// Refinement passes over the triangles.
const MAX_REFINE_PASSES: usize = 64;

#[derive(Debug, Clone, Copy)]
struct Tri {
    /// Counterclockwise vertices.
    v: [u32; 3],
    /// `n[i]`: the neighbour across the edge opposite `v[i]` (from `v[i+1]` to `v[i+2]`).
    n: [u32; 3],
    /// Whether the edge opposite `v[i]` is a constrained (boundary) edge.
    fixed: [bool; 3],
    inside: bool,
    alive: bool,
}

/// The triangles of the region: counterclockwise vertex triples into `pts`.
#[derive(Debug, Clone)]
pub(crate) struct Triangulation {
    pub pts: Vec<[f64; 2]>,
    pub p2: Vec<Point2>,
    pub tris: Vec<[u32; 3]>,
}

pub(crate) struct Cdt {
    pts: Vec<[f64; 2]>,
    p2: Vec<Point2>,
    tris: Vec<Tri>,
    free: Vec<u32>,
    vt: Vec<u32>,
    last: u32,
    mark: Vec<u32>,
    epoch: u32,
    max_tris: usize,
}

/// Steiner points keep this fraction of the local size from vertices (and, through the
/// caller's guard, from the boundary).
pub(crate) const CLEARANCE: f64 = 0.3;

/// A constructed point snapped into the predicates' domain, or `None` when it is outside the
/// model's coordinate range.
fn steiner_point(c: [f64; 2]) -> Option<Point2> {
    let c = [snap_to_domain(c[0]), snap_to_domain(c[1])];
    if !(c[0].abs() <= modelcraft_model::MAX_COORD && c[1].abs() <= modelcraft_model::MAX_COORD) {
        return None;
    }
    Point2::new(c[0], c[1]).ok()
}

fn err(what: &str) -> Error {
    Error::Failed(format!("triangulation: {what}"))
}

/// The edge index `i` of `tri` that runs from `a` to `b` (opposite `v[i]`).
fn edge_index(tri: &Tri, a: u32, b: u32) -> Option<usize> {
    (0..3).find(|&i| tri.v[(i + 1) % 3] == a && tri.v[(i + 2) % 3] == b)
}

/// Every edge of `tri` keeps `p` on its left or on it (exact).
fn contains(tri: &Tri, p2: &[Point2], p: &Point2) -> bool {
    (0..3).all(|i| match (p2.get(tri.v[(i + 1) % 3] as usize), p2.get(tri.v[(i + 2) % 3] as usize)) {
        (Some(a), Some(b)) => orient2d(a, b, p) != Sign::Negative,
        _ => false,
    })
}

impl Cdt {
    fn t(&self, t: u32) -> Result<&Tri> {
        self.tris.get(t as usize).ok_or_else(|| err("triangle index out of range"))
    }
    fn tm(&mut self, t: u32) -> Result<&mut Tri> {
        self.tris.get_mut(t as usize).ok_or_else(|| err("triangle index out of range"))
    }
    fn p(&self, v: u32) -> Result<&Point2> {
        self.p2.get(v as usize).ok_or_else(|| err("vertex index out of range"))
    }
    fn xy(&self, v: u32) -> Result<[f64; 2]> {
        self.pts.get(v as usize).copied().ok_or_else(|| err("vertex index out of range"))
    }

    /// Triangulate `region`'s boundary points (ids preserved) and recover its loops as
    /// constrained edges; triangles inside the region are marked.
    pub fn build(region: &Region, max_tris: usize) -> Result<Cdt> {
        let n = region.pts.len();
        let (lo, hi) = super::domain::bbox(region.pts.iter().copied());
        let c = [0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])];
        let d = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
        let corners = [[c[0] - 20.0 * d, c[1] - 10.0 * d], [c[0] + 20.0 * d, c[1] - 10.0 * d], [c[0], c[1] + 20.0 * d]];
        let mut pts = region.pts.clone();
        let mut p2 = region.p2.clone();
        for k in corners {
            let k = [snap_to_domain(k[0]), snap_to_domain(k[1])];
            p2.push(Point2::new(k[0], k[1])?);
            pts.push(k);
        }
        let s = n as u32;
        let mut cdt = Cdt {
            vt: vec![NONE; pts.len()],
            pts,
            p2,
            tris: vec![Tri { v: [s, s + 1, s + 2], n: [NONE; 3], fixed: [false; 3], inside: false, alive: true }],
            free: Vec::new(),
            last: 0,
            mark: vec![0],
            epoch: 0,
            max_tris,
        };
        for v in [s, s + 1, s + 2] {
            if let Some(x) = cdt.vt.get_mut(v as usize) {
                *x = 0;
            }
        }
        // Insert in Morton order (short walks), ties by id.
        let span = [(hi[0] - lo[0]).max(f64::MIN_POSITIVE), (hi[1] - lo[1]).max(f64::MIN_POSITIVE)];
        let key = |p: &[f64; 2]| {
            let q = |x: f64, l: f64, s: f64| ((((x - l) / s) * 65535.0) as u32).min(65535);
            let (mut x, mut y) = (q(p[0], lo[0], span[0]), q(p[1], lo[1], span[1]));
            let mut k = 0u64;
            for bit in 0..16 {
                k |= u64::from(x & 1) << (2 * bit) | u64::from(y & 1) << (2 * bit + 1);
                x >>= 1;
                y >>= 1;
            }
            k
        };
        let mut order: Vec<(u64, u32)> = region.pts.iter().enumerate().map(|(i, p)| (key(p), i as u32)).collect();
        order.sort_unstable();
        for (_, v) in order {
            if !cdt.insert(v, None)? {
                return Err(err("a boundary point could not be inserted"));
            }
        }
        for l in &region.loops {
            for (i, &a) in l.iter().enumerate() {
                let b = l.get((i + 1) % l.len()).copied().ok_or_else(|| err("loop"))?;
                cdt.recover(a, b)?;
            }
        }
        cdt.classify()?;
        #[cfg(test)]
        assert_eq!(cdt.verify(), Ok(()), "after build");
        Ok(cdt)
    }

    fn alloc(&mut self, tri: Tri) -> Result<u32> {
        if let Some(t) = self.free.pop() {
            *self.tm(t)? = tri;
            return Ok(t);
        }
        if self.tris.len() >= self.max_tris {
            return Err(Error::Limit(format!("the triangulation exceeds {} triangles", self.max_tris)));
        }
        self.tris.push(tri);
        self.mark.push(0);
        Ok(self.tris.len() as u32 - 1)
    }

    /// The triangle containing `p` (on its boundary counts), by a visibility walk from `start`
    /// with a linear scan as the fallback.
    fn locate(&self, p: &Point2, start: u32) -> Result<u32> {
        let mut t = if self.t(start).is_ok_and(|x| x.alive) { start } else { self.any_alive()? };
        let cap = 64 + 4 * self.tris.len();
        'walk: for step in 0..cap {
            let tri = *self.t(t)?;
            for k in 0..3 {
                let i = (k + step) % 3;
                let (a, b) = (tri.v[(i + 1) % 3], tri.v[(i + 2) % 3]);
                if orient2d(self.p(a)?, self.p(b)?, p) == Sign::Negative {
                    let next = tri.n[i];
                    if next == NONE {
                        return Err(err("point outside the super-triangle"));
                    }
                    t = next;
                    continue 'walk;
                }
            }
            return Ok(t);
        }
        self.tris.iter().position(|tri| tri.alive && contains(tri, &self.p2, p)).map(|i| i as u32).ok_or_else(|| err("point location failed"))
    }

    fn any_alive(&self) -> Result<u32> {
        self.tris.iter().position(|t| t.alive).map(|i| i as u32).ok_or_else(|| err("no triangles"))
    }

    /// Insert vertex `v` (already in `pts`). Boundary points (`steiner == None`) must be inserted.
    /// Steiner points (`Some(min_distance)`) may be rejected (outside the region, closer than
    /// `min_distance` to a vertex, or no valid cavity), leaving the triangulation unchanged;
    /// returns whether the point went in.
    fn insert(&mut self, v: u32, steiner: Option<f64>) -> Result<bool> {
        let p = *self.p(v)?;
        let t0 = self.locate(&p, self.last)?;
        let tri0 = *self.t(t0)?;
        for &u in &tri0.v {
            if self.p(u)?.coords() == p.coords() {
                return match steiner {
                    Some(_) => Ok(false),
                    None => Err(err("repeated point")),
                };
            }
        }
        if steiner.is_some() && !tri0.inside {
            return Ok(false);
        }
        let mut excluded: Vec<u32> = Vec::new();
        for _ in 0..MAX_CAVITY_ROUNDS {
            let (cavity, edges) = self.cavity(t0, &p, &excluded)?;
            let mut bad: Vec<u32> = Vec::new();
            // A constrained edge inside the cavity: drop the triangle on the far side from p.
            for &t in &cavity {
                let tri = *self.t(t)?;
                for i in 0..3 {
                    if tri.fixed[i] && tri.n[i] != NONE && self.marked(tri.n[i]) {
                        let (a, b) = (tri.v[(i + 1) % 3], tri.v[(i + 2) % 3]);
                        if orient2d(self.p(a)?, self.p(b)?, &p) != Sign::Positive {
                            bad.push(t);
                        }
                    }
                }
            }
            // Every new triangle (a, b, p) must be counterclockwise.
            for &(a, b, _, _, owner) in &edges {
                if orient2d(self.p(a)?, self.p(b)?, &p) != Sign::Positive {
                    bad.push(owner);
                }
            }
            if let Some(min_distance) = steiner {
                let pc = p.coords();
                for &(a, _, _, _, _) in &edges {
                    let pa = self.xy(a)?;
                    if (pa[0] - pc[0]).powi(2) + (pa[1] - pc[1]).powi(2) < min_distance * min_distance {
                        return Ok(false);
                    }
                }
            }
            if bad.is_empty() && edges.len() == cavity.len() + 2 {
                self.retriangulate(v, &cavity, &edges, tri0.inside)?;
                return Ok(true);
            }
            if bad.contains(&t0) || bad.is_empty() {
                break;
            }
            excluded.extend(bad);
            excluded.sort_unstable();
            excluded.dedup();
        }
        match steiner {
            Some(_) => Ok(false),
            None => Err(err("no valid cavity for a boundary point")),
        }
    }

    fn marked(&self, t: u32) -> bool {
        self.mark.get(t as usize).is_some_and(|&m| m == self.epoch)
    }

    /// The Bowyer–Watson cavity of `p` from `t0` (never crossing constrained edges, skipping
    /// `excluded`), marked with the current epoch, and its boundary edges `(a, b, outside
    /// neighbour, constrained, owner)` with the cavity on their left.
    #[allow(clippy::type_complexity)]
    fn cavity(&mut self, t0: u32, p: &Point2, excluded: &[u32]) -> Result<(Vec<u32>, Vec<(u32, u32, u32, bool, u32)>)> {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.mark.iter_mut().for_each(|m| *m = 0);
            self.epoch = 1;
        }
        let epoch = self.epoch;
        let set = |mark: &mut Vec<u32>, t: u32| {
            if let Some(m) = mark.get_mut(t as usize) {
                *m = epoch;
            }
        };
        set(&mut self.mark, t0);
        let mut cavity = vec![t0];
        let mut k = 0;
        while let Some(&t) = cavity.get(k) {
            k += 1;
            let tri = *self.t(t)?;
            for i in 0..3 {
                let nb = tri.n[i];
                if nb == NONE || tri.fixed[i] || self.marked(nb) || excluded.binary_search(&nb).is_ok() {
                    continue;
                }
                let o = *self.t(nb)?;
                if incircle(self.p(o.v[0])?, self.p(o.v[1])?, self.p(o.v[2])?, p) == Sign::Positive {
                    set(&mut self.mark, nb);
                    cavity.push(nb);
                }
            }
        }
        let mut edges = Vec::new();
        for &t in &cavity {
            let tri = *self.t(t)?;
            for i in 0..3 {
                let nb = tri.n[i];
                // Shared with another cavity triangle (a constrained one is reported by `insert`).
                if nb != NONE && self.marked(nb) {
                    continue;
                }
                edges.push((tri.v[(i + 1) % 3], tri.v[(i + 2) % 3], nb, tri.fixed[i], t));
            }
        }
        Ok((cavity, edges))
    }

    fn retriangulate(&mut self, v: u32, cavity: &[u32], edges: &[(u32, u32, u32, bool, u32)], inside: bool) -> Result<()> {
        for &t in cavity {
            self.tm(t)?.alive = false;
            self.free.push(t);
        }
        let mut made: Vec<(u32, u32, u32)> = Vec::with_capacity(edges.len()); // (a, b, tri)
        for &(a, b, nb, fixed, _) in edges {
            let t = self.alloc(Tri { v: [a, b, v], n: [NONE, NONE, nb], fixed: [false, false, fixed], inside, alive: true })?;
            made.push((a, b, t));
            if nb != NONE {
                self.relink(nb, a, b, t)?;
            }
        }
        let mut by_a: Vec<(u32, u32)> = made.iter().map(|&(a, _, t)| (a, t)).collect();
        let mut by_b: Vec<(u32, u32)> = made.iter().map(|&(_, b, t)| (b, t)).collect();
        by_a.sort_unstable();
        by_b.sort_unstable();
        let find = |list: &[(u32, u32)], key: u32| -> Result<u32> {
            let i = list.binary_search_by_key(&key, |e| e.0).map_err(|_| err("cavity boundary is not a closed loop"))?;
            list.get(i).map(|e| e.1).ok_or_else(|| err("cavity lookup"))
        };
        for &(a, b, t) in &made {
            let next = find(&by_a, b)?; // the triangle (b, c, v): shares edge b–v
            let prev = find(&by_b, a)?; // the triangle (z, a, v): shares edge v–a
            let tri = self.tm(t)?;
            tri.n[0] = next;
            tri.n[1] = prev;
            for u in [a, b, v] {
                if let Some(x) = self.vt.get_mut(u as usize) {
                    *x = t;
                }
            }
            self.last = t;
        }
        Ok(())
    }

    /// In triangle `t`, point the edge from `b` to `a` at `to`.
    fn relink(&mut self, t: u32, a: u32, b: u32, to: u32) -> Result<()> {
        let tri = self.tm(t)?;
        let i = edge_index(tri, b, a).ok_or_else(|| err("neighbour does not share the edge"))?;
        tri.n[i] = to;
        Ok(())
    }

    /// Visit the triangles around vertex `v` counterclockwise, each with `v`'s local index,
    /// until `visit` returns a value (or the rotation closes: `None`).
    fn around<R>(&self, v: u32, mut visit: impl FnMut(u32, &Tri, usize) -> Result<Option<R>>) -> Result<Option<R>> {
        let start = self.vt.get(v as usize).copied().ok_or_else(|| err("vertex"))?;
        let mut t = start;
        for _ in 0..self.tris.len() + 1 {
            let tri = self.t(t)?;
            let k = tri.v.iter().position(|&x| x == v).ok_or_else(|| err("vertex map is stale"))?;
            if let Some(r) = visit(t, tri, k)? {
                return Ok(Some(r));
            }
            t = tri.n[(k + 1) % 3];
            if t == NONE || t == start {
                return Ok(None);
            }
        }
        Err(err("vertex rotation did not close"))
    }

    /// The triangle and local index of the directed edge `a → b` (the edge opposite `v[i]` runs
    /// from `v[i+1]` to `v[i+2]`).
    fn find_edge(&self, a: u32, b: u32) -> Result<Option<(u32, usize)>> {
        self.around(a, |t, tri, k| Ok((tri.v[(k + 1) % 3] == b).then_some((t, (k + 2) % 3))))
    }

    /// Flip the edge opposite `v[i]` of `t`. Returns the new edge's end points.
    fn flip(&mut self, t: u32, i: usize) -> Result<(u32, u32)> {
        let a = *self.t(t)?;
        let (p, q, r) = (a.v[i], a.v[(i + 1) % 3], a.v[(i + 2) % 3]);
        let t2 = a.n[i];
        let b = *self.t(t2)?;
        let j = edge_index(&b, r, q).ok_or_else(|| err("flip: neighbour mismatch"))?;
        let s = b.v[j];
        let (n_pq, f_pq) = (a.n[(i + 2) % 3], a.fixed[(i + 2) % 3]);
        let (n_rp, f_rp) = (a.n[(i + 1) % 3], a.fixed[(i + 1) % 3]);
        let (n_sr, f_sr) = (b.n[(j + 2) % 3], b.fixed[(j + 2) % 3]);
        let (n_qs, f_qs) = (b.n[(j + 1) % 3], b.fixed[(j + 1) % 3]);
        *self.tm(t)? = Tri { v: [p, q, s], n: [n_qs, t2, n_pq], fixed: [f_qs, false, f_pq], inside: a.inside, alive: true };
        *self.tm(t2)? = Tri { v: [p, s, r], n: [n_sr, n_rp, t], fixed: [f_sr, f_rp, false], inside: b.inside, alive: true };
        if n_qs != NONE {
            self.relink(n_qs, q, s, t)?;
        }
        if n_rp != NONE {
            self.relink(n_rp, r, p, t2)?;
        }
        for (u, x) in [(p, t), (q, t), (s, t2), (r, t2)] {
            if let Some(y) = self.vt.get_mut(u as usize) {
                *y = x;
            }
        }
        Ok((p, s))
    }

    /// The edge opposite `v[i]` of `t` with its far vertex, when it is flippable (strictly convex
    /// quadrilateral).
    fn flippable(&self, t: u32, i: usize) -> Result<Option<u32>> {
        let a = *self.t(t)?;
        let t2 = a.n[i];
        if t2 == NONE || a.fixed[i] {
            return Ok(None);
        }
        let (p, q, r) = (a.v[i], a.v[(i + 1) % 3], a.v[(i + 2) % 3]);
        let b = *self.t(t2)?;
        let s = b.v[edge_index(&b, r, q).ok_or_else(|| err("neighbour mismatch"))?];
        let (pp, ps) = (self.p(p)?, self.p(s)?);
        Ok((orient2d(pp, ps, self.p(q)?) * orient2d(pp, ps, self.p(r)?) == Sign::Negative).then_some(s))
    }

    /// Make `a–b` an edge (Sloan 1993) and mark it constrained.
    fn recover(&mut self, a: u32, b: u32) -> Result<()> {
        if self.find_edge(a, b)?.is_none() {
            let crossing = self.crossing_edges(a, b)?;
            let (pa, pb) = (*self.p(a)?, *self.p(b)?);
            let mut queue: VecDeque<(u32, u32)> = crossing.into_iter().collect();
            let cap = 1000 + 16 * queue.len() * queue.len();
            let mut fresh = Vec::new();
            let mut steps = 0usize;
            while let Some((u, w)) = queue.pop_front() {
                steps += 1;
                if steps > cap {
                    return Err(Error::Limit(format!("boundary edge recovery did not finish in {cap} flips")));
                }
                let (t, i) = self.find_edge(u, w)?.ok_or_else(|| err("crossing edge vanished"))?;
                match self.flippable(t, i)? {
                    Some(_) => {
                        let (x, y) = self.flip(t, i)?;
                        let crosses =
                            x != a && x != b && y != a && y != b && orient2d(&pa, &pb, self.p(x)?) * orient2d(&pa, &pb, self.p(y)?) == Sign::Negative;
                        if crosses { queue.push_back((x, y)) } else { fresh.push((x, y)) }
                    }
                    None => queue.push_back((u, w)),
                }
            }
            self.mark_fixed(a, b)?;
            self.lawson(fresh)?;
            return Ok(());
        }
        self.mark_fixed(a, b)
    }

    fn mark_fixed(&mut self, a: u32, b: u32) -> Result<()> {
        let (t, i) = self.find_edge(a, b)?.ok_or_else(|| err("recovered edge is missing"))?;
        let nb = self.t(t)?.n[i];
        self.tm(t)?.fixed[i] = true;
        if nb != NONE {
            let tri = self.tm(nb)?;
            if let Some(k) = edge_index(tri, b, a) {
                tri.fixed[k] = true;
            }
        }
        Ok(())
    }

    /// The edges crossed by the open segment `a–b`, from `a` to `b`.
    fn crossing_edges(&self, a: u32, b: u32) -> Result<Vec<(u32, u32)>> {
        let (pa, pb) = (*self.p(a)?, *self.p(b)?);
        let found = self.around(a, |t, tri, k| {
            let (u, w) = (tri.v[(k + 1) % 3], tri.v[(k + 2) % 3]);
            let (ou, ow) = (orient2d(&pa, self.p(u)?, &pb), orient2d(&pa, self.p(w)?, &pb));
            if ou.is_zero() || ow.is_zero() {
                let x = if ou.is_zero() { u } else { w };
                let px = self.xy(x)?;
                let (qa, qb) = (pa.coords(), pb.coords());
                if (px[0] - qa[0]) * (qb[0] - qa[0]) + (px[1] - qa[1]) * (qb[1] - qa[1]) > 0.0 {
                    return Err(Error::Invalid("a point lies on a boundary edge".into()));
                }
            }
            Ok((ou == Sign::Positive && ow == Sign::Negative).then_some((t, u, w)))
        })?;
        let (mut t, mut right, mut left) = found.ok_or_else(|| err("segment start not found"))?;
        let mut out = vec![(right, left)];
        for _ in 0..self.tris.len() + 1 {
            let tri = *self.t(t)?;
            let i = tri.v.iter().position(|&x| x != right && x != left).ok_or_else(|| err("walk"))?;
            let next = tri.n[i];
            let nt = *self.t(next)?;
            let d = *nt.v.iter().find(|&&x| x != right && x != left).ok_or_else(|| err("walk"))?;
            if d == b {
                return Ok(out);
            }
            match orient2d(&pa, &pb, self.p(d)?) {
                Sign::Positive => left = d,
                Sign::Negative => right = d,
                Sign::Zero => return Err(Error::Invalid("a point lies on a boundary edge".into())),
            }
            out.push((right, left));
            t = next;
        }
        Err(err("segment walk did not reach its end"))
    }

    /// Lawson flips from the given edges until every unconstrained edge is locally Delaunay.
    fn lawson(&mut self, edges: Vec<(u32, u32)>) -> Result<()> {
        let mut stack = edges;
        let cap = 1000 + 64 * self.tris.len();
        let mut steps = 0usize;
        while let Some((u, w)) = stack.pop() {
            steps += 1;
            if steps > cap {
                return Err(Error::Limit("Delaunay restoration did not finish".into()));
            }
            let Some((t, i)) = self.find_edge(u, w)? else { continue };
            let tri = *self.t(t)?;
            if tri.fixed[i] || tri.n[i] == NONE {
                continue;
            }
            let Some(s) = self.flippable(t, i)? else { continue };
            let [a, b, c] = tri.v;
            if incircle(self.p(a)?, self.p(b)?, self.p(c)?, self.p(s)?) != Sign::Positive {
                continue;
            }
            let (p, _) = self.flip(t, i)?;
            let (q, r) = (tri.v[(i + 1) % 3], tri.v[(i + 2) % 3]);
            stack.extend([(p, q), (q, s), (s, r), (r, p)]);
        }
        Ok(())
    }

    /// Mark the triangles inside the region: those separated from the super-triangle by an odd
    /// number of constrained edges.
    fn classify(&mut self) -> Result<()> {
        let n = self.tris.len();
        let mut depth = vec![u32::MAX; n];
        let start = self.vt.get(self.pts.len().saturating_sub(1)).copied().ok_or_else(|| err("super vertex"))?;
        let mut current = vec![start];
        if let Some(d) = depth.get_mut(start as usize) {
            *d = 0;
        }
        let mut level = 0u32;
        while !current.is_empty() {
            let mut next = Vec::new();
            let mut k = 0;
            while let Some(&t) = current.get(k) {
                k += 1;
                let tri = *self.t(t)?;
                for i in 0..3 {
                    let nb = tri.n[i];
                    if nb == NONE || depth.get(nb as usize).is_none_or(|&d| d != u32::MAX) {
                        continue;
                    }
                    if tri.fixed[i] {
                        next.push(nb);
                    } else {
                        if let Some(d) = depth.get_mut(nb as usize) {
                            *d = level;
                        }
                        current.push(nb);
                    }
                }
            }
            level += 1;
            next.retain(|&t| depth.get(t as usize).is_some_and(|&d| d == u32::MAX));
            next.sort_unstable();
            next.dedup();
            for &t in &next {
                if let Some(d) = depth.get_mut(t as usize) {
                    *d = level;
                }
            }
            current = next;
            if level > 4 {
                break;
            }
        }
        for (tri, d) in self.tris.iter_mut().zip(depth) {
            tri.inside = tri.alive && d == 1;
        }
        Ok(())
    }

    /// Try to insert Steiner point `p` (located from `hint`), at least `min_distance` from the
    /// vertices of its cavity.
    fn try_steiner(&mut self, p: Point2, hint: u32, min_distance: f64) -> Result<bool> {
        let v = self.pts.len() as u32;
        self.pts.push(p.coords());
        self.p2.push(p);
        self.vt.push(NONE);
        self.last = hint;
        let ok = self.insert(v, Some(min_distance))?;
        if !ok {
            self.pts.pop();
            self.p2.pop();
            self.vt.pop();
        }
        Ok(ok)
    }

    /// Delaunay refinement of the inside triangles to `scale × size field`. A Steiner point `p`
    /// for local size `h` is skipped when `allows(p, h)` refuses it (the caller's clearance from
    /// the true boundary) or when it comes closer than [`CLEARANCE`]` × h` to a vertex.
    pub fn refine(&mut self, size: &dyn SizeField, scale: f64, allows: &dyn Fn(&Point2, f64) -> bool) -> Result<()> {
        for _ in 0..MAX_REFINE_PASSES {
            let mut inserted = 0usize;
            let n = self.tris.len();
            for t in 0..n as u32 {
                let tri = *self.t(t)?;
                if !tri.alive || !tri.inside {
                    continue;
                }
                let [a, b, c] = [self.xy(tri.v[0])?, self.xy(tri.v[1])?, self.xy(tri.v[2])?];
                let centroid = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0];
                let h = scale * checked_size(size, [centroid[0], centroid[1], 0.0])?;
                let (bx, by, cx, cy) = (b[0] - a[0], b[1] - a[1], c[0] - a[0], c[1] - a[1]);
                let d = 2.0 * (bx * cy - by * cx);
                let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
                let (ux, uy) = ((cy * b2 - by * c2) / d, (bx * c2 - cx * b2) / d);
                let r = (ux * ux + uy * uy).sqrt();
                let shortest = b2.min(c2).min((cx - bx).powi(2) + (cy - by).powi(2)).sqrt();
                if !r.is_finite() || !(d > 0.0) {
                    continue;
                }
                let too_big = r > 0.75 * h;
                let poor = r > std::f64::consts::SQRT_2 * shortest;
                if !too_big && !poor {
                    continue;
                }
                let candidates = [Some([a[0] + ux, a[1] + uy]), too_big.then_some(centroid)];
                for c in candidates.into_iter().flatten() {
                    if let Some(p) = steiner_point(c).filter(|p| allows(p, h))
                        && self.try_steiner(p, t, CLEARANCE * h)?
                    {
                        inserted += 1;
                        break;
                    }
                }
            }
            if inserted == 0 {
                break;
            }
        }
        #[cfg(test)]
        assert_eq!(self.verify(), Ok(()), "after refinement");
        Ok(())
    }

    /// The inside triangles around vertex `v`.
    fn inside_fan(&self, v: u32) -> Result<Vec<u32>> {
        let mut fan = Vec::new();
        self.around(v, |t, tri, _| {
            if tri.inside {
                fan.push(t);
            }
            Ok(None::<()>)
        })?;
        Ok(fan)
    }

    /// [`Cdt::ensure_fan`] for every vertex with its count in `need`.
    pub fn ensure_fans(&mut self, need: &[usize]) -> Result<()> {
        for (v, &k) in need.iter().enumerate() {
            if k > 1 {
                self.ensure_fan(v as u32, k)?;
            }
        }
        #[cfg(test)]
        assert_eq!(self.verify(), Ok(()), "after fans");
        Ok(())
    }

    /// Give boundary vertex `v` at least `count` inside triangles by inserting the centroid of
    /// its widest triangle (bounded attempts; the caller's validity gate judges the result).
    fn ensure_fan(&mut self, v: u32, count: usize) -> Result<()> {
        for _ in 0..count + 2 {
            let fan = self.inside_fan(v)?;
            if fan.len() >= count {
                return Ok(());
            }
            let pv = self.xy(v)?;
            let mut widest: Option<(f64, u32, [f64; 2])> = None;
            for &t in &fan {
                let tri = *self.t(t)?;
                let k = tri.v.iter().position(|&x| x == v).ok_or_else(|| err("fan"))?;
                let (a, b) = (self.xy(tri.v[(k + 1) % 3])?, self.xy(tri.v[(k + 2) % 3])?);
                let (u, w) = ([a[0] - pv[0], a[1] - pv[1]], [b[0] - pv[0], b[1] - pv[1]]);
                let cos = (u[0] * w[0] + u[1] * w[1]) / ((u[0] * u[0] + u[1] * u[1]).sqrt() * (w[0] * w[0] + w[1] * w[1]).sqrt());
                let c = [(pv[0] + a[0] + b[0]) / 3.0, (pv[1] + a[1] + b[1]) / 3.0];
                if widest.is_none_or(|(m, _, _)| cos < m) {
                    widest = Some((cos, t, c));
                }
            }
            let Some((t, p)) = widest.and_then(|(_, t, c)| steiner_point(c).map(|p| (t, p))) else { return Ok(()) };
            if !self.try_steiner(p, t, 0.0)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Consistency check for tests: every live triangle counterclockwise (exact), neighbours
    /// mutual, constrained flags matching on both sides.
    #[cfg(test)]
    pub fn verify(&self) -> std::result::Result<(), String> {
        for (t, tri) in self.tris.iter().enumerate() {
            if !tri.alive {
                continue;
            }
            let [a, b, c] = tri.v;
            let o = orient2d(&self.p2[a as usize], &self.p2[b as usize], &self.p2[c as usize]);
            if o != Sign::Positive {
                return Err(format!("triangle {t} {:?} has orientation {o:?} (inside {})", tri.v, tri.inside));
            }
            for i in 0..3 {
                let nb = tri.n[i];
                if nb == NONE {
                    continue;
                }
                let o = &self.tris[nb as usize];
                let (p, q) = (tri.v[(i + 1) % 3], tri.v[(i + 2) % 3]);
                let back = (0..3).find(|&j| o.v[(j + 1) % 3] == q && o.v[(j + 2) % 3] == p);
                match back {
                    Some(j) if o.n[j] == t as u32 && o.alive && o.fixed[j] == tri.fixed[i] => {
                        let s = o.v[j];
                        if !tri.fixed[i]
                            && incircle(&self.p2[a as usize], &self.p2[b as usize], &self.p2[c as usize], &self.p2[s as usize]) == Sign::Positive
                        {
                            return Err(format!("triangle {t} edge {i} is not locally Delaunay (inside {})", tri.inside));
                        }
                    }
                    _ => return Err(format!("triangle {t} edge {i}: neighbour {nb} does not match")),
                }
            }
        }
        Ok(())
    }

    /// The inside triangles.
    pub fn into_triangulation(self) -> Triangulation {
        let tris = self.tris.iter().filter(|t| t.alive && t.inside).map(|t| t.v).collect();
        Triangulation { pts: self.pts, p2: self.p2, tris }
    }
}
