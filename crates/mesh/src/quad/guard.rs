//! Where the half-resolution triangulation may place Steiner points.
//!
//! The half-resolution region replaces each pair of boundary edges `a–m–b` by the chord `a–b`;
//! the split later puts `m` back as the chord's mid node. The final mesh therefore covers the true
//! region, not the coarse one, and a Steiner point must respect the true boundary: it may not lie
//! in a *cap* (the triangle `a, b, m` when `m` lies inside the coarse region, which the true region
//! excludes), and it keeps a clearance from the true edges `a–m`, `m–b` of at least half the chord
//! (so the elements along the boundary keep the boundary's spacing and can absorb `m`'s offset
//! from the chord). Lookups go through a uniform grid; the cap test is exact (`orient2d`).

use crate::predicates::{Point2, Sign, orient2d};
use crate::{Error, Result};

/// Above this many grid cells a clearance query scans every segment instead.
const MAX_QUERY_CELLS: usize = 1024;

pub(crate) struct Guard {
    lo: [f64; 2],
    step: [f64; 2],
    g: usize,
    /// True boundary edges with their own clearance radius.
    segs: Vec<([f64; 2], [f64; 2], f64)>,
    /// Counterclockwise caps `(a, b, m)`.
    caps: Vec<[Point2; 3]>,
    seg_cells: Vec<Vec<u32>>,
    cap_cells: Vec<Vec<u32>>,
}

fn distance(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    let (d, w) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let len2 = d[0] * d[0] + d[1] * d[1];
    let t = if len2 > 0.0 { ((w[0] * d[0] + w[1] * d[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((w[0] - t * d[0]).powi(2) + (w[1] - t * d[1]).powi(2)).sqrt()
}

impl Guard {
    /// The guard of chords `(a, b, m)`: coarse boundary edge `a → b` (region on its left) and the
    /// skipped boundary point `m`.
    pub fn new(chords: &[(Point2, Point2, Point2)]) -> Result<Guard> {
        let (lo, hi) = super::domain::bbox(chords.iter().flat_map(|(a, b, m)| [a.coords(), b.coords(), m.coords()]));
        if chords.is_empty() {
            return Err(Error::Failed("guard without a boundary".into()));
        }
        let g = ((2.0 * chords.len() as f64).sqrt().ceil() as usize).clamp(1, 1024);
        let step = [((hi[0] - lo[0]) / g as f64).max(f64::MIN_POSITIVE), ((hi[1] - lo[1]) / g as f64).max(f64::MIN_POSITIVE)];
        let mut guard =
            Guard { lo, step, g, segs: Vec::new(), caps: Vec::new(), seg_cells: vec![Vec::new(); g * g], cap_cells: vec![Vec::new(); g * g] };
        for (a, b, m) in chords {
            let (pa, pb, pm) = (a.coords(), b.coords(), m.coords());
            let radius = 0.5 * ((pb[0] - pa[0]).powi(2) + (pb[1] - pa[1]).powi(2)).sqrt();
            for (s, t) in [(pa, pm), (pm, pb)] {
                let id = guard.segs.len() as u32;
                guard.segs.push((s, t, radius));
                let (x0, y0) = guard.cell([s[0].min(t[0]) - radius, s[1].min(t[1]) - radius]);
                let (x1, y1) = guard.cell([s[0].max(t[0]) + radius, s[1].max(t[1]) + radius]);
                guard.register(true, id, (x0, y0), (x1, y1));
            }
            if orient2d(a, b, m) == Sign::Positive {
                let id = guard.caps.len() as u32;
                guard.caps.push([*a, *b, *m]);
                let (x0, y0) = guard.cell([pa[0].min(pb[0]).min(pm[0]), pa[1].min(pb[1]).min(pm[1])]);
                let (x1, y1) = guard.cell([pa[0].max(pb[0]).max(pm[0]), pa[1].max(pb[1]).max(pm[1])]);
                guard.register(false, id, (x0, y0), (x1, y1));
            }
        }
        Ok(guard)
    }

    fn cell(&self, p: [f64; 2]) -> (usize, usize) {
        let f = |x: f64, lo: f64, step: f64| {
            let c = ((x - lo) / step).floor();
            if c.is_nan() || c < 0.0 { 0 } else { (c as usize).min(self.g - 1) }
        };
        (f(p[0], self.lo[0], self.step[0]), f(p[1], self.lo[1], self.step[1]))
    }

    fn register(&mut self, seg: bool, id: u32, from: (usize, usize), to: (usize, usize)) {
        let cells = if seg { &mut self.seg_cells } else { &mut self.cap_cells };
        for y in from.1..=to.1 {
            for x in from.0..=to.0 {
                if let Some(c) = cells.get_mut(y * self.g + x) {
                    c.push(id);
                }
            }
        }
    }

    /// Whether a Steiner point may go at `p`: outside every cap, and farther than both the
    /// segment's own radius and `clearance` from every true boundary edge.
    pub fn allows(&self, p: &Point2, clearance: f64) -> bool {
        let c = p.coords();
        let (cx, cy) = self.cell(c);
        if let Some(caps) = self.cap_cells.get(cy * self.g + cx) {
            for &k in caps {
                if let Some([a, b, m]) = self.caps.get(k as usize)
                    && orient2d(a, b, p) != Sign::Negative
                    && orient2d(b, m, p) != Sign::Negative
                    && orient2d(m, a, p) != Sign::Negative
                {
                    return false;
                }
            }
        }
        let too_close = |&(a, b, r): &([f64; 2], [f64; 2], f64)| distance(a, b, c) < r.max(clearance);
        let (x0, y0) = self.cell([c[0] - clearance, c[1] - clearance]);
        let (x1, y1) = self.cell([c[0] + clearance, c[1] + clearance]);
        if (x1 - x0 + 1) * (y1 - y0 + 1) > MAX_QUERY_CELLS {
            return !self.segs.iter().any(too_close);
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &k in self.seg_cells.get(y * self.g + x).map(Vec::as_slice).unwrap_or(&[]) {
                    if self.segs.get(k as usize).is_some_and(too_close) {
                        return false;
                    }
                }
            }
        }
        true
    }
}
