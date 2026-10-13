//! The planar arrangement of the curves a multi-level sweep projects into its cross-section
//! plane: points merged within a tolerance, segments split where a point lies on them or where
//! two cross, then the faces of the planar graph (regions with their holes) and the curves
//! between curve ends.
//!
//! This is the imprinting step of many-to-many sweeping (Lai, Benzley, White, *Automated
//! hexahedral mesh generation by generalized multiple source to multiple target sweeping*,
//! IJNME 49(1–2):261–275, 2000): projecting every level's boundary into one plane splits the
//! cross-section into regions whose columns are each a one-to-one sweep. Crossing and side tests
//! use the exact `orient2d` predicate on the merged coordinates; the merge itself is a tolerance
//! (the projected copies of one point differ by rounding).

use std::collections::{BTreeMap, HashMap};

use crate::predicates::{Point2, Sign, orient2d, snap_to_domain};
use crate::{Error, Result};

pub(crate) type Q = [f64; 2];

/// The most segments one arrangement may hold.
pub(crate) const MAX_SEGMENTS: usize = 2_000_000;
/// The most rounds of splitting (each round settles every T-junction and crossing found).
const MAX_ROUNDS: usize = 16;

/// A curve of the arrangement: a chain of points between curve ends (or a closed chain).
#[derive(Debug, Clone)]
pub(crate) struct Curve {
    /// Point ids in order; closed curves repeat the first at the end.
    pub points: Vec<usize>,
    /// The tags of the input polylines it came from (sorted, unique).
    pub tags: Vec<usize>,
}

/// A bounded face of the arrangement.
#[derive(Debug, Clone)]
pub(crate) struct Region {
    /// Loops of oriented curves `(curve, forward)`: the outer loop first (counter-clockwise), then
    /// the holes (clockwise).
    pub loops: Vec<Vec<(usize, bool)>>,
}

#[derive(Debug, Clone)]
pub(crate) struct Arrangement {
    pub points: Vec<Q>,
    pub curves: Vec<Curve>,
    pub regions: Vec<Region>,
}

pub(super) fn d2(a: Q, b: Q) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// Squared distance from `p` to segment `ab`, and the parameter of the closest point.
pub(super) fn seg_dist2(p: Q, a: Q, b: Q) -> (f64, f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if !(len2 > 0.0) {
        return (d2(p, a), 0.0);
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
    (d2(p, [a[0] + t * dx, a[1] + t * dy]), t)
}

pub(super) fn sign(a: Q, b: Q, c: Q) -> Result<Sign> {
    Ok(orient2d(&Point2::new(a[0], a[1])?, &Point2::new(b[0], b[1])?, &Point2::new(c[0], c[1])?))
}

/// Points merged within a tolerance through a uniform grid; the first point of a cluster is its
/// representative, so the result follows insertion order.
struct Snapper {
    tol: f64,
    cell: f64,
    grid: HashMap<(i64, i64), Vec<usize>>,
    points: Vec<Q>,
}

impl Snapper {
    fn new(tol: f64) -> Self {
        Snapper { tol, cell: tol * 4.0, grid: HashMap::new(), points: Vec::new() }
    }

    fn key(&self, p: Q) -> (i64, i64) {
        ((p[0] / self.cell).floor() as i64, (p[1] / self.cell).floor() as i64)
    }

    fn add(&mut self, p: Q) -> Result<usize> {
        let p = [snap_to_domain(p[0]), snap_to_domain(p[1])];
        Point2::new(p[0], p[1])?;
        let (i, j) = self.key(p);
        let mut best: Option<(f64, usize)> = None;
        for di in -1..=1 {
            for dj in -1..=1 {
                for &k in self.grid.get(&(i + di, j + dj)).map(Vec::as_slice).unwrap_or(&[]) {
                    let d = d2(p, self.points.get(k).copied().unwrap_or([f64::INFINITY; 2]));
                    if d <= self.tol * self.tol && best.is_none_or(|(bd, bk)| d < bd || (d == bd && k < bk)) {
                        best = Some((d, k));
                    }
                }
            }
        }
        if let Some((_, k)) = best {
            return Ok(k);
        }
        self.points.push(p);
        let id = self.points.len() - 1;
        self.grid.entry((i, j)).or_default().push(id);
        Ok(id)
    }
}

/// A grid over boxes, for candidate queries.
struct BoxGrid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

/// The most grid cells one box may cover before the grid is judged too fine.
const MAX_CELLS_PER_BOX: i64 = 1 << 16;

impl BoxGrid {
    fn range(&self, lo: Q, hi: Q) -> Option<(i64, i64, i64, i64)> {
        let (i0, j0) = ((lo[0] / self.cell).floor() as i64, (lo[1] / self.cell).floor() as i64);
        let (i1, j1) = ((hi[0] / self.cell).floor() as i64, (hi[1] / self.cell).floor() as i64);
        ((i1 - i0 + 1).saturating_mul(j1 - j0 + 1) <= MAX_CELLS_PER_BOX).then_some((i0, j0, i1, j1))
    }

    fn insert(&mut self, id: usize, lo: Q, hi: Q) -> Result<()> {
        let (i0, j0, i1, j1) = self.range(lo, hi).ok_or_else(|| Error::Limit("a segment covers too many grid cells".into()))?;
        for i in i0..=i1 {
            for j in j0..=j1 {
                self.cells.entry((i, j)).or_default().push(id);
            }
        }
        Ok(())
    }

    /// Ids in the cells the box covers (sorted, unique).
    fn query(&self, lo: Q, hi: Q) -> Result<Vec<usize>> {
        let (i0, j0, i1, j1) = self.range(lo, hi).ok_or_else(|| Error::Limit("a query covers too many grid cells".into()))?;
        let mut out = Vec::new();
        for i in i0..=i1 {
            for j in j0..=j1 {
                out.extend(self.cells.get(&(i, j)).map(Vec::as_slice).unwrap_or(&[]));
            }
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }
}

fn bbox(a: Q, b: Q, pad: f64) -> (Q, Q) {
    ([a[0].min(b[0]) - pad, a[1].min(b[1]) - pad], [a[0].max(b[0]) + pad, a[1].max(b[1]) + pad])
}

/// Build the arrangement of `polylines` (each with a tag), with `vertices` (curve ends that must
/// stay ends) merged into it. Points closer than `tol` are one point. Two input polylines with
/// the same end points that stay within `coincide` of each other are one curve (the copies of a
/// wall's outline at different levels).
pub(crate) fn build(vertices: &[Q], polylines: &[(Vec<Q>, usize)], tol: f64, coincide: f64) -> Result<Arrangement> {
    if !(tol > 0.0 && tol.is_finite() && coincide >= 0.0 && coincide.is_finite()) {
        return Err(Error::Invalid("arrangement tolerances must be positive and finite".into()));
    }
    let mut snap = Snapper::new(tol);
    let mut is_end = Vec::new();
    for &v in vertices {
        let id = snap.add(v)?;
        if is_end.len() <= id {
            is_end.resize(id + 1, false);
        }
        if let Some(x) = is_end.get_mut(id) {
            *x = true;
        }
    }
    // Polylines as point ids, dropping repeats; then coincident duplicates.
    let mut lines: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    let mut by_ends: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (pl, tag) in polylines {
        let mut ids: Vec<usize> = Vec::with_capacity(pl.len());
        for &p in pl {
            let id = snap.add(p)?;
            if ids.last() != Some(&id) {
                ids.push(id);
            }
        }
        if ids.len() < 2 {
            continue;
        }
        let (a, b) = (ids.first().copied().unwrap_or(0), ids.last().copied().unwrap_or(0));
        let key = (a.min(b), a.max(b));
        let pts = &snap.points;
        let at = |i: usize| pts.get(i).copied().unwrap_or([0.0; 2]);
        let near = |p: Q, other: &[usize]| other.windows(2).any(|w| seg_dist2(p, at(w[0]), at(w[1])).0 <= coincide * coincide);
        let same = by_ends.get(&key).and_then(|cands| {
            cands
                .iter()
                .copied()
                .find(|&k| lines.get(k).is_some_and(|(other, _)| ids.iter().all(|&i| near(at(i), other)) && other.iter().all(|&i| near(at(i), &ids))))
        });
        match same {
            Some(k) => {
                if let Some(l) = lines.get_mut(k) {
                    l.1.push(*tag);
                }
            }
            None => {
                by_ends.entry(key).or_default().push(lines.len());
                lines.push((ids, vec![*tag]));
            }
        }
    }
    // Segments, unique by end points.
    let mut segs: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (ids, tags) in &lines {
        for w in ids.windows(2) {
            let key = (w[0].min(w[1]), w[0].max(w[1]));
            segs.entry(key).or_default().extend(tags);
            if segs.len() > MAX_SEGMENTS {
                return Err(Error::Limit(format!("the arrangement exceeds {MAX_SEGMENTS} segments")));
            }
        }
    }
    let mut segs: Vec<((usize, usize), Vec<usize>)> = segs.into_iter().collect();
    // Split at points on segments and at crossings until nothing changes.
    let mut settled = false;
    for _ in 0..MAX_ROUNDS {
        let pts = snap.points.clone();
        let at = |i: usize| pts.get(i).copied().unwrap_or([0.0; 2]);
        let total: f64 = segs.iter().map(|((a, b), _)| d2(at(*a), at(*b)).sqrt()).sum();
        let cell = (total / segs.len().max(1) as f64).max(tol * 16.0);
        let mut seg_grid = BoxGrid { cell, cells: HashMap::new() };
        for (k, ((a, b), _)) in segs.iter().enumerate() {
            let (lo, hi) = bbox(at(*a), at(*b), tol);
            seg_grid.insert(k, lo, hi)?;
        }
        // Crossings add points.
        let mut added = false;
        for (k, ((a, b), _)) in segs.iter().enumerate() {
            let (lo, hi) = bbox(at(*a), at(*b), tol);
            for other in seg_grid.query(lo, hi)? {
                if other <= k {
                    continue;
                }
                let Some(((c, d), _)) = segs.get(other) else { continue };
                if a == c || a == d || b == c || b == d {
                    continue;
                }
                let (pa, pb, pc, pd) = (at(*a), at(*b), at(*c), at(*d));
                let (o1, o2) = (sign(pa, pb, pc)?, sign(pa, pb, pd)?);
                let (o3, o4) = (sign(pc, pd, pa)?, sign(pc, pd, pb)?);
                let opposite = |x: Sign, y: Sign| matches!((x, y), (Sign::Positive, Sign::Negative) | (Sign::Negative, Sign::Positive));
                if opposite(o1, o2) && opposite(o3, o4) {
                    let den = (pb[0] - pa[0]) * (pd[1] - pc[1]) - (pb[1] - pa[1]) * (pd[0] - pc[0]);
                    let t = ((pc[0] - pa[0]) * (pd[1] - pc[1]) - (pc[1] - pa[1]) * (pd[0] - pc[0])) / den;
                    if t.is_finite() {
                        let before = snap.points.len();
                        snap.add([pa[0] + t * (pb[0] - pa[0]), pa[1] + t * (pb[1] - pa[1])])?;
                        added |= snap.points.len() > before;
                    }
                }
            }
        }
        // Points on segments split them.
        let pts = snap.points.clone();
        let at = |i: usize| pts.get(i).copied().unwrap_or([0.0; 2]);
        let mut pt_grid = BoxGrid { cell, cells: HashMap::new() };
        for (i, p) in pts.iter().enumerate() {
            pt_grid.insert(i, *p, *p)?;
        }
        let mut split = false;
        let mut next: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
        for ((a, b), tags) in &segs {
            let (pa, pb) = (at(*a), at(*b));
            let (lo, hi) = bbox(pa, pb, tol);
            let mut on: Vec<(f64, usize)> = Vec::new();
            for p in pt_grid.query(lo, hi)? {
                if p == *a || p == *b {
                    continue;
                }
                let (dd, t) = seg_dist2(at(p), pa, pb);
                if dd <= tol * tol && t > 0.0 && t < 1.0 {
                    on.push((t, p));
                }
            }
            on.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
            split |= !on.is_empty();
            let chain: Vec<usize> = std::iter::once(*a).chain(on.into_iter().map(|x| x.1)).chain(std::iter::once(*b)).collect();
            for w in chain.windows(2) {
                if w[0] != w[1] {
                    next.entry((w[0].min(w[1]), w[0].max(w[1]))).or_default().extend(tags);
                }
            }
            if next.len() > MAX_SEGMENTS {
                return Err(Error::Limit(format!("the arrangement exceeds {MAX_SEGMENTS} segments")));
            }
        }
        segs = next.into_iter().collect();
        if !split && !added {
            settled = true;
            break;
        }
    }
    if !settled {
        return Err(Error::Failed("the projected curves did not settle into a planar arrangement".into()));
    }
    for (_, tags) in segs.iter_mut() {
        tags.sort_unstable();
        tags.dedup();
    }
    faces(snap.points, is_end, segs)
}

/// The planar graph's curves and bounded faces.
fn faces(points: Vec<Q>, mut is_end: Vec<bool>, segs: Vec<((usize, usize), Vec<usize>)>) -> Result<Arrangement> {
    let n = points.len();
    is_end.resize(n, false);
    let at = |i: usize| points.get(i).copied().unwrap_or([0.0; 2]);
    // Outgoing half-edges per point, counter-clockwise. Half-edge 2s goes a → b, 2s+1 b → a.
    let head = |h: usize| segs.get(h / 2).map_or(0, |((a, b), _)| if h.is_multiple_of(2) { *b } else { *a });
    let tail = |h: usize| segs.get(h / 2).map_or(0, |((a, b), _)| if h.is_multiple_of(2) { *a } else { *b });
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for h in 0..segs.len() * 2 {
        out.get_mut(tail(h)).ok_or_else(|| Error::Failed("arrangement point".into()))?.push(h);
    }
    for (p, list) in out.iter_mut().enumerate() {
        let o = at(p);
        let half = |h: usize| {
            let q = at(head(h));
            let (dx, dy) = (q[0] - o[0], q[1] - o[1]);
            if dy > 0.0 || (dy == 0.0 && dx > 0.0) { 0 } else { 1 }
        };
        let mut err = None;
        list.sort_by(|&x, &y| {
            half(x).cmp(&half(y)).then_with(|| match sign(o, at(head(x)), at(head(y))) {
                Ok(Sign::Positive) => std::cmp::Ordering::Less,
                Ok(Sign::Negative) => std::cmp::Ordering::Greater,
                Ok(Sign::Zero) => x.cmp(&y),
                Err(e) => {
                    err = Some(e);
                    x.cmp(&y)
                }
            })
        });
        if let Some(e) = err {
            return Err(e);
        }
    }
    let pos: Vec<usize> = {
        let mut pos = vec![0; segs.len() * 2];
        for list in &out {
            for (k, &h) in list.iter().enumerate() {
                if let Some(x) = pos.get_mut(h) {
                    *x = k;
                }
            }
        }
        pos
    };
    let next = |h: usize| -> usize {
        let b = head(h);
        let twin = h ^ 1;
        let list = out.get(b).map(Vec::as_slice).unwrap_or(&[]);
        let k = pos.get(twin).copied().unwrap_or(0);
        list.get((k + list.len() - 1) % list.len().max(1)).copied().unwrap_or(twin)
    };
    // Curves: chains between ends (degree ≠ 2 or marked).
    let degree = |p: usize| out.get(p).map_or(0, Vec::len);
    let end = |p: usize| degree(p) != 2 || is_end.get(p).copied().unwrap_or(false);
    let mut curve_of: Vec<(usize, usize, bool)> = vec![(usize::MAX, 0, true); segs.len() * 2];
    let mut curves: Vec<Curve> = Vec::new();
    let mut seen = vec![false; segs.len()];
    let walk = |start: usize, curves: &mut Vec<Curve>, curve_of: &mut Vec<(usize, usize, bool)>, seen: &mut Vec<bool>| {
        let id = curves.len();
        let mut pts = vec![tail(start)];
        let mut tags: Vec<usize> = Vec::new();
        let mut h = start;
        for k in 0..=segs.len() {
            if let Some(s) = seen.get_mut(h / 2) {
                *s = true;
            }
            if let Some(x) = curve_of.get_mut(h) {
                *x = (id, k, true);
            }
            if let Some(x) = curve_of.get_mut(h ^ 1) {
                *x = (id, k, false);
            }
            tags.extend(segs.get(h / 2).map(|s| s.1.as_slice()).unwrap_or(&[]));
            let b = head(h);
            pts.push(b);
            if end(b) || b == tail(start) {
                break;
            }
            // Continue through the other half-edge leaving b.
            match out.get(b).and_then(|l| l.iter().copied().find(|&o| o / 2 != h / 2)) {
                Some(o) if !seen.get(o / 2).copied().unwrap_or(true) => h = o,
                _ => break,
            }
        }
        tags.sort_unstable();
        tags.dedup();
        curves.push(Curve { points: pts, tags });
    };
    for p in 0..n {
        if !end(p) {
            continue;
        }
        for &h in out.get(p).map(Vec::as_slice).unwrap_or(&[]) {
            if !seen.get(h / 2).copied().unwrap_or(true) {
                walk(h, &mut curves, &mut curve_of, &mut seen);
            }
        }
    }
    for s in 0..segs.len() {
        if !seen.get(s).copied().unwrap_or(true) {
            walk(2 * s, &mut curves, &mut curve_of, &mut seen);
        }
    }
    // Face cycles.
    let mut cycle_of = vec![usize::MAX; segs.len() * 2];
    let mut cycles: Vec<Vec<usize>> = Vec::new();
    for h0 in 0..segs.len() * 2 {
        if cycle_of.get(h0).copied() != Some(usize::MAX) {
            continue;
        }
        let id = cycles.len();
        let mut cyc = Vec::new();
        let mut h = h0;
        for _ in 0..=segs.len() * 2 {
            if let Some(c) = cycle_of.get_mut(h) {
                *c = id;
            }
            cyc.push(h);
            h = next(h);
            if h == h0 {
                break;
            }
        }
        if h != h0 {
            return Err(Error::Failed("an arrangement face cycle does not close".into()));
        }
        cycles.push(cyc);
    }
    let area = |cyc: &[usize]| -> f64 {
        0.5 * cyc
            .iter()
            .map(|&h| {
                let (a, b) = (at(tail(h)), at(head(h)));
                a[0] * b[1] - a[1] * b[0]
            })
            .sum::<f64>()
    };
    // Connected components (union-find over points).
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        for _ in 0..parent.len() {
            let p = parent.get(x).copied().unwrap_or(x);
            if p == x {
                break;
            }
            let gp = parent.get(p).copied().unwrap_or(p);
            if let Some(slot) = parent.get_mut(x) {
                *slot = gp;
            }
            x = gp;
        }
        x
    }
    for ((a, b), _) in &segs {
        let (ra, rb) = (find(&mut parent, *a), find(&mut parent, *b));
        if ra != rb
            && let Some(slot) = parent.get_mut(ra.max(rb))
        {
            *slot = ra.min(rb);
        }
    }
    let comp: Vec<usize> = (0..n).map(|p| find(&mut parent, p)).collect();
    let areas: Vec<f64> = cycles.iter().map(|c| area(c)).collect();
    // Is point p inside the polygon of cycle c (crossing number with exact side tests)?
    let inside = |p: Q, cyc: &[usize]| -> Result<bool> {
        let mut wn = 0i64;
        for &h in cyc {
            let (a, b) = (at(tail(h)), at(head(h)));
            if a[1] <= p[1] {
                if b[1] > p[1] && sign(a, b, p)? == Sign::Positive {
                    wn += 1;
                }
            } else if b[1] <= p[1] && sign(a, b, p)? == Sign::Negative {
                wn -= 1;
            }
        }
        Ok(wn != 0)
    };
    let mut holes: Vec<Vec<usize>> = vec![Vec::new(); cycles.len()];
    for (c, cyc) in cycles.iter().enumerate() {
        if areas.get(c).copied().unwrap_or(0.0) >= 0.0 {
            continue;
        }
        let Some(&h) = cyc.first() else { continue };
        let p = tail(h);
        let mut best: Option<(f64, usize)> = None;
        for (o, other) in cycles.iter().enumerate() {
            let a = areas.get(o).copied().unwrap_or(0.0);
            let same = other.first().is_some_and(|&x| comp.get(tail(x)) == comp.get(p));
            if a <= 0.0 || same || best.is_some_and(|(ba, _)| a >= ba) {
                continue;
            }
            if inside(at(p), other)? {
                best = Some((a, o));
            }
        }
        if let Some((_, o)) = best
            && let Some(hs) = holes.get_mut(o)
        {
            hs.push(c);
        }
    }
    // Cycles as loops of curves: one entry per curve use, in cycle order. A use starts at the
    // curve's first segment going forwards, or at its last going backwards.
    let starts_use = |h: usize| match curve_of.get(h) {
        Some(&(c, k, true)) => k == 0 && c != usize::MAX,
        Some(&(c, k, false)) => curves.get(c).is_some_and(|cv| k + 2 == cv.points.len()),
        None => false,
    };
    let as_loop = |cyc: &[usize]| -> Vec<(usize, bool)> {
        let start = cyc.iter().position(|&h| starts_use(h)).unwrap_or(0);
        (0..cyc.len())
            .filter_map(|k| cyc.get((start + k) % cyc.len()).copied())
            .filter(|&h| starts_use(h))
            .filter_map(|h| curve_of.get(h).map(|&(c, _, fwd)| (c, fwd)))
            .collect()
    };
    let mut regions = Vec::new();
    for (c, cyc) in cycles.iter().enumerate() {
        let a = areas.get(c).copied().unwrap_or(0.0);
        if a <= 0.0 {
            continue;
        }
        let mut loops = vec![as_loop(cyc)];
        for &h in holes.get(c).map(Vec::as_slice).unwrap_or(&[]) {
            loops.push(as_loop(cycles.get(h).map(Vec::as_slice).unwrap_or(&[])));
        }
        regions.push(Region { loops });
    }
    Ok(Arrangement { points, curves, regions })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A region's area: its loops' signed areas summed (holes run clockwise).
    fn area(a: &Arrangement, r: &Region) -> f64 {
        r.loops
            .iter()
            .map(|lp| {
                let pts: Vec<Q> = lp
                    .iter()
                    .flat_map(|&(c, fwd)| {
                        let mut v: Vec<usize> = a.curves[c].points.clone();
                        if !fwd {
                            v.reverse();
                        }
                        v.pop();
                        v
                    })
                    .map(|p| a.points[p])
                    .collect();
                0.5 * (0..pts.len()).map(|k| pts[k][0] * pts[(k + 1) % pts.len()][1] - pts[(k + 1) % pts.len()][0] * pts[k][1]).sum::<f64>()
            })
            .sum()
    }

    fn square(x0: f64, y0: f64, s: f64) -> Vec<Q> {
        vec![[x0, y0], [x0 + s, y0], [x0 + s, y0 + s], [x0, y0 + s], [x0, y0]]
    }

    #[test]
    fn square_with_a_hole() {
        let polys = vec![(square(0.0, 0.0, 10.0), 0), (square(4.0, 4.0, 2.0), 1)];
        let verts: Vec<Q> = square(0.0, 0.0, 10.0).into_iter().chain(square(4.0, 4.0, 2.0)).collect();
        let a = build(&verts, &polys, 1e-9, 0.0).unwrap();
        assert_eq!(a.points.len(), 8);
        assert_eq!(a.curves.len(), 8);
        assert_eq!(a.regions.len(), 2);
        let ring = a.regions.iter().find(|r| r.loops.len() == 2).unwrap();
        assert!((area(&a, ring) - 96.0).abs() < 1e-9, "{}", area(&a, ring));
        assert_eq!(ring.loops[0].len(), 4);
        assert_eq!(ring.loops[1].len(), 4);
        let inner = a.regions.iter().find(|r| r.loops.len() == 1).unwrap();
        assert!((area(&a, inner) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn t_junctions_and_crossings_split() {
        // A square cut by a line from the middle of the bottom to the middle of the top, and a
        // line crossing it.
        let polys = vec![(square(0.0, 0.0, 10.0), 0), (vec![[5.0, 0.0], [5.0, 10.0]], 1), (vec![[0.0, 5.0], [10.0, 5.0]], 2)];
        let a = build(&[], &polys, 1e-9, 0.0).unwrap();
        assert_eq!(a.regions.len(), 4);
        for r in &a.regions {
            assert!((area(&a, r) - 25.0).abs() < 1e-9);
            assert_eq!(r.loops.len(), 1);
        }
    }

    #[test]
    fn coincident_copies_merge_and_tags_join() {
        let mut copy = square(0.0, 0.0, 10.0);
        copy[1][1] += 1e-12;
        let polys = vec![(square(0.0, 0.0, 10.0), 0), (copy, 1)];
        let a = build(&[[0.0, 0.0]], &polys, 1e-9, 1e-6).unwrap();
        assert_eq!(a.regions.len(), 1);
        assert_eq!(a.curves.len(), 1);
        assert_eq!(a.curves[0].tags, vec![0, 1]);
    }

    #[test]
    fn hostile_input_is_an_error() {
        assert!(build(&[[f64::NAN, 0.0]], &[], 1e-9, 0.0).is_err());
        assert!(build(&[], &[], 0.0, 0.0).is_err());
        assert!(build(&[], &[(vec![[0.0, 0.0], [1e300, 0.0]], 0)], 1e-9, 0.0).is_err());
    }
}
