//! Multi-level sweeping along a straight direction (extrusions) or around an axis (revolutions).
//!
//! **Recognition.** In a sweep frame every point has cross-section coordinates `q` and a sweep
//! coordinate `h` (height along the direction, or angle about the axis). A body is sweepable in
//! the frame when every face is a *cap* (a plane across the sweep: normal along the direction, or
//! a half-plane through the axis) or a *wall* (along the sweep: planes and cylinders parallel to
//! the direction; planes across the axis and coaxial cylinders, cones, spheres and tori), and
//! every edge runs either across the sweep (constant `h`) or along it (constant `q`). The caps
//! and the across edges sit at a finite set of *levels*.
//!
//! **Decomposition.** Projecting into the cross-section every edge that starts a wall upward
//! (the lower outline of each wall) and imprinting them on each other gives a planar arrangement
//! ([`super::arrangement`]) whose regions are the columns of the body: over each region the body
//! is a union of whole level intervals. This is many-to-many sweeping by imprinting all sources
//! and targets into one projection plane (Lai, Benzley, White, IJNME 49(1–2):261–275, 2000;
//! Knupp, *Next-generation sweep tool: a method for generating all-hex meshes on two-and-one-half
//! dimensional geometries*, Proc. 7th International Meshing Roundtable, 1998). Which intervals a
//! region fills is decided by the inside test at its centre ([`super::geom::Inside`]).
//!
//! **Meshing.** Interval assignment ([`super::interval`]) gives every curve its count so that
//! four-sided regions are mapped (opposite sides equal) and the others have even loops for the
//! free all-quad mesher; each level interval gets its own layer count. Each region's quads are
//! stacked through its intervals; shared curves and levels share nodes, so the hexahedra
//! conform. Nodes of curves on analytic walls are projected onto them; in a revolution, the
//! rotated nodes then lie exactly on every surface of revolution.

use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_4, PI, TAU};

use super::arrangement::{self, Arrangement, Q};
use super::brep::{Brep, Face};
use super::geom::{Inside, Surface, add, dist, scale, unit};
use super::interval::Intervals;
use super::region::{self, Scheme};
use super::{Ctx, Raw};
use crate::size::{SizeField, divide_polyline, ideal_divisions};
use crate::vec3::{P, cross, dot, norm, sub};
use crate::{Error, Result};

/// Angular tolerance (radians) of "parallel" and "perpendicular" for the recognition.
const ANGLE_TOL: f64 = 1e-6;
/// The most candidate frames tried per method.
pub(crate) const MAX_CANDIDATES: usize = 6;

/// A sweep frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Frame {
    /// Cross-section `(p·u, p·v)` relative to `o`, height `p·d`; `u × v = d`.
    Translate { o: P, u: P, v: P, d: P },
    /// Cross-section `(z, r)` (along the axis, from the axis), angle about `a` from `e1`
    /// counter-clockwise; `e2 = a × e1`.
    Rotate { o: P, a: P, e1: P, e2: P },
}

fn perpendicular(d: P) -> P {
    let k = if d[0].abs() <= d[1].abs() && d[0].abs() <= d[2].abs() {
        [1.0, 0.0, 0.0]
    } else if d[1].abs() <= d[2].abs() {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    unit(cross(d, k)).unwrap_or([1.0, 0.0, 0.0])
}

impl Frame {
    pub(crate) fn translate(o: P, d: P) -> Option<Frame> {
        let d = unit(d)?;
        let u = perpendicular(d);
        Some(Frame::Translate { o, u, v: cross(d, u), d })
    }

    pub(crate) fn rotate(o: P, a: P) -> Option<Frame> {
        let a = unit(a)?;
        let e1 = perpendicular(a);
        Some(Frame::Rotate { o, a, e1, e2: cross(a, e1) })
    }

    pub(crate) fn coords(&self, p: P) -> (Q, f64) {
        match *self {
            Frame::Translate { o, u, v, d } => {
                let w = sub(p, o);
                ([dot(w, u), dot(w, v)], dot(w, d))
            }
            Frame::Rotate { o, a, e1, e2 } => {
                let w = sub(p, o);
                let z = dot(w, a);
                let (x, y) = (dot(w, e1), dot(w, e2));
                let t = y.atan2(x);
                ([z, x.hypot(y)], if t < 0.0 { t + TAU } else { t })
            }
        }
    }

    pub(crate) fn point(&self, q: Q, h: f64) -> P {
        match *self {
            Frame::Translate { o, u, v, d } => add(o, add(add(scale(u, q[0]), scale(v, q[1])), scale(d, h))),
            Frame::Rotate { o, a, e1, e2 } => add(o, add(scale(a, q[0]), scale(add(scale(e1, h.cos()), scale(e2, h.sin())), q[1]))),
        }
    }

    fn periodic(&self) -> bool {
        matches!(self, Frame::Rotate { .. })
    }

    /// The signed sweep distance from `h0` to `h1` (angles wrapped to (−π, π]).
    fn dh(&self, h1: f64, h0: f64) -> f64 {
        let d = h1 - h0;
        match self {
            Frame::Translate { .. } => d,
            Frame::Rotate { .. } => {
                let w = d.rem_euclid(TAU);
                if w > PI { w - TAU } else { w }
            }
        }
    }

    /// The length a sweep distance `dh` spans at cross-section point `q`.
    fn span(&self, q: Q, dh: f64) -> f64 {
        match self {
            Frame::Translate { .. } => dh.abs(),
            Frame::Rotate { .. } => dh.abs() * q[1].abs(),
        }
    }

    pub(crate) fn describe(&self) -> String {
        let f = |p: P| format!("({:.6}, {:.6}, {:.6})", p[0], p[1], p[2]);
        match *self {
            Frame::Translate { d, .. } => format!("extruded along {}", f(d)),
            Frame::Rotate { o, a, .. } => format!("revolved about the axis through {} along {}", f(o), f(a)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Role {
    Cap,
    Wall,
}

fn parallel(a: P, b: P) -> bool {
    norm(cross(a, b)) <= ANGLE_TOL
}

fn perpendicular_to(a: P, b: P) -> bool {
    dot(a, b).abs() <= ANGLE_TOL
}

/// Distance from `p` to the line through `o` along unit `a`.
fn line_distance(p: P, o: P, a: P) -> f64 {
    let w = sub(p, o);
    norm(sub(w, scale(a, dot(w, a))))
}

/// The face's role in the frame, or why it has none.
fn role(brep: &Brep, face: &Face, frame: &Frame, tol: f64) -> std::result::Result<Role, String> {
    let name = face.id;
    match *frame {
        Frame::Translate { d, .. } => {
            if let Some((_, n)) = face.plane {
                if parallel(n, d) {
                    return Ok(Role::Cap);
                }
                if perpendicular_to(n, d) {
                    return Ok(Role::Wall);
                }
                return Err(format!("planar face {name} is slanted to the direction"));
            }
            match face.surface {
                Some(Surface::Cylinder { axis, .. }) if parallel(axis, d) => Ok(Role::Wall),
                Some(Surface::Cylinder { .. }) => Err(format!("cylindrical face {name} is not parallel to the direction")),
                Some(_) => Err(format!("face {name} is curved across the direction")),
                None => {
                    // A free-form face is a wall when every triangle is parallel to the direction.
                    let ok = face.triangles.iter().all(|&t| {
                        let Some(tri) = brep.surface.triangles().get(t as usize) else { return false };
                        let [a, b, c] = tri.map(|v| brep.point(v));
                        unit(cross(sub(b, a), sub(c, a))).is_some_and(|n| perpendicular_to(n, d))
                    });
                    if ok { Ok(Role::Wall) } else { Err(format!("free-form face {name} is not parallel to the direction")) }
                }
            }
        }
        Frame::Rotate { o, a, .. } => {
            let coaxial = |origin: P, axis: P| parallel(axis, a) && line_distance(origin, o, a) <= tol;
            if let Some((p, n)) = face.plane {
                if perpendicular_to(n, a) && dot(sub(p, o), n).abs() <= tol {
                    return Ok(Role::Cap);
                }
                if parallel(n, a) {
                    return Ok(Role::Wall);
                }
                return Err(format!("planar face {name} neither contains nor crosses the axis"));
            }
            match face.surface {
                Some(Surface::Cylinder { origin, axis, .. })
                | Some(Surface::Cone { apex: origin, axis, .. })
                | Some(Surface::Torus { center: origin, axis, .. })
                    if coaxial(origin, axis) =>
                {
                    Ok(Role::Wall)
                }
                Some(Surface::Sphere { center, .. }) if line_distance(center, o, a) <= tol => Ok(Role::Wall),
                Some(_) => Err(format!("face {name} is not a surface of revolution about the axis")),
                None => Err(format!("free-form face {name}: a revolution needs analytic faces")),
            }
        }
    }
}

/// Candidate extrusion directions: the normals of planar faces and the axes of cylinders, most
/// common first.
pub(crate) fn translations(brep: &Brep) -> Vec<Frame> {
    let mut dirs: Vec<(P, f64)> = Vec::new();
    let mut push = |d: P, weight: f64| {
        let Some(mut d) = unit(d) else { return };
        // A canonical sign: the largest component positive.
        let k = (0..3).max_by(|&i, &j| d[i].abs().total_cmp(&d[j].abs()).then(j.cmp(&i))).unwrap_or(0);
        if d.get(k).copied().unwrap_or(0.0) < 0.0 {
            d = scale(d, -1.0);
        }
        match dirs.iter_mut().find(|(e, _)| parallel(*e, d)) {
            Some(slot) => slot.1 += weight,
            None => dirs.push((d, weight)),
        }
    };
    for f in &brep.faces {
        if let Some((_, n)) = f.plane {
            push(n, f.area);
        } else if let Some(Surface::Cylinder { axis, .. }) = f.surface {
            push(axis, 0.0);
        }
    }
    dirs.sort_by(|a, b| b.1.total_cmp(&a.1));
    let o = brep.point(0);
    dirs.into_iter().filter_map(|(d, _)| Frame::translate(o, d)).collect()
}

/// Candidate axes of revolution: the axes of the analytic surfaces of revolution.
pub(crate) fn rotations(brep: &Brep) -> Vec<Frame> {
    let mut axes: Vec<(P, P)> = Vec::new();
    for f in &brep.faces {
        let (o, a) = match f.surface {
            Some(Surface::Cylinder { origin, axis, .. })
            | Some(Surface::Cone { apex: origin, axis, .. })
            | Some(Surface::Torus { center: origin, axis, .. }) => (origin, axis),
            _ => continue,
        };
        if !axes.iter().any(|(p, b)| parallel(*b, a) && line_distance(o, *p, *b) <= brep.eps * 10.0) {
            axes.push((o, a));
        }
    }
    axes.into_iter().filter_map(|(o, a)| Frame::rotate(o, a)).collect()
}

/// A level of the sweep and the curves projected from it.
struct Setup {
    roles: Vec<Role>,
    /// Sorted levels (heights or angles in `[0, 2π)`).
    levels: Vec<f64>,
}

/// Classify faces and edges and find the levels; an `Unsupported` error says why the body does
/// not sweep in this frame.
fn setup(ctx: &Ctx, frame: &Frame) -> Result<Setup> {
    let brep = ctx.brep;
    let tol = ctx.tol;
    let mut roles = Vec::with_capacity(brep.faces.len());
    for f in &brep.faces {
        roles.push(role(brep, f, frame, tol).map_err(Error::Unsupported)?);
    }
    if frame.periodic() {
        // A revolved region touching the axis would collapse its elements there.
        if let Some(p) = brep.surface.points().iter().find(|&&p| frame.coords(p).0[1] <= tol) {
            return Err(Error::Unsupported(format!("the body reaches the axis of revolution (at {p:?}); revolved hexahedra would collapse there")));
        }
    }
    let r_max = brep.surface.points().iter().map(|&p| frame.coords(p).0[1].abs()).fold(0.0, f64::max);
    let h_tol = match frame {
        Frame::Translate { .. } => tol,
        Frame::Rotate { .. } => tol / r_max.max(tol),
    };
    let mut heights = Vec::new();
    for (e, edge) in brep.edges.iter().enumerate() {
        let pts: Vec<(Q, f64)> = edge.points.iter().map(|&v| frame.coords(brep.point(v))).collect();
        let Some(&(q0, h0)) = pts.first() else { continue };
        let across = pts.iter().all(|&(q, h)| frame.span(q, frame.dh(h, h0)) <= tol);
        let along = pts.iter().all(|&(q, _)| dist([q[0], q[1], 0.0], [q0[0], q0[1], 0.0]) <= tol);
        if across {
            heights.push(h0);
        } else if !along {
            let [f0, f1] = edge.faces.map(|f| brep.face_tag(f));
            return Err(Error::Unsupported(format!("edge {e} between faces {f0} and {f1} runs neither along nor across the sweep")));
        }
    }
    for (f, r) in brep.faces.iter().zip(&roles) {
        if *r == Role::Cap
            && let Some(&v) = f.vertices.first()
        {
            heights.push(frame.coords(brep.point(v)).1);
        }
    }
    heights.sort_by(f64::total_cmp);
    let mut levels: Vec<f64> = Vec::new();
    for h in heights {
        if levels.last().is_none_or(|&l| h - l > h_tol) {
            levels.push(h);
        }
    }
    if frame.periodic()
        && levels.len() > 1
        && let (Some(&first), Some(&last)) = (levels.first(), levels.last())
        && first + TAU - last <= h_tol
    {
        levels.pop();
    }
    let need = if frame.periodic() { 1 } else { 2 };
    if levels.len() < need {
        return Err(Error::Unsupported("no source and target faces across the sweep".into()));
    }
    Ok(Setup { roles, levels })
}

/// A size field on the cross-section plane at sweep coordinate `h`.
struct PlaneField<'a> {
    field: &'a dyn SizeField,
    frame: Frame,
    h: f64,
}

impl SizeField for PlaneField<'_> {
    fn size(&self, p: [f64; 3]) -> f64 {
        self.field.size(self.frame.point([p[0], p[1]], self.h))
    }
}

/// The number of levels the frame sweeps through, if the body sweeps in it (for ordering
/// candidates).
pub(crate) fn levels(ctx: &Ctx, frame: &Frame) -> std::result::Result<usize, String> {
    match setup(ctx, frame) {
        Ok(s) => Ok(s.levels.len()),
        Err(e) => Err(e.to_string()),
    }
}

/// Mesh the body by sweeping in `frame`, with the cross-section's curves `2^refine` times finer
/// than the size asks.
pub(crate) fn mesh(ctx: &Ctx, frame: &Frame, refine: u32) -> Result<Raw> {
    let brep = ctx.brep;
    let tol = ctx.tol;
    let Setup { roles, levels } = setup(ctx, frame)?;
    let h_ref = levels.first().copied().unwrap_or(0.0);
    // Lower outlines of the walls, and every vertex.
    let mut polylines: Vec<(Vec<Q>, usize)> = Vec::new();
    for edge in &brep.edges {
        let pts: Vec<(Q, f64)> = edge.points.iter().map(|&v| frame.coords(brep.point(v))).collect();
        let Some(&(_, h0)) = pts.first() else { continue };
        if !pts.iter().all(|&(q, h)| frame.span(q, frame.dh(h, h0)) <= tol) {
            continue;
        }
        let (Some(&a), Some(&b)) = (edge.points.first(), edge.points.get(1)) else { continue };
        for &f in &edge.faces {
            if roles.get(f) != Some(&Role::Wall) {
                continue;
            }
            // The wall's triangle on the edge's first segment: is its third vertex above?
            let Some(face) = brep.faces.get(f) else { continue };
            let third = face.triangles.iter().find_map(|&t| {
                let tri = brep.surface.triangles().get(t as usize)?;
                (tri.contains(&a) && tri.contains(&b)).then(|| tri.iter().copied().find(|&v| v != a && v != b)).flatten()
            });
            let Some(c) = third else { continue };
            let (qc, hc) = frame.coords(brep.point(c));
            if frame.dh(hc, h0) > 0.0 && frame.span(qc, frame.dh(hc, h0)) > tol {
                polylines.push((pts.iter().map(|x| x.0).collect(), f));
            }
        }
    }
    let vertices: Vec<Q> = brep
        .edges
        .iter()
        .flat_map(|e| [e.first(), e.last()])
        .chain((0..brep.is_vertex.len() as u32).filter(|&v| brep.is_vertex.get(v as usize).copied().unwrap_or(false)))
        .map(|v| frame.coords(brep.point(v)).0)
        .collect();
    let coincide = 2.0 * ctx.opts.chordal_tolerance.max(0.0) + 4.0 * tol;
    // Level intervals.
    let intervals: Vec<(f64, f64)> = if frame.periodic() {
        (0..levels.len())
            .map(|k| {
                let lo = levels.get(k).copied().unwrap_or(0.0);
                let hi = levels.get(k + 1).copied().unwrap_or(levels.first().copied().unwrap_or(0.0) + TAU);
                (lo, hi)
            })
            .collect()
    } else {
        levels.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let mut inside = Inside::new(brep.surface);
    let first = arrangement::build(&vertices, &polylines, tol, coincide)?;
    let (_, used_first) = occupancy(&first, frame, &intervals, &mut inside)?;
    // Cut the filled regions at their reflex corners, then lay the arrangement out again.
    let cut_lines = cuts(&first, &used_first, tol);
    let arr = if cut_lines.is_empty() {
        first
    } else {
        polylines.extend(cut_lines);
        arrangement::build(&vertices, &polylines, tol, coincide)?
    };
    let (filled, used) = occupancy(&arr, frame, &intervals, &mut inside)?;
    // Interval assignment over the curves of filled regions, and the layer counts.
    let mut ia = Intervals::default();
    let plane_field = PlaneField { field: ctx.field, frame: *frame, h: h_ref };
    let curve3d = |c: usize| -> Vec<P> {
        arr.curves
            .get(c)
            .map(|cv| cv.points.iter().map(|&p| frame.point(arr.points.get(p).copied().unwrap_or([0.0; 2]), h_ref)).collect())
            .unwrap_or_default()
    };
    let fine = f64::from(1u32 << refine.min(8));
    for c in 0..arr.curves.len() {
        let goal = ideal_divisions(ctx.field, &curve3d(c))?;
        ia.var(goal * fine, 1);
    }
    let oriented = |c: usize, fwd: bool| -> Vec<Q> {
        let mut v: Vec<Q> =
            arr.curves.get(c).map(|x| x.points.iter().map(|&p| arr.points.get(p).copied().unwrap_or([0.0; 2])).collect()).unwrap_or_default();
        if !fwd {
            v.reverse();
        }
        v
    };
    let mut schemes: Vec<Scheme> = vec![Scheme::Free; arr.regions.len()];
    for (r, region) in arr.regions.iter().enumerate() {
        if !used.get(r).copied().unwrap_or(false) {
            continue;
        }
        let polylines: Vec<Vec<Vec<Q>>> = region.loops.iter().map(|lp| lp.iter().map(|&(c, f)| oriented(c, f)).collect()).collect();
        let scheme = region::classify(&polylines);
        let vars: Vec<Vec<usize>> = region.loops.iter().map(|lp| lp.iter().map(|x| x.0).collect()).collect();
        region::constrain(&mut ia, &vars, scheme);
        if let Some(slot) = schemes.get_mut(r) {
            *slot = scheme;
        }
    }
    let counts = ia.solve()?;
    let r_max = arr.points.iter().map(|q| q[1].abs()).fold(0.0, f64::max);
    let mut layers = Vec::with_capacity(intervals.len());
    for &(lo, hi) in &intervals {
        let path: Vec<P> = match frame {
            Frame::Translate { .. } => vec![frame.point([0.0, 0.0], lo), frame.point([0.0, 0.0], hi)],
            Frame::Rotate { .. } => (0..=16).map(|k| frame.point([0.0, r_max], lo + (hi - lo) * k as f64 / 16.0)).collect(),
        };
        let mut n = ideal_divisions(ctx.field, &path)?.round().max(1.0) as usize;
        if frame.periodic() {
            n = n.max(((hi - lo) / FRAC_PI_4).ceil() as usize);
        }
        layers.push(n);
    }
    // 2D nodes: curve ends, then curve interiors (projected onto analytic walls).
    let mut nodes2: Vec<Q> = Vec::new();
    let mut point_node: HashMap<usize, u32> = HashMap::new();
    let mut curve_nodes: Vec<Vec<u32>> = Vec::with_capacity(arr.curves.len());
    for (c, curve) in arr.curves.iter().enumerate() {
        let n = counts.get(c).copied().unwrap_or(1);
        let pts3 = divide_polyline(ctx.field, &curve3d(c), n)?;
        let wall = curve.tags.iter().find_map(|&f| brep.faces.get(f).and_then(|x| x.surface)).filter(|s| !matches!(s, Surface::Plane { .. }));
        let mut ids = Vec::with_capacity(pts3.len());
        let last = pts3.len().saturating_sub(1);
        for (k, p) in pts3.iter().enumerate() {
            let end = if k == 0 {
                curve.points.first()
            } else if k == last {
                curve.points.last()
            } else {
                None
            };
            let id = match end {
                Some(&pid) => *point_node.entry(pid).or_insert_with(|| {
                    nodes2.push(arr.points.get(pid).copied().unwrap_or([0.0; 2]));
                    nodes2.len() as u32 - 1
                }),
                None => {
                    let p = wall.and_then(|s| s.project(*p)).unwrap_or(*p);
                    nodes2.push(frame.coords(p).0);
                    nodes2.len() as u32 - 1
                }
            };
            ids.push(id);
        }
        curve_nodes.push(ids);
    }
    // Quad meshes of the filled regions.
    let mut region_quads: Vec<Vec<[u32; 4]>> = vec![Vec::new(); arr.regions.len()];
    let quad_opts = &ctx.opts.quad;
    for (r, region) in arr.regions.iter().enumerate() {
        if !used.get(r).copied().unwrap_or(false) {
            continue;
        }
        let mut loops_ids: Vec<Vec<u32>> = Vec::with_capacity(region.loops.len());
        let mut starts: Vec<Vec<usize>> = Vec::new();
        for lp in &region.loops {
            let mut ids = Vec::new();
            let mut st = Vec::new();
            for &(c, fwd) in lp {
                st.push(ids.len());
                let cn = curve_nodes.get(c).map(Vec::as_slice).unwrap_or(&[]);
                let mut seq: Vec<u32> = cn.to_vec();
                if !fwd {
                    seq.reverse();
                }
                seq.pop();
                ids.extend(seq);
            }
            loops_ids.push(ids);
            starts.push(st);
        }
        let at = |i: u32| nodes2.get(i as usize).copied().unwrap_or([0.0; 2]);
        let loops: Vec<Vec<Q>> = loops_ids.iter().map(|l| l.iter().map(|&i| at(i)).collect()).collect();
        let scheme = schemes.get(r).copied().unwrap_or(Scheme::Free);
        let (mesh, used_scheme) = region::mesh(&loops, &starts, scheme, &plane_field, quad_opts).map_err(|e| region_error(r, e))?;
        if let Some(slot) = schemes.get_mut(r) {
            *slot = used_scheme;
        }
        let boundary: Vec<u32> = loops_ids.concat();
        let mut local: Vec<u32> = boundary.clone();
        for p in mesh.nodes.iter().skip(mesh.boundary_nodes) {
            nodes2.push(*p);
            local.push(nodes2.len() as u32 - 1);
        }
        let quads = region_quads.get_mut(r).ok_or_else(|| Error::Failed("region index".into()))?;
        for q in &mesh.quads {
            quads.push(q.map(|i| local.get(i as usize).copied().unwrap_or(0)));
        }
    }
    // Layer coordinates.
    let mut heights: Vec<f64> = Vec::new();
    let mut first_layer: Vec<usize> = Vec::with_capacity(intervals.len());
    for (k, &(lo, hi)) in intervals.iter().enumerate() {
        first_layer.push(heights.len());
        let n = layers.get(k).copied().unwrap_or(1);
        for j in 0..n {
            heights.push(lo + (hi - lo) * j as f64 / n as f64);
        }
    }
    if !frame.periodic()
        && let Some(&(_, hi)) = intervals.last()
    {
        heights.push(hi);
    }
    let total_layers = heights.len();
    // Element budget.
    let mut count = 0usize;
    for (r, quads) in region_quads.iter().enumerate() {
        for (k, f) in filled.get(r).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
            if *f {
                count = count.saturating_add(quads.len().saturating_mul(layers.get(k).copied().unwrap_or(0)));
            }
        }
    }
    if count > ctx.opts.max_elements {
        return Err(Error::Limit(format!("the mesh would have {count} hexahedra; the limit is {}", ctx.opts.max_elements)));
    }
    // Hexahedra, with nodes made as first used.
    let mut nodes: Vec<P> = Vec::new();
    let mut node_at: HashMap<(u32, usize), u32> = HashMap::new();
    let mut node = |n2: u32, layer: usize, nodes: &mut Vec<P>| -> u32 {
        let layer = if frame.periodic() { layer % total_layers.max(1) } else { layer };
        *node_at.entry((n2, layer)).or_insert_with(|| {
            let q = nodes2.get(n2 as usize).copied().unwrap_or([0.0; 2]);
            nodes.push(frame.point(q, heights.get(layer).copied().unwrap_or(0.0)));
            nodes.len() as u32 - 1
        })
    };
    let mut hexes: Vec<[u32; 8]> = Vec::with_capacity(count);
    for (r, quads) in region_quads.iter().enumerate() {
        for (k, f) in filled.get(r).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
            if !*f {
                continue;
            }
            let (s, n) = (first_layer.get(k).copied().unwrap_or(0), layers.get(k).copied().unwrap_or(0));
            for q in quads {
                for j in s..s + n {
                    let b = q.map(|v| node(v, j, &mut nodes));
                    let t = q.map(|v| node(v, j + 1, &mut nodes));
                    hexes.push([b[0], b[1], b[2], b[3], t[0], t[1], t[2], t[3]]);
                }
            }
        }
    }
    let filled_regions = used.iter().filter(|&&u| u).count();
    let structured = schemes.iter().zip(&used).filter(|(s, u)| **u && **s != Scheme::Free).count();
    let description = format!(
        "{}: {} level{}, {} region{} ({} structured, {} free), {} layer{}",
        frame.describe(),
        levels.len(),
        if levels.len() == 1 { "" } else { "s" },
        filled_regions,
        if filled_regions == 1 { "" } else { "s" },
        structured,
        filled_regions - structured,
        total_layers.saturating_sub(usize::from(!frame.periodic())),
        if total_layers == 2 { "" } else { "s" },
    );
    Ok(Raw { nodes, hexes, description })
}

/// Which level intervals each region of the arrangement fills (inside test at the region's
/// centre, mid-interval), and which regions fill any.
fn occupancy(arr: &Arrangement, frame: &Frame, intervals: &[(f64, f64)], inside: &mut Inside) -> Result<(Vec<Vec<bool>>, Vec<bool>)> {
    if arr.regions.is_empty() {
        return Err(Error::Unsupported("the projected outline encloses no region".into()));
    }
    let mut filled: Vec<Vec<bool>> = Vec::with_capacity(arr.regions.len());
    for region in &arr.regions {
        let c = interior_point(arr, region)?;
        let mut row = Vec::with_capacity(intervals.len());
        for &(lo, hi) in intervals {
            row.push(inside.contains(frame.point(c, 0.5 * (lo + hi)))?);
        }
        filled.push(row);
    }
    let used: Vec<bool> = filled.iter().map(|r| r.iter().any(|&x| x)).collect();
    if !used.iter().any(|&u| u) {
        return Err(Error::Unsupported("no region of the cross-section lies inside the body".into()));
    }
    Ok((filled, used))
}

/// The tag of cut lines (no wall behind them).
const CUT: usize = usize::MAX;

/// Straight cuts from the reflex corners of the filled regions into them, each along one of the
/// corner's edges (the shorter way) to the first boundary it meets. A rectilinear region falls
/// into rectangles this way; others lose their reflex corners. This is the explicit form of the
/// virtual decomposition behind submapping (Whiteley, White, Benzley, Blacker 1996), done in the
/// arrangement so that interval assignment sees every piece.
fn cuts(arr: &Arrangement, used: &[bool], tol: f64) -> Vec<(Vec<Q>, usize)> {
    let at = |p: usize| arr.points.get(p).copied().unwrap_or([0.0; 2]);
    let segments: Vec<(usize, usize)> = arr.curves.iter().flat_map(|c| c.points.windows(2).map(|w| (w[0], w[1]))).collect();
    // The nearest boundary hit of the ray from point v along unit d (ignoring segments at v).
    let hit = |v: usize, d: Q| -> Option<(f64, Q, Option<usize>)> {
        let o = at(v);
        let mut best: Option<(f64, Q, Option<usize>)> = None;
        for &(a, b) in &segments {
            if a == v || b == v {
                continue;
            }
            let (pa, pb) = (at(a), at(b));
            let e = [pb[0] - pa[0], pb[1] - pa[1]];
            let den = d[0] * e[1] - d[1] * e[0];
            if den.abs() <= f64::EPSILON * (e[0].abs() + e[1].abs()) {
                continue;
            }
            let w = [pa[0] - o[0], pa[1] - o[1]];
            let t = (w[0] * e[1] - w[1] * e[0]) / den;
            let u = (w[0] * d[1] - w[1] * d[0]) / den;
            if t > tol && (-1e-12..=1.0 + 1e-12).contains(&u) && best.is_none_or(|(bt, _, _)| t < bt) {
                let x = [o[0] + t * d[0], o[1] + t * d[1]];
                // End at a segment end that is this close to the hit.
                let near = [(a, pa), (b, pb)].into_iter().find(|(_, p)| ((p[0] - x[0]).powi(2) + (p[1] - x[1]).powi(2)).sqrt() <= 0.1 * t);
                best = Some((t, near.map_or(x, |n| n.1), near.map(|n| n.0)));
            }
        }
        best
    };
    let mut out: Vec<(Vec<Q>, usize)> = Vec::new();
    let mut seen: Vec<(usize, usize)> = Vec::new();
    for (r, region) in arr.regions.iter().enumerate() {
        if !used.get(r).copied().unwrap_or(false) {
            continue;
        }
        for lp in &region.loops {
            let n = lp.len();
            for k in 0..n {
                let (Some(&(pc, pf)), Some(&(nc, nf))) = (lp.get((k + n - 1) % n), lp.get(k)) else { continue };
                let pts = |c: usize, f: bool| {
                    let mut v = arr.curves.get(c).map(|x| x.points.clone()).unwrap_or_default();
                    if !f {
                        v.reverse();
                    }
                    v
                };
                let (prev, next) = (pts(pc, pf), pts(nc, nf));
                let (Some(&a), Some(&v), Some(&b)) = (prev.get(prev.len().wrapping_sub(2)), next.first(), next.get(1)) else { continue };
                let (pa, pv, pb) = (at(a), at(v), at(b));
                let (u, w) = ([pv[0] - pa[0], pv[1] - pa[1]], [pb[0] - pv[0], pb[1] - pv[1]]);
                let turn = (u[0] * w[1] - u[1] * w[0]).atan2(u[0] * w[0] + u[1] * w[1]);
                // The region is on the left: a right turn of more than 45° is a reflex corner.
                if PI - turn < 1.25 * PI {
                    continue;
                }
                let norm2 = |x: Q| (x[0] * x[0] + x[1] * x[1]).sqrt();
                let (lu, lw) = (norm2(u), norm2(w));
                if !(lu > 0.0 && lw > 0.0) {
                    continue;
                }
                let rays = [[u[0] / lu, u[1] / lu], [-w[0] / lw, -w[1] / lw]];
                let best = rays.iter().filter_map(|&d| hit(v, d)).min_by(|x, y| x.0.total_cmp(&y.0));
                if let Some((_, end, end_point)) = best {
                    if let Some(e) = end_point {
                        let key = (v.min(e), v.max(e));
                        if seen.contains(&key) {
                            continue;
                        }
                        seen.push(key);
                    }
                    out.push((vec![pv, end], CUT));
                }
            }
        }
    }
    out
}

fn region_error(r: usize, e: Error) -> Error {
    match e {
        Error::Limit(m) => Error::Limit(m),
        other => Error::Failed(format!("cross-section region {r}: {other}")),
    }
}

/// A point well inside a region: off the middle of one of its outer edges, inside the outer
/// loop, outside the holes and clear of every boundary segment.
fn interior_point(arr: &Arrangement, region: &arrangement::Region) -> Result<Q> {
    let at = |p: usize| arr.points.get(p).copied().unwrap_or([0.0; 2]);
    let polygon = |lp: &[(usize, bool)]| -> Vec<Q> {
        let mut out = Vec::new();
        for &(c, fwd) in lp {
            let mut v: Vec<usize> = arr.curves.get(c).map(|x| x.points.clone()).unwrap_or_default();
            if !fwd {
                v.reverse();
            }
            v.pop();
            out.extend(v.into_iter().map(at));
        }
        out
    };
    let loops: Vec<Vec<Q>> = region.loops.iter().map(|l| polygon(l)).collect();
    let winding = |p: Q, poly: &[Q]| -> i64 {
        let mut wn = 0;
        for k in 0..poly.len() {
            let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
            let side = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
            if a[1] <= p[1] {
                if b[1] > p[1] && side > 0.0 {
                    wn += 1;
                }
            } else if b[1] <= p[1] && side < 0.0 {
                wn -= 1;
            }
        }
        wn
    };
    let clearance = |p: Q| -> f64 {
        loops
            .iter()
            .flat_map(|poly| (0..poly.len()).map(move |k| (poly[k], poly[(k + 1) % poly.len()])))
            .map(|(a, b)| {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let l2 = dx * dx + dy * dy;
                let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
            })
            .fold(f64::INFINITY, f64::min)
    };
    let outer = loops.first().ok_or_else(|| Error::Failed("region without a loop".into()))?;
    let mut edges: Vec<(f64, usize)> = (0..outer.len()).map(|k| (dist2(outer[k], outer[(k + 1) % outer.len()]), k)).collect();
    edges.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut best: Option<(f64, Q)> = None;
    for &(len2, k) in edges.iter().take(64) {
        let (a, b) = (outer[k], outer[(k + 1) % outer.len()]);
        let len = len2.sqrt();
        let (m, n) = ([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5], [-(b[1] - a[1]) / len, (b[0] - a[0]) / len]);
        for f in [0.5, 0.25, 0.1, 0.03, 0.01, 1e-3] {
            let p = [m[0] + n[0] * f * len, m[1] + n[1] * f * len];
            if winding(p, outer) == 0 || loops.iter().skip(1).any(|h| winding(p, h) != 0) {
                continue;
            }
            let c = clearance(p);
            if best.is_none_or(|(bc, _)| c > bc) {
                best = Some((c, p));
            }
        }
    }
    best.filter(|(c, _)| *c > 0.0).map(|(_, p)| p).ok_or_else(|| Error::Failed("no point found inside a cross-section region".into()))
}

fn dist2(a: Q, b: Q) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}
