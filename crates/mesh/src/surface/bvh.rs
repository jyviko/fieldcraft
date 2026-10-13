//! The self-intersection search: a bounding volume hierarchy of axis-aligned triangle boxes,
//! built top-down by median split on the longest centroid extent, and the self-collision
//! traversal that pairs nodes whose boxes touch (Ericson 2005, ch. 6). Boxes are compared with
//! closed `<=` on the exact coordinates, so touching triangles are always paired; every pair is
//! then decided exactly ([`super::intersect::surface_pair_intersects`]).

use super::intersect::surface_pair_intersects;
use super::{Box3, Defect, Defects, MAX_PAIR_TESTS, Surface};
use crate::{Error, Result};

/// Triangles per leaf.
const LEAF: usize = 4;

/// Twice the centre of a box (no rounding from halving).
fn centre2(b: &Box3) -> [f64; 3] {
    [0, 1, 2].map(|k| b.lo[k] + b.hi[k])
}

#[derive(Debug, Clone, Copy)]
struct Node {
    bounds: Box3,
    /// The left child (the right one follows it), or `None` for a leaf over `order[start..end]`.
    children: Option<usize>,
    start: usize,
    end: usize,
}

struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
}

fn build(boxes: &[Box3]) -> Bvh {
    let mut order: Vec<u32> = (0..boxes.len()).map(super::index).collect();
    let empty = Node { bounds: Box3::EMPTY, children: None, start: 0, end: 0 };
    let mut nodes = vec![empty];
    let mut work = vec![(0usize, 0usize, boxes.len())];
    while let Some((ni, start, end)) = work.pop() {
        let Some(slice) = order.get_mut(start..end) else { continue };
        let mut bounds = Box3::EMPTY;
        let mut centres = Box3::EMPTY;
        for &t in slice.iter() {
            if let Some(b) = boxes.get(t as usize) {
                bounds.add(b);
                centres.add_point(centre2(b));
            }
        }
        let mut node = Node { bounds, children: None, start, end };
        if end - start > LEAF {
            let extent = [0, 1, 2].map(|k| centres.hi[k] - centres.lo[k]);
            let axis = if extent[0] >= extent[1] && extent[0] >= extent[2] {
                0
            } else if extent[1] >= extent[2] {
                1
            } else {
                2
            };
            let key = |t: &u32| boxes.get(*t as usize).map_or(0.0, |b| centre2(b)[axis]);
            let mid = (end - start) / 2;
            slice.select_nth_unstable_by(mid, |a, b| key(a).total_cmp(&key(b)).then(a.cmp(b)));
            let l = nodes.len();
            nodes.push(empty);
            nodes.push(empty);
            node.children = Some(l);
            work.push((l, start, start + mid));
            work.push((l + 1, start + mid, end));
        }
        if let Some(n) = nodes.get_mut(ni) {
            *n = node;
        }
    }
    Bvh { nodes, order }
}

struct Search<'a> {
    surface: &'a Surface<'a>,
    boxes: &'a [Box3],
    tests: u64,
    defects: Defects,
}

impl Search<'_> {
    /// Test one pair; `false` once the defect list is full.
    fn pair(&mut self, s: u32, t: u32) -> Result<bool> {
        let (Some(bs), Some(bt)) = (self.boxes.get(s as usize), self.boxes.get(t as usize)) else { return Ok(true) };
        if s == t || !bs.touches(bt) {
            return Ok(true);
        }
        self.tests += 1;
        if self.tests > MAX_PAIR_TESTS {
            return Err(Error::Limit(format!("self-intersection check: more than {MAX_PAIR_TESTS} triangle pairs to test")));
        }
        let (Some(ts), Some(tt)) = (self.surface.triangles.get(s as usize), self.surface.triangles.get(t as usize)) else { return Ok(true) };
        if surface_pair_intersects([*ts, *tt], self.surface.corners(ts)?, self.surface.corners(tt)?) {
            let (s, t) = (s.min(t) as usize, s.max(t) as usize);
            self.defects
                .push(Defect::SelfIntersection { triangles: [self.surface.source_tri(s), self.surface.source_tri(t)], at: self.surface.centroid(s) });
        }
        Ok(!self.defects.is_full())
    }
}

/// Every pair of triangles that meet outside what they share, up to the listing cap.
pub(crate) fn self_intersections(surface: &Surface) -> Result<Defects> {
    let boxes: Vec<Box3> = surface
        .triangles
        .iter()
        .map(|t| {
            let mut b = Box3::EMPTY;
            for v in t {
                b.add_point(surface.xyz(*v));
            }
            b
        })
        .collect();
    let bvh = build(&boxes);
    let mut search = Search { surface, boxes: &boxes, tests: 0, defects: Defects::new() };
    let leaf = |n: &Node| bvh.order.get(n.start..n.end).unwrap_or(&[]);
    let mut stack = vec![(0usize, 0usize)];
    'search: while let Some((a, b)) = stack.pop() {
        let (Some(na), Some(nb)) = (bvh.nodes.get(a), bvh.nodes.get(b)) else { continue };
        if a == b {
            match na.children {
                Some(l) => stack.extend([(l, l + 1), (l + 1, l + 1), (l, l)]),
                None => {
                    let tris = leaf(na);
                    for (i, &s) in tris.iter().enumerate() {
                        for &t in tris.iter().skip(i + 1) {
                            if !search.pair(s, t)? {
                                break 'search;
                            }
                        }
                    }
                }
            }
            continue;
        }
        if !na.bounds.touches(&nb.bounds) {
            continue;
        }
        match (na.children, nb.children) {
            (None, None) => {
                for &s in leaf(na) {
                    for &t in leaf(nb) {
                        if !search.pair(s, t)? {
                            break 'search;
                        }
                    }
                }
            }
            (Some(l), None) => stack.extend([(l + 1, b), (l, b)]),
            (None, Some(l)) => stack.extend([(a, l + 1), (a, l)]),
            (Some(l), Some(_)) if na.end - na.start >= nb.end - nb.start => stack.extend([(l + 1, b), (l, b)]),
            (Some(_), Some(l)) => stack.extend([(a, l + 1), (a, l)]),
        }
    }
    let mut defects = search.defects;
    defects.listed.sort_by_key(|d| match d {
        Defect::SelfIntersection { triangles, .. } => *triangles,
        _ => [u32::MAX; 2],
    });
    Ok(defects)
}
