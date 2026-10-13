//! The B-rep topology recovered from a face-tagged boundary surface: faces (their triangles,
//! geometry and boundary loops), edges (chains of face-boundary feature edges between the same
//! two faces) and vertices (where three or more faces meet, or where the pair of faces along a
//! chain changes).
//!
//! The hex meshers reason on this topology the way B-rep sweepers do (Blacker, *The Cooper
//! tool*, Proc. 5th International Meshing Roundtable, 1996): which faces are sources, targets and
//! linking faces, and which edges bound them.

use std::collections::BTreeMap;

use super::geom::{Surface, add, dist, scale, unit};
use crate::surface::{BoundarySurface, FeatureKind};
use crate::vec3::{P, cross, dot, norm, sub};
use crate::{Error, Result};

/// A B-rep face.
#[derive(Debug, Clone)]
pub(crate) struct Face {
    /// The B-rep face tag (`tri_face`).
    pub id: u32,
    pub triangles: Vec<u32>,
    /// Surface vertices of the face (sorted).
    pub vertices: Vec<u32>,
    pub area: f64,
    /// Its analytic surface, when the geometry gave one.
    pub surface: Option<Surface>,
    /// When the face is planar: a point and the outward unit normal.
    pub plane: Option<(P, P)>,
    /// Boundary loops: oriented edges `(edge, forward)` with the face on the left.
    pub loops: Vec<Vec<(usize, bool)>>,
}

/// A B-rep edge: a chain of surface vertices along the boundary between two faces.
#[derive(Debug, Clone)]
pub(crate) struct Edge {
    /// In order; a closed edge repeats its first point at the end.
    pub points: Vec<u32>,
    /// Face indices: `faces[0]` traverses the edge forwards (has it on its counter-clockwise
    /// boundary seen from outside), `faces[1]` backwards.
    pub faces: [usize; 2],
}

impl Edge {
    pub fn first(&self) -> u32 {
        self.points.first().copied().unwrap_or(0)
    }
    pub fn last(&self) -> u32 {
        self.points.last().copied().unwrap_or(0)
    }
    pub fn closed(&self) -> bool {
        self.first() == self.last()
    }
    /// The other face of the edge.
    pub fn other(&self, face: usize) -> usize {
        if self.faces[0] == face { self.faces[1] } else { self.faces[0] }
    }
}

pub(crate) struct Brep<'a> {
    pub surface: &'a BoundarySurface,
    pub faces: Vec<Face>,
    pub edges: Vec<Edge>,
    /// Whether each surface vertex is a B-rep vertex.
    pub is_vertex: Vec<bool>,
    /// Length of the bounding-box diagonal.
    pub diagonal: f64,
    /// Geometric tolerance: points closer than this are the same, planes flatter than this are
    /// planar.
    pub eps: f64,
}

/// Relative geometric tolerance (of the bounding-box diagonal).
pub(crate) const RELATIVE_EPS: f64 = 1e-7;

impl<'a> Brep<'a> {
    /// Recover the topology. `surfaces[tag]` is the analytic surface of B-rep face `tag`, if any.
    pub(crate) fn new(surface: &'a BoundarySurface, surfaces: &[Option<Surface>]) -> Result<Self> {
        let pts = surface.points();
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in pts {
            for ((l, h), x) in lo.iter_mut().zip(hi.iter_mut()).zip(p) {
                *l = l.min(*x);
                *h = h.max(*x);
            }
        }
        let diagonal = dist(lo, hi);
        if !(diagonal > 0.0 && diagonal.is_finite()) {
            return Err(Error::Invalid("the boundary surface is empty or degenerate".into()));
        }
        let eps = RELATIVE_EPS * diagonal;
        // Faces in tag order.
        let mut by_tag: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for (t, &f) in surface.tri_face().iter().enumerate() {
            by_tag.entry(f).or_default().push(t as u32);
        }
        let index: BTreeMap<u32, usize> = by_tag.keys().enumerate().map(|(i, &f)| (f, i)).collect();
        let mut faces = Vec::with_capacity(by_tag.len());
        for (&id, tris) in &by_tag {
            let mut verts = Vec::with_capacity(tris.len() * 3);
            let (mut area, mut nsum) = (0.0, [0.0; 3]);
            for &t in tris {
                let tri = surface.triangles().get(t as usize).copied().unwrap_or([0; 3]);
                verts.extend(tri);
                let [a, b, c] = tri.map(|v| pts.get(v as usize).copied().unwrap_or([0.0; 3]));
                let n = cross(sub(b, a), sub(c, a));
                area += 0.5 * norm(n);
                nsum = add(nsum, scale(n, 0.5));
            }
            verts.sort_unstable();
            verts.dedup();
            let analytic = surfaces.get(id as usize).copied().flatten().and_then(|s| s.checked());
            let normal = unit(nsum).unwrap_or([0.0; 3]);
            let plane = planarity(pts, &verts, normal, analytic, eps);
            faces.push(Face { id, triangles: tris.clone(), vertices: verts, area, surface: analytic, plane, loops: Vec::new() });
        }
        let face_of = |t: u32| surface.tri_face().get(t as usize).and_then(|f| index.get(f)).copied();
        // Oriented boundary segments: `from → to` with face `left` traversing it.
        struct Seg {
            from: u32,
            to: u32,
            left: usize,
            right: usize,
        }
        let mut segs = Vec::new();
        for fe in surface.feature_edges().iter().filter(|e| e.kind == FeatureKind::FaceBoundary) {
            let (Some(left), Some(right)) = (face_of(fe.triangles[0]), face_of(fe.triangles[1])) else {
                return Err(Error::Failed("feature edge without faces".into()));
            };
            // Orient each segment as the lower-indexed face traverses it, so that the segments of
            // one edge chain head to tail.
            let [lo, hi] = fe.vertices;
            segs.push(if left < right { Seg { from: lo, to: hi, left, right } } else { Seg { from: hi, to: lo, left: right, right: left } });
        }
        let mut incident: Vec<Vec<usize>> = vec![Vec::new(); pts.len()];
        for (i, s) in segs.iter().enumerate() {
            for v in [s.from, s.to] {
                incident.get_mut(v as usize).ok_or_else(|| Error::Failed("vertex index".into()))?.push(i);
            }
        }
        let pair = |s: &Seg| (s.left, s.right);
        let mut is_vertex = vec![false; pts.len()];
        for (v, inc) in incident.iter().enumerate() {
            let vertex = match inc.as_slice() {
                [] => false,
                [a, b] => match (segs.get(*a), segs.get(*b)) {
                    (Some(a), Some(b)) => {
                        pair(a) != pair(b) || !((a.to == v as u32 && b.from == v as u32) || (b.to == v as u32 && a.from == v as u32))
                    }
                    _ => true,
                },
                _ => true,
            };
            if let Some(x) = is_vertex.get_mut(v) {
                *x = vertex;
            }
        }
        // The segment leaving `v` along the chain that arrived through segment `s`.
        let next = |s: usize, v: u32| -> Option<usize> {
            incident.get(v as usize)?.iter().copied().find(|&o| o != s && segs.get(o).is_some_and(|x| x.from == v))
        };
        let mut visited = vec![false; segs.len()];
        let mut edges = Vec::new();
        for pass in 0..2 {
            for start in 0..segs.len() {
                let Some(s0) = segs.get(start) else { continue };
                if visited.get(start).copied().unwrap_or(true) || (pass == 0 && !is_vertex.get(s0.from as usize).copied().unwrap_or(false)) {
                    continue;
                }
                let mut points = vec![s0.from];
                let mut s = start;
                // A chain has at most one step per segment.
                for _ in 0..=segs.len() {
                    if let Some(x) = visited.get_mut(s) {
                        *x = true;
                    }
                    let Some(seg) = segs.get(s) else { break };
                    points.push(seg.to);
                    if is_vertex.get(seg.to as usize).copied().unwrap_or(true) || seg.to == s0.from {
                        break;
                    }
                    match next(s, seg.to) {
                        Some(n) if !visited.get(n).copied().unwrap_or(true) => s = n,
                        _ => break,
                    }
                }
                edges.push(Edge { points, faces: [s0.left, s0.right] });
            }
        }
        // Loops of each face from its oriented edges.
        let mut uses: Vec<Vec<(usize, bool)>> = vec![Vec::new(); faces.len()];
        for (e, edge) in edges.iter().enumerate() {
            if let Some(u) = uses.get_mut(edge.faces[0]) {
                u.push((e, true));
            }
            if let Some(u) = uses.get_mut(edge.faces[1]) {
                u.push((e, false));
            }
        }
        for (f, u) in uses.iter().enumerate() {
            let ends = |(e, fwd): (usize, bool)| -> (u32, u32) {
                let edge = edges.get(e);
                let (a, b) = (edge.map_or(0, Edge::first), edge.map_or(0, Edge::last));
                if fwd { (a, b) } else { (b, a) }
            };
            let mut used = vec![false; u.len()];
            let mut loops = Vec::new();
            for start in 0..u.len() {
                if used.get(start).copied().unwrap_or(true) {
                    continue;
                }
                let Some(&first) = u.get(start) else { continue };
                let origin = ends(first).0;
                let mut lp = Vec::new();
                let mut cur = start;
                for _ in 0..=u.len() {
                    if let Some(x) = used.get_mut(cur) {
                        *x = true;
                    }
                    let Some(&oe) = u.get(cur) else { break };
                    lp.push(oe);
                    let end = ends(oe).1;
                    if end == origin {
                        break;
                    }
                    match (0..u.len()).find(|&k| !used.get(k).copied().unwrap_or(true) && u.get(k).is_some_and(|&x| ends(x).0 == end)) {
                        Some(k) => cur = k,
                        None => return Err(Error::Failed(format!("face {} has an open boundary loop", faces.get(f).map_or(0, |x| x.id)))),
                    }
                }
                loops.push(lp);
            }
            if let Some(face) = faces.get_mut(f) {
                face.loops = loops;
            }
        }
        Ok(Brep { surface, faces, edges, is_vertex, diagonal, eps })
    }

    pub(crate) fn point(&self, v: u32) -> P {
        self.surface.points().get(v as usize).copied().unwrap_or([0.0; 3])
    }

    /// The points of an oriented edge.
    pub(crate) fn edge_points(&self, e: usize, forward: bool) -> Vec<P> {
        let mut pts: Vec<P> = self.edges.get(e).map(|x| x.points.iter().map(|&v| self.point(v)).collect()).unwrap_or_default();
        if !forward {
            pts.reverse();
        }
        pts
    }

    /// The oriented edge's surface vertices.
    pub(crate) fn edge_vertices(&self, e: usize, forward: bool) -> Vec<u32> {
        let mut v = self.edges.get(e).map(|x| x.points.clone()).unwrap_or_default();
        if !forward {
            v.reverse();
        }
        v
    }

    /// The face's name for messages: its tag.
    pub(crate) fn face_tag(&self, f: usize) -> u32 {
        self.faces.get(f).map_or(u32::MAX, |x| x.id)
    }

    /// The faces that use the triangle's tag (index into `faces`).
    pub(crate) fn face_of_triangle(&self, t: u32) -> Option<usize> {
        let tag = *self.surface.tri_face().get(t as usize)?;
        self.faces.binary_search_by_key(&tag, |f| f.id).ok()
    }
}

/// A point and outward normal of a planar face: the analytic plane, or the plane fitted through
/// the face's vertices when they all lie within `eps` of it.
fn planarity(pts: &[P], verts: &[u32], normal: P, analytic: Option<Surface>, eps: f64) -> Option<(P, P)> {
    let first = pts.get(*verts.first()? as usize).copied()?;
    let (point, n) = match analytic {
        Some(Surface::Plane { point, normal: an }) => (point, if dot(an, normal) < 0.0 { scale(an, -1.0) } else { an }),
        Some(_) => return None,
        None => {
            if norm(normal) == 0.0 {
                return None;
            }
            (first, normal)
        }
    };
    let flat = verts.iter().all(|&v| pts.get(v as usize).is_some_and(|p| dot(sub(*p, point), n).abs() <= eps.max(1e-9 * norm(sub(*p, point)))));
    // An analytic plane is trusted within a looser bound: its vertices lie on it to the kernel's
    // tolerance, not to ours.
    (flat || matches!(analytic, Some(Surface::Plane { .. }))).then_some((point, n))
}
