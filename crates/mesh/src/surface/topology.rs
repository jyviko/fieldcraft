//! Edges, manifoldness, orientation consistency, shells, nesting and feature edges.
//!
//! Edges come from sorting half-edges (no hash order); shells are numbered by their first
//! triangle; every loop is bounded by the input size.

use std::f64::consts::PI;

use super::{Box3, Defect, Defects, FeatureEdge, FeatureKind, MAX_WINDING_WORK, Shell, Surface, index};
use crate::vec3::{angle_deg, det, dot, norm, sub};
use crate::{Error, Result};

/// No triangle (only while building).
const NONE: u32 = u32::MAX;

/// An interior edge: its vertices (ascending) and its two triangles, the one traversing it from
/// `v[0]` to `v[1]` first.
pub(crate) struct Edge {
    pub v: [u32; 2],
    pub t: [u32; 2],
}

/// One triangle's use of an edge.
struct Half {
    lo: u32,
    hi: u32,
    /// Traversed from `lo` to `hi`.
    forward: bool,
    tri: u32,
    corner: usize,
}

pub(crate) struct Topology {
    /// Per triangle `[a, b, c]`: the triangles across `ab`, `bc`, `ca`.
    pub neighbors: Vec<[u32; 3]>,
    pub edges: Vec<Edge>,
}

impl Topology {
    /// Pair every edge with exactly one oppositely oriented edge and check every vertex is a
    /// single fan.
    pub fn build(s: &Surface) -> Result<Topology> {
        let mut half: Vec<Half> = Vec::with_capacity(s.triangles.len() * 3);
        for (t, tri) in s.triangles.iter().enumerate() {
            for (k, (a, b)) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])].into_iter().enumerate() {
                half.push(Half { lo: a.min(b), hi: a.max(b), forward: a < b, tri: index(t), corner: k });
            }
        }
        half.sort_unstable_by_key(|h| (h.lo, h.hi, h.tri, h.corner));

        let mut neighbors = vec![[NONE; 3]; s.triangles.len()];
        let mut edges = Vec::with_capacity(half.len() / 2);
        let mut defects = Defects::new();
        for run in half.chunk_by(|x, y| (x.lo, x.hi) == (y.lo, y.hi)) {
            let Some(&Half { lo, hi, tri: t0, .. }) = run.first() else { continue };
            let vertices = || [s.source_vert(lo), s.source_vert(hi)];
            let at = || s.midpoint(lo, hi);
            match run {
                [_] => defects.push(Defect::OpenEdge { vertices: vertices(), triangle: s.source_tri(t0 as usize), at: at() }),
                [x, y] if x.forward == y.forward => defects.push(Defect::Flipped {
                    vertices: vertices(),
                    triangles: [s.source_tri(x.tri as usize), s.source_tri(y.tri as usize)],
                    at: at(),
                }),
                [x, y] => {
                    for (from, to) in [(x, y), (y, x)] {
                        if let Some(slot) = neighbors.get_mut(from.tri as usize).and_then(|n| n.get_mut(from.corner)) {
                            *slot = to.tri;
                        }
                    }
                    let t = if x.forward { [x.tri, y.tri] } else { [y.tri, x.tri] };
                    edges.push(Edge { v: [lo, hi], t });
                }
                _ => defects.push(Defect::NonManifoldEdge { vertices: vertices(), triangles: index(run.len()), at: at() }),
            }
        }
        defects.into_result()?;

        let topo = Topology { neighbors, edges };
        topo.check_fans(s)?;
        Ok(topo)
    }

    /// Every vertex's triangles form one fan: walking across the edges around it from any of its
    /// triangles visits all of them.
    fn check_fans(&self, s: &Surface) -> Result<()> {
        let mut degree = vec![0u32; s.pts.len()];
        for v in s.triangles.iter().flatten() {
            if let Some(d) = degree.get_mut(*v as usize) {
                *d += 1;
            }
        }
        let mut seen = vec![false; s.pts.len()];
        let mut defects = Defects::new();
        for (t0, tri) in s.triangles.iter().enumerate() {
            for &v in tri {
                let Some(flag) = seen.get_mut(v as usize) else { continue };
                if *flag {
                    continue;
                }
                *flag = true;
                let deg = degree.get(v as usize).copied().unwrap_or(0);
                // Rotate across the edge leaving `v` in each triangle until back at the start.
                let (mut t, mut steps) = (index(t0), 0u32);
                loop {
                    let corner = s.triangles.get(t as usize).and_then(|tr| tr.iter().position(|&w| w == v));
                    let next = corner.and_then(|k| self.neighbors.get(t as usize).and_then(|n| n.get(k))).copied().unwrap_or(NONE);
                    steps += 1;
                    if next == NONE || next == index(t0) || steps > deg {
                        break;
                    }
                    t = next;
                }
                if steps != deg {
                    defects.push(Defect::NonManifoldVertex { vertex: s.source_vert(v), at: s.xyz(v) });
                }
            }
        }
        defects.into_result()
    }
}

pub(crate) struct Shells {
    pub tri_shell: Vec<u32>,
    pub shells: Vec<Shell>,
}

/// Connected components, their genus, signed volume and nesting; fails when a shell faces the
/// wrong way.
pub(crate) fn shells(s: &Surface, topo: &Topology) -> Result<Shells> {
    let n = s.triangles.len();
    let mut tri_shell = vec![NONE; n];
    let mut shells = Vec::new();
    // Per shell: its box and its reference point (first vertex of its first triangle).
    let mut extent: Vec<(Box3, [f64; 3])> = Vec::new();
    let mut vertex_shell = vec![NONE; s.pts.len()];
    let mut stack = Vec::new();
    for start in 0..n {
        if tri_shell.get(start) != Some(&NONE) {
            continue;
        }
        let id = index(shells.len());
        let origin = s.triangles.get(start).map_or([0.0; 3], |t| s.xyz(t[0]));
        let (mut faces, mut verts, mut six_v) = (0u32, 0u32, 0.0);
        let mut bounds = Box3::EMPTY;
        if let Some(x) = tri_shell.get_mut(start) {
            *x = id;
        }
        stack.push(start);
        while let Some(t) = stack.pop() {
            faces += 1;
            let tri = s.triangles.get(t).copied().unwrap_or([0; 3]);
            for v in tri {
                if let Some(vs) = vertex_shell.get_mut(v as usize)
                    && *vs != id
                {
                    *vs = id;
                    verts += 1;
                    bounds.add_point(s.xyz(v));
                }
            }
            let [a, b, c] = s.tri_xyz(t).map(|p| sub(p, origin));
            six_v += det(a, b, c);
            for &m in topo.neighbors.get(t).into_iter().flatten() {
                if let Some(x) = tri_shell.get_mut(m as usize)
                    && *x == NONE
                {
                    *x = id;
                    stack.push(m as usize);
                }
            }
        }
        // Closed 2-manifold: E = 3F/2, χ = V − E + F = 2 − 2g.
        let chi = i64::from(verts) - i64::from(faces) * 3 / 2 + i64::from(faces);
        let genus = u32::try_from((2 - chi).max(0) / 2).unwrap_or(u32::MAX);
        shells.push(Shell { first_triangle: index(start), triangles: faces, vertices: verts, genus, volume: six_v / 6.0, void: false });
        extent.push((bounds, origin));
    }

    nest(s, &tri_shell, &mut shells, &extent)?;

    let mut defects = Defects::new();
    for (id, sh) in shells.iter().enumerate() {
        let d = extent.get(id).map_or(0.0, |(b, _)| b.diagonal());
        let triangle = s.source_tri(sh.first_triangle as usize);
        // Each term of the sum is at most d³ and carries a few ulps of rounding.
        if !(sh.volume.abs() > 8.0 * f64::EPSILON * f64::from(sh.triangles) * d * d * d) {
            defects.push(Defect::Flat { shell: index(id), triangle });
        } else if (sh.volume > 0.0) == sh.void {
            defects.push(Defect::Inverted { shell: index(id), triangle, volume: sh.volume });
        }
    }
    defects.into_result()?;
    Ok(Shells { tri_shell, shells })
}

/// Mark the voids: shells inside an odd number of other shells, by the generalized winding
/// number of one of their vertices (shells do not intersect, so it is ±1 inside and 0 outside,
/// up to rounding).
fn nest(s: &Surface, tri_shell: &[u32], shells: &mut [Shell], extent: &[(Box3, [f64; 3])]) -> Result<()> {
    if shells.len() < 2 {
        return Ok(());
    }
    // Triangles of each shell, in order.
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); shells.len()];
    for (t, &sh) in tri_shell.iter().enumerate() {
        if let Some(m) = members.get_mut(sh as usize) {
            m.push(t);
        }
    }
    let mut work = 0u64;
    for (i, &(_, x)) in extent.iter().enumerate() {
        let mut depth = 0u32;
        for (j, (bounds, _)) in extent.iter().enumerate() {
            if j == i || !bounds.contains(x) {
                continue;
            }
            let tris = members.get(j).map_or(&[][..], Vec::as_slice);
            work += tris.len() as u64;
            if work > MAX_WINDING_WORK {
                return Err(Error::Limit(format!("shell nesting: more than {MAX_WINDING_WORK} triangle solid angles")));
            }
            let omega: f64 = tris.iter().map(|&t| solid_angle(s, t, x)).sum();
            if (omega / (4.0 * PI)).abs() > 0.5 {
                depth += 1;
            }
        }
        if let Some(sh) = shells.get_mut(i) {
            sh.void = depth % 2 == 1;
        }
    }
    Ok(())
}

/// The signed solid angle triangle `t` subtends at `x` (van Oosterom and Strackee 1983).
fn solid_angle(s: &Surface, t: usize, x: [f64; 3]) -> f64 {
    let [a, b, c] = s.tri_xyz(t).map(|p| sub(p, x));
    let (la, lb, lc) = (norm(a), norm(b), norm(c));
    let den = la * lb * lc + dot(a, b) * lc + dot(a, c) * lb + dot(b, c) * la;
    2.0 * det(a, b, c).atan2(den)
}

/// Edges between different B-rep faces, and edges inside a face whose normals turn by more than
/// `feature_angle` degrees.
pub(crate) fn feature_edges(s: &Surface, topo: &Topology, tri_face: &[u32], feature_angle: f64) -> Vec<FeatureEdge> {
    let face = |t: u32| tri_face.get(t as usize).copied().unwrap_or(0);
    let normals: Vec<[f64; 3]> = (0..s.triangles.len()).map(|t| s.normal(t)).collect();
    let normal = |t: u32| normals.get(t as usize).copied().unwrap_or([0.0; 3]);
    topo.edges
        .iter()
        .filter_map(|e| {
            let angle = angle_deg(normal(e.t[0]), normal(e.t[1]));
            let kind = if face(e.t[0]) != face(e.t[1]) {
                FeatureKind::FaceBoundary
            } else if angle > feature_angle {
                FeatureKind::Sharp
            } else {
                return None;
            };
            Some(FeatureEdge { vertices: e.v, triangles: e.t, angle, kind })
        })
        .collect()
}
