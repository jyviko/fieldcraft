//! Symbols for a model's analysis setup, drawn over the model by the desktop viewport and the
//! headless renderer alike, so both show the same thing.
//!
//! - **Boundary conditions:** a cone per constrained translation, tip on the node, pointing along
//!   the axis; a second, smaller cone behind it for a constrained rotation (only where the node
//!   can rotate: shell and beam nodes, reference points). Thermal conditions are dots.
//! - **Forces:** straight arrows ending on their nodes, along the force. **Moments:** arrows with
//!   two heads, starting at the node.
//! - **Pressures:** arrows normal to the loaded faces (onto them for a positive pressure, away for
//!   a suction); **tractions:** arrows onto the faces along the traction. 2D models get them on
//!   the loaded edges, in the plane.
//! - **Gravity, body forces and centrifugal loads:** one arrow beside the model (a centrifugal
//!   load: its axis and a curved arrow around it). **Initial velocities:** one arrow behind the
//!   moving set, along its motion.
//! - **Reference points:** a three-axis cross with the point's name; **couplings:** thin lines
//!   from the point to some of the coupled nodes; **rigid walls:** a hatched square.
//!
//! Sizes follow the model: one symbol unit is 2.5 % of its bounding box diagonal.
//! Arrow shafts and outlines are lines of a fixed width in pixels, so they stay visible at any
//! zoom. Symbols closer than about a symbol apart are thinned on a grid, and each definition
//! draws at most [`MAX_PER_DEFINITION`] symbols ([`MAX_SYMBOLS`] in all): a definition with more
//! places than that is drawn with a sample and listed in [`Glyphs::notes`], so the viewer can
//! say so.
//!
//! [`Glyphs::from_model`] does the work that depends on the model only (resolving sets, picking
//! the symbols' nodes); [`Glyphs::draw`] places them at the nodes' current positions (a results
//! plot's deformed shape) in the given colours.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fieldcraft_model::{BoundaryCondition, ElementFace, Load, Model, Procedure, ResolvedSet, Target};

use crate::display::{centre, polygon_normal};
use crate::{Aabb3, Rgb, Scene, SceneLabel, SceneLine, ScenePoint, SceneTri, Vec3};

/// One symbol unit as a fraction of the model's bounding box diagonal.
const UNIT_FRACTION: f64 = 0.025;
/// Most symbols one definition draws.
pub const MAX_PER_DEFINITION: usize = 400;
/// Most symbols drawn in all.
pub const MAX_SYMBOLS: usize = 6000;
/// Most lines one coupling draws to its nodes.
const MAX_COUPLING_LINES: usize = 24;
/// How far symbols over a results plot fade toward the background ([`GlyphColors::over_results`]).
pub const MUTED: f64 = 0.55;
/// Most nodes or faces of one set looked at when choosing where symbols go (a larger set is
/// strided down to this first).
const MAX_SCAN: usize = 200_000;
/// Segments around a cone.
const CONE_SEGMENTS: usize = 10;
/// Segments of a centrifugal load's curved arrow.
const ARC_SEGMENTS: usize = 24;

/// Arrow shaft, outline and coupling line widths, in output pixels.
const SHAFT_WIDTH: f64 = 2.5;
const OUTLINE_WIDTH: f64 = 1.5;
const COUPLING_WIDTH: f64 = 1.0;
/// Reference point and thermal dot sizes, in output pixels.
const POINT_SIZE: f64 = 8.0;
const THERMAL_SIZE: f64 = 6.0;

/// The colours symbols are drawn in, by role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphColors {
    /// Boundary conditions and rigid walls.
    pub constraint: Rgb,
    /// Forces, moments, pressures, body loads and initial velocities.
    pub load: Rgb,
    /// Reference points and couplings.
    pub interaction: Rgb,
    /// Temperatures, heat fluxes, convection and heat sources.
    pub thermal: Rgb,
    /// Reference point names.
    pub label: Rgb,
    /// Datum planes and axes.
    pub datum: Rgb,
}

impl GlyphColors {
    /// The colours moved a fraction `k` (0..=1) of the way to `toward`: symbols that stay out
    /// of the way (over a results plot).
    pub fn muted(self, toward: Rgb, k: f64) -> GlyphColors {
        let k = if k.is_finite() { k.clamp(0.0, 1.0) } else { 0.0 };
        let mix = |c: Rgb| {
            let m = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * k).round().clamp(0.0, 255.0) as u8;
            Rgb(m(c.0, toward.0), m(c.1, toward.1), m(c.2, toward.2))
        };
        GlyphColors {
            constraint: mix(self.constraint),
            load: mix(self.load),
            interaction: mix(self.interaction),
            thermal: mix(self.thermal),
            label: mix(self.label),
            datum: mix(self.datum),
        }
    }

    /// The colours over a results plot drawn on `background`: [`MUTED`] of the way to it.
    pub fn over_results(self, background: Rgb) -> GlyphColors {
        self.muted(background, MUTED)
    }
}

/// Which groups of symbols are drawn (the browser's eye toggles).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphShow {
    /// Boundary conditions, loads and initial velocities.
    pub loads: bool,
    /// Reference points, couplings and rigid walls.
    pub interactions: bool,
}

impl GlyphShow {
    pub const ALL: GlyphShow = GlyphShow { loads: true, interactions: true };
}

/// Where a symbol sits: a mesh node, or a reference point (its own node label when a result
/// moves it, else its coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Anchor {
    Node(u32),
    Point { label: u32, xyz: Vec3 },
}

/// The corners of a loaded face (or the ends of a loaded edge), with the outward direction it
/// had on the undeformed model (the solver's convention: a face's right-hand normal, outward
/// for solids; an in-plane edge's normal to the right of its direction).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Face {
    corners: [u32; 4],
    count: usize,
    outward: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum FaceLoad {
    /// A pressure: onto the face (positive) or away from it.
    Pressure { onto: bool },
    /// A traction along a unit direction.
    Traction(Vec3),
}

#[derive(Clone, Debug, PartialEq)]
enum Mark {
    /// Constrained translations (bits 0..3) and rotations (bits 3..6).
    Constraint {
        at: Anchor,
        dofs: u8,
    },
    /// A force (ending on the node) or moment (starting there), `len` units long.
    Arrow {
        at: Anchor,
        dir: Vec3,
        len: f64,
        moment: bool,
    },
    FaceArrow {
        face: Face,
        load: FaceLoad,
    },
    Thermal {
        at: Anchor,
    },
    /// A fixed arrow (gravity, body force, initial velocity) from `tail` along `dir`.
    Free {
        tail: Vec3,
        dir: Vec3,
        len: f64,
    },
    /// A centrifugal load: its axis through `centre` and a curved arrow around it.
    Spin {
        centre: Vec3,
        axis: Vec3,
        half_length: f64,
    },
    ReferencePoint {
        at: Anchor,
        name: String,
    },
    CouplingLine {
        from: Anchor,
        to: u32,
    },
    /// A rigid wall: a square through `point`, `half` units across, hatched on the side away
    /// from `normal`.
    Wall {
        point: Vec3,
        normal: Vec3,
        half: f64,
    },
}

impl Mark {
    /// Whether the symbol belongs to the Interactions group (else Loads & BCs).
    fn interaction(&self) -> bool {
        matches!(self, Mark::ReferencePoint { .. } | Mark::CouplingLine { .. } | Mark::Wall { .. })
    }
}

/// The symbols of a model's analysis setup, ready to draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glyphs {
    marks: Vec<Mark>,
    /// One symbol unit, in model length.
    unit: f64,
    /// Definitions drawn with a sample of their symbols: `Clamp: 400 of 12000 nodes`.
    pub notes: Vec<String>,
}

/// Every element of a long list, strided down to about [`MAX_SCAN`] of them.
fn scan<T>(items: &[T]) -> impl Iterator<Item = &T> {
    items.iter().step_by(items.len().div_ceil(MAX_SCAN).max(1))
}

/// The nodes that can rotate: shell and beam nodes (not those of 2D sections: plane stress,
/// plane strain, axisymmetric), as the model check counts them.
fn rotating_nodes(model: &Model) -> BTreeSet<u32> {
    let planar: BTreeSet<u32> = model
        .sections()
        .values()
        .filter(|s| s.kind.extent().is_some())
        .filter_map(|s| model.resolve_set_partial(&s.set))
        .flat_map(|r| r.elements)
        .collect();
    model.rotational_nodes(|l| planar.contains(&l))
}

/// The corner node labels of an element face, in the face's order.
fn face_corners(model: &Model, f: ElementFace) -> Option<Vec<u32>> {
    let kind = model.element(f.element)?.kind;
    let local = kind.faces().get(usize::from(f.face).checked_sub(1)?)?;
    let corners = local.iter().filter(|&&i| i < kind.corner_count()).count();
    let mut labels = model.face_nodes(f)?;
    labels.truncate(corners);
    Some(labels)
}

/// A node's coordinates, when finite.
fn xyz(model: &Model, label: u32) -> Option<Vec3> {
    model.node(label).map(|n| Vec3::from(n.xyz)).filter(|p| p.is_finite())
}

/// Two unit vectors perpendicular to `a` and to each other.
fn perpendiculars(a: Vec3) -> (Vec3, Vec3) {
    let helper = if a.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = a.cross(helper).normalized().unwrap_or(Vec3::Y);
    let v = a.cross(u).normalized().unwrap_or(Vec3::Z);
    (u, v)
}

/// Thin `items` at their positions to one per grid cell of side `cell` (the first in each),
/// then stride them down to `cap`.
fn thin<T: Copy>(items: &[(T, Vec3)], cell: f64, cap: usize) -> Vec<T> {
    let cell = if cell.is_finite() && cell > 0.0 { cell } else { 1.0 };
    let key = |v: f64| (v / cell).floor().clamp(-1e15, 1e15) as i64;
    let mut seen = HashSet::new();
    let mut kept: Vec<T> = Vec::new();
    for (item, p) in items {
        if p.is_finite() && seen.insert((key(p.x), key(p.y), key(p.z))) {
            kept.push(*item);
        }
    }
    if kept.len() > cap {
        let stride = kept.len().div_ceil(cap.max(1));
        kept = kept.into_iter().step_by(stride).take(cap).collect();
    }
    kept
}

/// What [`Glyphs::from_model`] reads, with the sets resolved once each.
struct Builder<'m> {
    model: &'m Model,
    sets: BTreeMap<String, ResolvedSet>,
    /// The nodes that can rotate, worked out when a constraint needs them.
    rotating: Option<BTreeSet<u32>>,
    unit: f64,
    bounds: Aabb3,
    marks: Vec<Mark>,
    notes: Vec<String>,
    /// Free arrows placed so far (each beside the last).
    free: usize,
}

impl Builder<'_> {
    fn set(&mut self, name: &str) -> Option<&ResolvedSet> {
        if !self.sets.contains_key(name) {
            let r = self.model.resolve_set_partial(name)?;
            self.sets.insert(name.to_string(), r);
        }
        self.sets.get(name)
    }

    /// A set's nodes (a sample of a huge one) with their coordinates, and how many it has.
    fn node_points(&mut self, set: &str) -> (Vec<(u32, Vec3)>, usize) {
        let model = self.model;
        let Some(r) = self.set(set) else { return (Vec::new(), 0) };
        (scan(&r.nodes).filter_map(|&n| Some((n, xyz(model, n)?))).collect(), r.nodes.len())
    }

    fn room(&self) -> usize {
        MAX_SYMBOLS.saturating_sub(self.marks.len()).min(MAX_PER_DEFINITION)
    }

    /// Note a definition with more places than one definition draws symbols, drawn with a
    /// sample (small ones are only thinned where symbols would overlap).
    fn note(&mut self, name: &str, drawn: usize, total: usize, what: &str) {
        if drawn < total && total > MAX_PER_DEFINITION {
            self.notes.push(format!("{name}: {drawn} of {total} {what}"));
        }
    }

    /// The anchors of a target, thinned on a grid of `cell` units.
    fn anchors(&mut self, name: &str, target: &Target, cell: f64) -> Vec<Anchor> {
        match target {
            Target::ReferencePoint(r) => {
                self.model.reference_point(r).map(|p| Anchor::Point { label: p.label, xyz: Vec3::from(p.xyz) }).into_iter().collect()
            }
            Target::Set(s) => {
                let (at, total) = self.node_points(s);
                let kept = thin(&at, cell * self.unit, self.room());
                self.note(name, kept.len(), total, "nodes");
                kept.into_iter().map(Anchor::Node).collect()
            }
        }
    }

    /// Dots at a thermal condition's nodes.
    fn thermal(&mut self, name: &str, target: &Target) {
        for at in self.anchors(name, target, 1.5) {
            self.marks.push(Mark::Thermal { at });
        }
    }

    /// The loaded faces of a set (its element faces, else the edges of its 2D elements), thinned.
    fn faces(&mut self, name: &str, set: &str) -> Vec<Face> {
        let model = self.model;
        let Some(r) = self.set(set) else { return Vec::new() };
        let face = |labels: &[u32], outward: Vec3| {
            let mut corners = [0u32; 4];
            for (slot, l) in corners.iter_mut().zip(labels) {
                *slot = *l;
            }
            Face { corners, count: labels.len().min(4), outward }
        };
        let corners_at = |labels: &[u32]| -> Option<Vec<Vec3>> { labels.iter().map(|&n| xyz(model, n)).collect() };
        let mut found: Vec<(Face, Vec3)> = Vec::new();
        for &f in scan(&r.faces) {
            let Some(labels) = face_corners(model, f) else { continue };
            let Some(p) = corners_at(&labels) else { continue };
            if let Some(n) = polygon_normal(&p) {
                found.push((face(&labels, n), centre(&p)));
            }
        }
        let total = if found.is_empty() {
            // A 2D model's loaded edges (in the XY plane).
            for &g in scan(&r.edges) {
                if model.element(g.element).is_none_or(|e| e.kind.dimension() != 2) {
                    continue;
                }
                let Some(labels) = model.edge_nodes(g) else { continue };
                let labels = labels.get(..2).unwrap_or(&[]);
                let Some(p) = corners_at(labels) else { continue };
                let (Some(a), Some(b)) = (p.first(), p.get(1)) else { continue };
                let t = *b - *a;
                if let Some(n) = Vec3::new(t.y, -t.x, 0.0).normalized() {
                    found.push((face(labels, n), centre(&p)));
                }
            }
            r.edges.len()
        } else {
            r.faces.len()
        };
        let kept = thin(&found, 1.6 * self.unit, self.room());
        self.note(name, kept.len(), total, "faces");
        kept
    }

    /// The bounding box of a set's nodes.
    fn set_bounds(&mut self, set: &str) -> Aabb3 {
        let mut b = Aabb3::EMPTY;
        for (_, p) in self.node_points(set).0 {
            b.add(p);
        }
        b
    }

    /// A fixed arrow over the model (gravity, body forces).
    fn free_arrow(&mut self, dir: Vec3) {
        let Some(dir) = dir.normalized() else { return };
        let len = 5.0;
        let b = self.bounds;
        // Over the model's middle, in front of it, each one beside the last.
        let at = Vec3::new(b.center().x, b.min.y, b.max.z) + Vec3::new(2.5 * self.free as f64, -4.0, 2.0) * self.unit;
        self.free += 1;
        // The arrow's middle sits at the spot, whichever way it points.
        let tail = at - dir * (0.5 * len * self.unit);
        self.marks.push(Mark::Free { tail, dir, len });
    }

    fn boundary_condition(&mut self, name: &str, bc: &BoundaryCondition) {
        if bc.is_thermal() {
            return self.thermal(name, bc.target());
        }
        let dofs = bc.dofs().bits();
        if dofs == 0 {
            return;
        }
        let anchors = self.anchors(name, bc.target(), 1.5);
        if dofs & 0b111_000 != 0 && self.rotating.is_none() {
            self.rotating = Some(rotating_nodes(self.model));
        }
        let none = BTreeSet::new();
        let rotating = self.rotating.as_ref().unwrap_or(&none);
        for at in anchors {
            let rotates = match at {
                Anchor::Node(n) => rotating.contains(&n),
                Anchor::Point { .. } => true,
            };
            let dofs = if rotates { dofs } else { dofs & 0b000_111 };
            if dofs != 0 {
                self.marks.push(Mark::Constraint { at, dofs });
            }
        }
    }

    fn load(&mut self, name: &str, load: &Load) {
        match load {
            Load::Force { target, vector, .. } | Load::Moment { target, vector, .. } => {
                let Some(dir) = Vec3::from(*vector).normalized() else { return };
                let moment = matches!(load, Load::Moment { .. });
                let anchors = self.anchors(name, target, 2.0);
                let len = if anchors.len() > 1 { 3.0 } else { 5.0 };
                for at in anchors {
                    self.marks.push(Mark::Arrow { at, dir, len, moment });
                }
            }
            Load::Pressure { set, magnitude } => {
                if !magnitude.is_finite() || *magnitude == 0.0 {
                    return;
                }
                let onto = *magnitude > 0.0;
                for face in self.faces(name, set) {
                    self.marks.push(Mark::FaceArrow { face, load: FaceLoad::Pressure { onto } });
                }
            }
            Load::Traction { set, vector } => {
                let Some(dir) = Vec3::from(*vector).normalized() else { return };
                for face in self.faces(name, set) {
                    self.marks.push(Mark::FaceArrow { face, load: FaceLoad::Traction(dir) });
                }
            }
            Load::Gravity { acceleration, .. } => self.free_arrow(Vec3::from(*acceleration)),
            Load::BodyForce { vector, .. } => self.free_arrow(Vec3::from(*vector)),
            Load::Centrifugal { point, axis, .. } => {
                let (Some(axis), point) = (Vec3::from(*axis).normalized(), Vec3::from(*point)) else { return };
                if !point.is_finite() {
                    return;
                }
                // The axis through the model's middle, as long as the model.
                let c = self.bounds.center();
                let centre = point + axis * axis.dot(c - point);
                let half_length = (0.5 * self.bounds.diagonal()).max(4.0 * self.unit);
                self.marks.push(Mark::Spin { centre, axis, half_length });
            }
            Load::Temperature { target, .. } => self.thermal(name, target),
            Load::SurfaceFlux { set, .. } | Load::Convection { set, .. } | Load::HeatSource { set, .. } => {
                self.thermal(name, &Target::Set(set.clone()))
            }
        }
    }

    fn interactions(&mut self) {
        let model = self.model;
        for (name, p) in model.reference_points() {
            let xyz = Vec3::from(p.xyz);
            if xyz.is_finite() && self.marks.len() < MAX_SYMBOLS {
                self.marks.push(Mark::ReferencePoint { at: Anchor::Point { label: p.label, xyz }, name: name.clone() });
            }
        }
        for c in model.couplings().values() {
            let Some(p) = model.reference_point(&c.reference_point) else { continue };
            let from = Anchor::Point { label: p.label, xyz: Vec3::from(p.xyz) };
            let (at, _) = self.node_points(&c.set);
            // Spread over the set: a grid of about four cells across its extent.
            let mut b = Aabb3::EMPTY;
            for (_, q) in &at {
                b.add(*q);
            }
            let cell = (b.diagonal() / 4.0).max(self.unit);
            let room = self.room().min(MAX_COUPLING_LINES);
            for to in thin(&at, cell, room) {
                self.marks.push(Mark::CouplingLine { from, to });
            }
        }
    }

    fn steps(&mut self) {
        let model = self.model;
        for step in model.steps() {
            let Procedure::ExplicitDynamic(e) = &step.procedure else { continue };
            for v in &e.initial_velocities {
                let Some(dir) = Vec3::from(v.velocity).normalized() else { continue };
                let b = self.set_bounds(&v.set);
                if b.is_empty() || self.marks.len() >= MAX_SYMBOLS {
                    continue;
                }
                // Behind the set, ending a unit short of it, so it clears what the set moves
                // toward (a rigid wall).
                let half = (b.max - b.min) * 0.5;
                let reach = (dir.x * half.x).abs() + (dir.y * half.y).abs() + (dir.z * half.z).abs();
                let len = 5.0;
                let tail = b.center() - dir * (reach + (1.0 + len) * self.unit);
                self.marks.push(Mark::Free { tail, dir, len });
            }
            for w in &e.rigid_walls {
                let (Some(normal), point) = (Vec3::from(w.normal).normalized(), Vec3::from(w.point)) else { continue };
                if !point.is_finite() || self.marks.len() >= MAX_SYMBOLS {
                    continue;
                }
                // Behind the set's nodes and wide enough to back them.
                let b = self.set_bounds(&w.set);
                let (point, half) = if b.is_empty() {
                    (point, 4.0)
                } else {
                    let c = b.center();
                    (c - normal * normal.dot(c - point), (0.5 * b.diagonal() / self.unit + 1.0).clamp(2.0, 40.0))
                };
                self.marks.push(Mark::Wall { point, normal, half });
            }
        }
    }
}

impl Glyphs {
    /// The symbols of `model`'s boundary conditions, loads, reference points, couplings and
    /// explicit steps' initial velocities and rigid walls.
    pub fn from_model(model: &Model) -> Glyphs {
        let mut bounds = Aabb3::EMPTY;
        for n in model.nodes().values() {
            bounds.add(Vec3::from(n.xyz));
        }
        for p in model.reference_points().values() {
            bounds.add(Vec3::from(p.xyz));
        }
        let d = bounds.diagonal();
        let unit = if d.is_finite() && d > 0.0 { d * UNIT_FRACTION } else { 1.0 };
        let mut b = Builder { model, sets: BTreeMap::new(), rotating: None, unit, bounds, marks: Vec::new(), notes: Vec::new(), free: 0 };
        for (name, bc) in model.boundary_conditions() {
            b.boundary_condition(name, bc);
        }
        for (name, load) in model.loads() {
            b.load(name, load);
        }
        b.steps();
        b.interactions();
        b.marks.truncate(MAX_SYMBOLS);
        Glyphs { marks: b.marks, unit, notes: b.notes }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.marks.len()
    }

    /// Draw the symbols into `scene`, at the nodes' positions `pos(label)` (the undeformed model,
    /// or a results plot's deformed shape); a symbol at a node `pos` does not know is left out
    /// (a reference point falls back to its own coordinates). Returns the box around what was
    /// drawn, for framing and clipping.
    pub fn draw(&self, pos: impl Fn(u32) -> Option<Vec3>, show: GlyphShow, colors: &GlyphColors, scene: &mut Scene) -> Aabb3 {
        let at = |a: Anchor| -> Option<Vec3> {
            match a {
                Anchor::Node(n) => pos(n),
                Anchor::Point { label, xyz } => Some(pos(label).unwrap_or(xyz)),
            }
            .filter(|p| p.is_finite())
        };
        let u = self.unit;
        let mut d = Draw { scene, bounds: Aabb3::EMPTY };
        for m in &self.marks {
            if !(if m.interaction() { show.interactions } else { show.loads }) {
                continue;
            }
            match m {
                Mark::Constraint { at: a, dofs } => {
                    let Some(p) = at(*a) else { continue };
                    for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
                        let (t, r) = (dofs & (1 << i) != 0, dofs & (1 << (i + 3)) != 0);
                        if t {
                            d.cone(p, axis, u, 0.4 * u, colors.constraint);
                        }
                        if r {
                            // Behind the translation's cone (or where it would be).
                            d.cone(p - axis * (1.05 * u), axis, 0.7 * u, 0.3 * u, colors.constraint);
                        }
                    }
                }
                Mark::Arrow { at: a, dir, len, moment } => {
                    let Some(p) = at(*a) else { continue };
                    if *moment {
                        d.arrow(p, *dir, len * u, u, true, colors.load);
                    } else {
                        d.arrow(p - *dir * (len * u), *dir, len * u, u, false, colors.load);
                    }
                }
                Mark::FaceArrow { face, load } => {
                    let mut p = [Vec3::ZERO; 4];
                    let corners = face.corners.iter().take(face.count).map(|&n| pos(n));
                    if face.count == 0 || corners.zip(p.iter_mut()).any(|(q, slot)| q.map(|q| *slot = q).is_none()) {
                        continue;
                    }
                    let p = p.get(..face.count).unwrap_or(&[]);
                    let c = centre(p);
                    // The face's own normal now (it may be deformed), on the side it pointed.
                    let outward = if face.count >= 3 { polygon_normal(p).map(|n| if n.dot(face.outward) < 0.0 { -n } else { n }) } else { None };
                    let outward = outward.unwrap_or(face.outward);
                    let len = 2.2 * u;
                    match load {
                        FaceLoad::Pressure { onto: true } => d.arrow(c + outward * len, -outward, len, 0.7 * u, false, colors.load),
                        FaceLoad::Pressure { onto: false } => d.arrow(c, outward, len, 0.7 * u, false, colors.load),
                        FaceLoad::Traction(dir) => d.arrow(c - *dir * len, *dir, len, 0.7 * u, false, colors.load),
                    }
                }
                Mark::Thermal { at: a } => {
                    if let Some(p) = at(*a) {
                        d.point(p, colors.thermal, THERMAL_SIZE);
                    }
                }
                Mark::Free { tail, dir, len } => d.arrow(*tail, *dir, len * u, 1.2 * u, false, colors.load),
                Mark::Spin { centre: c, axis, half_length } => {
                    d.line(*c - *axis * *half_length, *c + *axis * *half_length, colors.load, COUPLING_WIDTH);
                    d.spin(*c + *axis * (half_length * 0.8), *axis, 2.0 * u, u, colors.load);
                }
                Mark::ReferencePoint { at: a, name } => {
                    let Some(p) = at(*a) else { continue };
                    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                        d.line(p - axis * (0.9 * u), p + axis * (0.9 * u), colors.interaction, OUTLINE_WIDTH);
                    }
                    d.point(p, colors.interaction, POINT_SIZE);
                    d.scene.labels.push(SceneLabel { p, text: name.clone(), color: colors.label });
                }
                Mark::CouplingLine { from, to } => {
                    if let (Some(a), Some(b)) = (at(*from), pos(*to)) {
                        d.line(a, b, colors.interaction, COUPLING_WIDTH);
                    }
                }
                Mark::Wall { point, normal, half } => d.wall(*point, *normal, half * u, u, colors.constraint),
            }
        }
        d.bounds
    }
}

/// How far a datum plane's square reaches from the model's centre, and a datum axis's line,
/// as a fraction of the bounding box diagonal.
const DATUM_REACH: f64 = 0.6;
/// Hatch lines across a datum plane's square.
const DATUM_HATCH: usize = 4;

/// Draw the model's datum planes (an outlined, hatched square around the model, `bounds`) and
/// axes (a line through the model), each with its name; returns the box around them.
pub fn datums(model: &Model, bounds: &Aabb3, colors: &GlyphColors, scene: &mut Scene) -> Aabb3 {
    let mut d = Draw { scene, bounds: Aabb3::EMPTY };
    let (centre, diagonal) = if bounds.is_empty() { (Vec3::ZERO, 1.0) } else { (bounds.center(), bounds.diagonal().max(1e-9)) };
    let reach = DATUM_REACH * diagonal;
    for (name, datum) in model.datums() {
        match *datum {
            fieldcraft_model::Datum::Plane { point, normal } => {
                let (p, n) = (Vec3::from(point), Vec3::from(normal));
                // The model's centre moved onto the plane.
                let c = centre - n * (centre - p).dot(n);
                let (u, v) = perpendiculars(n);
                let corner = |a: f64, b: f64| c + u * (a * reach) + v * (b * reach);
                let ring = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
                for (k, a) in ring.iter().enumerate() {
                    let b = ring.get((k + 1) % 4).copied().unwrap_or(*a);
                    d.line(*a, b, colors.datum, OUTLINE_WIDTH);
                }
                for k in 1..DATUM_HATCH {
                    let t = -1.0 + 2.0 * k as f64 / DATUM_HATCH as f64;
                    d.line(corner(t, -1.0), corner(t, 1.0), colors.datum, COUPLING_WIDTH);
                    d.line(corner(-1.0, t), corner(1.0, t), colors.datum, COUPLING_WIDTH);
                }
                d.scene.labels.push(SceneLabel { p: corner(1.0, 1.0), text: name.clone(), color: colors.datum });
            }
            fieldcraft_model::Datum::Axis { point, direction } => {
                let (p, a) = (Vec3::from(point), Vec3::from(direction));
                let c = p + a * (centre - p).dot(a);
                d.line(c - a * reach, c + a * reach, colors.datum, SHAFT_WIDTH);
                d.scene.labels.push(SceneLabel { p: c + a * reach, text: name.clone(), color: colors.datum });
            }
        }
    }
    d.bounds
}

/// Symbol primitives into a scene.
struct Draw<'s> {
    scene: &'s mut Scene,
    bounds: Aabb3,
}

impl Draw<'_> {
    fn point(&mut self, p: Vec3, color: Rgb, size: f64) {
        self.bounds.add(p);
        self.scene.points.push(ScenePoint { p, color, size });
    }

    fn line(&mut self, a: Vec3, b: Vec3, color: Rgb, width: f64) {
        if a.is_finite() && b.is_finite() {
            self.bounds.add(a);
            self.bounds.add(b);
            self.scene.lines.push(SceneLine { a, b, color, width });
        }
    }

    fn tri(&mut self, p: [Vec3; 3], color: Rgb) {
        if let Some(normal) = (p[1] - p[0]).cross(p[2] - p[0]).normalized() {
            for q in p {
                self.bounds.add(q);
            }
            self.scene.tris.push(SceneTri { p, normal, color });
        }
    }

    /// A solid cone with its tip at `tip`, pointing along unit `axis`.
    fn cone(&mut self, tip: Vec3, axis: Vec3, len: f64, radius: f64, color: Rgb) {
        let base = tip - axis * len;
        let (u, v) = perpendiculars(axis);
        let ring: [Vec3; CONE_SEGMENTS] = std::array::from_fn(|k| {
            let a = std::f64::consts::TAU * k as f64 / CONE_SEGMENTS as f64;
            base + (u * a.cos() + v * a.sin()) * radius
        });
        for (k, a) in ring.iter().enumerate() {
            let b = ring.get((k + 1) % CONE_SEGMENTS).copied().unwrap_or(*a);
            self.tri([tip, *a, b], color);
            self.tri([base, b, *a], color);
        }
    }

    /// An arrow from `tail` along unit `dir`, `len` long, its head `head` long; a moment's has
    /// two heads.
    fn arrow(&mut self, tail: Vec3, dir: Vec3, len: f64, head: f64, moment: bool, color: Rgb) {
        let tip = tail + dir * len;
        let heads = if moment { 2.0 } else { 1.0 };
        self.line(tail, tip - dir * (head * heads * 0.9), color, SHAFT_WIDTH);
        self.cone(tip, dir, head, 0.38 * head, color);
        if moment {
            self.cone(tip - dir * (0.75 * head), dir, head, 0.38 * head, color);
        }
    }

    /// A curved arrow three quarters of the way around `axis` at `centre`, `radius` across.
    fn spin(&mut self, centre: Vec3, axis: Vec3, radius: f64, head: f64, color: Rgb) {
        let (u, v) = perpendiculars(axis);
        let at = |a: f64| centre + (u * a.cos() + v * a.sin()) * radius;
        let end = 1.5 * std::f64::consts::PI;
        for k in 0..ARC_SEGMENTS {
            let (a, b) = (end * k as f64 / ARC_SEGMENTS as f64, end * (k + 1) as f64 / ARC_SEGMENTS as f64);
            self.line(at(a), at(b), color, SHAFT_WIDTH);
        }
        // The head along the tangent at the arc's end (turning right-handed about the axis).
        let tangent = (u * -end.sin() + v * end.cos()).normalized().unwrap_or(u);
        self.cone(at(end) + tangent * (0.6 * head), tangent, head, 0.38 * head, color);
    }

    /// A rigid wall: a square through `point` normal to `normal`, `half` across, with hatching
    /// on its back.
    fn wall(&mut self, point: Vec3, normal: Vec3, half: f64, unit: f64, color: Rgb) {
        let (u, v) = perpendiculars(normal);
        let c = |a: f64, b: f64| point + u * (a * half) + v * (b * half);
        let corners = [c(-1.0, -1.0), c(1.0, -1.0), c(1.0, 1.0), c(-1.0, 1.0)];
        for (k, a) in corners.iter().enumerate() {
            let b = corners.get((k + 1) % 4).copied().unwrap_or(*a);
            self.line(*a, b, color, SHAFT_WIDTH);
        }
        // Slanted ticks behind the square, on a grid about two symbol units apart.
        let ticks = ((half / unit).floor() as usize).clamp(2, 20);
        let slant = (u + v).normalized().unwrap_or(u);
        for i in 0..=ticks {
            for j in 0..=ticks {
                let at = c(-1.0 + 2.0 * i as f64 / ticks as f64, -1.0 + 2.0 * j as f64 / ticks as f64);
                self.line(at, at - normal * (0.8 * unit) + slant * (0.4 * unit), color, COUPLING_WIDTH);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use fieldcraft_model::{Coupling, CouplingKind, Dofs, ElementKind, ReferencePoint, Set};

    use super::*;

    const COLORS: GlyphColors = GlyphColors {
        constraint: Rgb(0, 0, 255),
        load: Rgb(255, 0, 0),
        interaction: Rgb(0, 255, 0),
        thermal: Rgb(255, 128, 0),
        label: Rgb(0, 0, 0),
        datum: Rgb(0, 128, 128),
    };

    /// A row of `n` unit hex8 cells along x.
    fn bar(n: u32) -> Model {
        let mut m = Model::default();
        let id = |i: u32, j: u32, k: u32| 1 + i * 4 + j * 2 + k;
        for i in 0..=n {
            for j in 0..2 {
                for k in 0..2 {
                    m.set_node(id(i, j, k), [i as f64, j as f64, k as f64]).unwrap();
                }
            }
        }
        for i in 0..n {
            let nodes = vec![id(i, 0, 0), id(i + 1, 0, 0), id(i + 1, 1, 0), id(i, 1, 0), id(i, 0, 1), id(i + 1, 0, 1), id(i + 1, 1, 1), id(i, 1, 1)];
            m.set_element(i + 1, ElementKind::Hex8, nodes).unwrap();
        }
        m.add_set("root", Set { nodes: vec![1, 2, 3, 4], ..Set::default() }).unwrap();
        m.add_set("tip", Set { nodes: (0..4).map(|k| 4 * n + 1 + k).collect(), ..Set::default() }).unwrap();
        m
    }

    fn drawn(g: &Glyphs, m: &Model, show: GlyphShow) -> Scene {
        let mut s = Scene::default();
        g.draw(|l| m.node(l).map(|n| Vec3::from(n.xyz)), show, &COLORS, &mut s);
        s
    }

    #[test]
    fn constraints_draw_a_cone_per_translation_and_rotations_only_where_nodes_rotate() {
        let mut m = bar(4);
        m.add_boundary_condition("Clamp", BoundaryCondition::Fixed { target: Target::Set("root".into()) }).unwrap();
        let g = Glyphs::from_model(&m);
        assert!((g.unit - m_diag(&m) * UNIT_FRACTION).abs() < 1e-12);
        // Solid nodes have no rotations: three cones on each of the four nodes, 2 × 10 triangles
        // a cone.
        let s = drawn(&g, &m, GlyphShow::ALL);
        assert_eq!(s.tris.len(), 4 * 3 * 2 * CONE_SEGMENTS);
        assert!(s.tris.iter().all(|t| t.color == COLORS.constraint));
        // A cone's tip is on its node; the cones reach a symbol unit below the root.
        assert!(s.tris.iter().any(|t| t.p[0] == Vec3::ZERO));
        let mut again = Scene::default();
        let b = g.draw(|l| m.node(l).map(|n| Vec3::from(n.xyz)), GlyphShow::ALL, &COLORS, &mut again);
        assert!((b.min.x + g.unit).abs() < 1e-9 && b.max.x > 0.3 * g.unit && b.max.x <= 0.4 * g.unit + 1e-12, "{b:?}");
        // Hidden with Loads & BCs.
        let s = drawn(&g, &m, GlyphShow { loads: false, interactions: true });
        assert!(s.tris.is_empty() && s.lines.is_empty());
        // A beam node can rotate: six cones.
        let mut m = Model::default();
        m.set_node(1, [0.0; 3]).unwrap();
        m.set_node(2, [10.0, 0.0, 0.0]).unwrap();
        m.set_element(1, ElementKind::Beam2, vec![1, 2]).unwrap();
        m.add_set("end", Set { nodes: vec![1], ..Set::default() }).unwrap();
        m.add_boundary_condition(
            "Pin",
            BoundaryCondition::Displacement { target: Target::Set("end".into()), values: [Some(0.0), None, None, Some(0.0), None, None] },
        )
        .unwrap();
        let s = drawn(&Glyphs::from_model(&m), &m, GlyphShow::ALL);
        assert_eq!(s.tris.len(), 2 * 2 * CONE_SEGMENTS, "one translation and one rotation");
    }

    fn m_diag(m: &Model) -> f64 {
        let mut b = Aabb3::EMPTY;
        for n in m.nodes().values() {
            b.add(Vec3::from(n.xyz));
        }
        b.diagonal()
    }

    #[test]
    fn forces_end_on_their_nodes_and_pressures_point_onto_their_faces() {
        let mut m = bar(4);
        m.add_load("Push", Load::Force { target: Target::Set("tip".into()), vector: [0.0, 0.0, -10.0], distribution: Default::default() }).unwrap();
        let g = Glyphs::from_model(&m);
        let s = drawn(&g, &m, GlyphShow::ALL);
        // One shaft per tip node, coming down onto it.
        assert_eq!(s.lines.len(), 4);
        for l in &s.lines {
            assert!(l.a.z > l.b.z && l.a.x == 4.0 && l.color == COLORS.load, "{l:?}");
        }
        // A pressure on the end face (x = 4): one arrow onto it, from outside (x > 4).
        let mut m = bar(4);
        m.add_set("end", Set { faces: vec![fieldcraft_model::ElementFace { element: 4, face: 4 }], ..Set::default() }).unwrap();
        m.add_load("Press", Load::Pressure { set: "end".into(), magnitude: 2.0 }).unwrap();
        let s = drawn(&Glyphs::from_model(&m), &m, GlyphShow::ALL);
        let l = s.lines.first().unwrap();
        assert_eq!(s.lines.len(), 1);
        assert!(l.a.x > 4.0 && l.b.x > 4.0 && l.b.x < l.a.x, "{l:?}");
        // A suction points away.
        m.replace_load("Press", Load::Pressure { set: "end".into(), magnitude: -2.0 }).unwrap();
        let s = drawn(&Glyphs::from_model(&m), &m, GlyphShow::ALL);
        let l = s.lines.first().unwrap();
        assert!((l.a.x - 4.0).abs() < 1e-9 && l.b.x > l.a.x, "{l:?}");
    }

    #[test]
    fn reference_points_couplings_gravity_and_moments() {
        let mut m = bar(4);
        m.add_reference_point("RP-1", ReferencePoint { xyz: [6.0, 0.5, 0.5], label: 0 }).unwrap();
        m.add_coupling("Tie", Coupling { reference_point: "RP-1".into(), set: "tip".into(), kind: CouplingKind::Kinematic, dofs: Dofs::all() })
            .unwrap();
        m.add_load(
            "Twist",
            Load::Moment { target: Target::ReferencePoint("RP-1".into()), vector: [1.0, 0.0, 0.0], distribution: Default::default() },
        )
        .unwrap();
        m.add_load("g", Load::Gravity { acceleration: [0.0, 0.0, -9810.0], set: None }).unwrap();
        let g = Glyphs::from_model(&m);
        let s = drawn(&g, &m, GlyphShow::ALL);
        assert_eq!(s.labels, vec![SceneLabel { p: Vec3::new(6.0, 0.5, 0.5), text: "RP-1".into(), color: COLORS.label }]);
        assert_eq!(s.lines.iter().filter(|l| l.color == COLORS.interaction && l.width == COUPLING_WIDTH).count(), 4, "a line to each tip node");
        // The moment's two heads and gravity's one: three cones.
        assert_eq!(s.tris.len(), 3 * 2 * CONE_SEGMENTS);
        // The moment starts at the point.
        assert!(s.lines.iter().any(|l| l.a == Vec3::new(6.0, 0.5, 0.5) && l.b.x > 6.0 && l.color == COLORS.load));
        // Interactions hidden: no point, cross or lines to the nodes; the loads stay.
        let s = drawn(&g, &m, GlyphShow { loads: true, interactions: false });
        assert!(s.labels.is_empty() && s.points.is_empty() && s.lines.iter().all(|l| l.color == COLORS.load));
        // A results plot moves the symbols with the nodes; the reference point keeps its place
        // when the plot does not know it.
        let mut s = Scene::default();
        g.draw(|l| m.node(l).map(|n| Vec3::from(n.xyz) + Vec3::new(0.0, 0.0, 1.0)), GlyphShow::ALL, &COLORS, &mut s);
        assert!(s.lines.iter().filter(|l| l.width == COUPLING_WIDTH).all(|l| l.a == Vec3::new(6.0, 0.5, 0.5) && l.b.z >= 1.0));
    }

    #[test]
    fn rigid_walls_back_their_set_and_initial_velocities_leave_it() {
        use fieldcraft_model::{ExplicitDynamic, Friction, InitialVelocity, Procedure, RigidWall, Step};
        let mut m = bar(4);
        m.add_set("all", Set { nodes: m.nodes().keys().copied().collect(), ..Set::default() }).unwrap();
        let e = ExplicitDynamic {
            initial_velocities: vec![InitialVelocity { set: "all".into(), velocity: [-10.0, 0.0, 0.0] }],
            rigid_walls: vec![RigidWall { set: "root".into(), point: [0.0, 0.0, 0.0], normal: [1.0, 0.0, 0.0], friction: Friction::NONE }],
            ..ExplicitDynamic::new(1.0)
        };
        m.add_step(Step::new("Impact", Procedure::ExplicitDynamic(e)), None).unwrap();
        let g = Glyphs::from_model(&m);
        let s = drawn(&g, &m, GlyphShow { loads: false, interactions: true });
        // The wall: in the plane x = 0, centred behind the root face, hatched toward -x.
        let outline: Vec<&SceneLine> = s.lines.iter().filter(|l| l.width == SHAFT_WIDTH).collect();
        assert_eq!(outline.len(), 4);
        assert!(outline.iter().all(|l| l.a.x.abs() < 1e-9 && l.b.x.abs() < 1e-9));
        let mid = outline.iter().fold(Vec3::ZERO, |a, l| a + l.a) * 0.25;
        assert!(mid.dist(Vec3::new(0.0, 0.5, 0.5)) < 1e-9, "{mid:?}");
        assert!(s.lines.iter().filter(|l| l.width == COUPLING_WIDTH).all(|l| l.b.x < 0.0));
        // The velocity: one arrow behind the bar's +x end, pointing along -x toward it.
        let s = drawn(&g, &m, GlyphShow { loads: true, interactions: false });
        let shaft = s.lines.first().unwrap();
        assert_eq!(s.lines.len(), 1);
        assert!(shaft.b.x > 4.0 && shaft.b.x < shaft.a.x, "{shaft:?}");
    }

    #[test]
    fn colours_mute_toward_the_background() {
        let m = COLORS.muted(Rgb(255, 255, 255), 0.5);
        assert_eq!(m.constraint, Rgb(128, 128, 255));
        assert_eq!(COLORS.muted(Rgb(1, 2, 3), f64::NAN), COLORS);
    }

    /// A set of a quarter million nodes draws a capped sample and says so; hostile coordinates and
    /// vectors draw nothing rather than fail.
    #[test]
    fn huge_and_hostile_setups_are_sampled_and_bounded() {
        let mut m = Model::default();
        let n = 500u32;
        for i in 0..n {
            for j in 0..n {
                m.set_node(1 + i * n + j, [i as f64, j as f64, 0.0]).unwrap();
            }
        }
        m.add_set("all", Set { nodes: (1..=n * n).collect(), ..Set::default() }).unwrap();
        m.add_boundary_condition("Clamp", BoundaryCondition::Fixed { target: Target::Set("all".into()) }).unwrap();
        m.add_load("Push", Load::Force { target: Target::Set("all".into()), vector: [0.0, 0.0, -1.0], distribution: Default::default() }).unwrap();
        let g = Glyphs::from_model(&m);
        assert!(g.len() <= 2 * MAX_PER_DEFINITION, "{}", g.len());
        assert!(g.notes.iter().any(|n| n.starts_with("Clamp: ") && n.ends_with(" of 250000 nodes")), "{:?}", g.notes);
        let s = drawn(&g, &m, GlyphShow::ALL);
        assert!(s.tris.len() <= MAX_PER_DEFINITION * (3 + 1) * 2 * CONE_SEGMENTS);
        // Positions that are not finite are left out.
        let mut s = Scene::default();
        g.draw(|_| Some(Vec3::new(f64::NAN, 0.0, 0.0)), GlyphShow::ALL, &COLORS, &mut s);
        assert!(s.tris.is_empty() && s.lines.is_empty());
        // An empty model, and one with a reference point only.
        assert_eq!(Glyphs::from_model(&Model::default()).len(), 0);
        let mut m = Model::default();
        m.add_reference_point("RP", ReferencePoint { xyz: [1e9, -1e9, 0.0], label: 0 }).unwrap();
        let g = Glyphs::from_model(&m);
        let mut s = Scene::default();
        g.draw(|_| None, GlyphShow::ALL, &COLORS, &mut s);
        assert_eq!(s.labels.len(), 1);
    }

    #[test]
    fn grid_thinning_keeps_one_per_cell_and_respects_the_cap() {
        let items: Vec<(u32, Vec3)> = (0..100).map(|i| (i, Vec3::new(f64::from(i) * 0.1, 0.0, 0.0))).collect();
        assert_eq!(thin(&items, 1.0, 1000).len(), 10);
        assert_eq!(thin(&items, 1e-9, 7).len(), 7);
        assert_eq!(thin(&items, f64::NAN, 1000).len(), 10);
        assert!(thin::<u32>(&[], 1.0, 10).is_empty());
        assert!(thin(&[(1u32, Vec3::new(f64::INFINITY, 0.0, 0.0))], 1.0, 10).is_empty());
    }

    #[test]
    fn datums_draw_around_the_model_with_their_names() {
        let mut m = Model::default();
        m.add_datum("Mid", fieldcraft_model::Datum::Plane { point: [5.0, 0.0, 0.0], normal: [1.0, 0.0, 0.0] }).unwrap();
        m.add_datum("Ax", fieldcraft_model::Datum::Axis { point: [0.0; 3], direction: [0.0, 0.0, 1.0] }).unwrap();
        let mut bounds = Aabb3::EMPTY;
        bounds.add(Vec3::ZERO);
        bounds.add(Vec3::new(10.0, 10.0, 10.0));
        let mut scene = Scene::default();
        let drawn = datums(&m, &bounds, &COLORS, &mut scene);
        // A square with its hatch (4 + 2 × 3 lines) and the axis line.
        assert_eq!(scene.lines.len(), 11);
        let mut names: Vec<&str> = scene.labels.iter().map(|l| l.text.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["Ax", "Mid"]);
        // The plane's lines lie on x = 5, centred on the model.
        assert!(scene.lines.iter().skip(1).all(|l| (l.a.x - 5.0).abs() < 1e-12 && (l.b.x - 5.0).abs() < 1e-12));
        assert!(drawn.diagonal() > bounds.diagonal());
    }
}
