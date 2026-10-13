//! LS-DYNA keyword decks for implicit linear static and eigenvalue analyses.
//!
//! The deck is written from the public LS-DYNA keyword manual (R14), in our own words and with
//! our own examples. What it writes:
//! - `*KEYWORD long=y`: every field 20 columns wide (see [`card`] for why), then `*NODE` for the
//!   mesh and the reference points;
//! - one `*PART` per section and element kind, with `*SECTION_SOLID`, `*SECTION_SHELL` or
//!   `*SECTION_BEAM` (rods as trusses) and the element formulation mapped from the section's
//!   ([`elements`]); `*HOURGLASS` where a reduced-integration formulation needs it;
//! - `*MAT_ELASTIC` for isotropic and `*MAT_ORTHOTROPIC_ELASTIC` (global material axes) for
//!   orthotropic materials;
//! - `*ELEMENT_SOLID` (tetrahedra and wedges as degenerate hexahedra, 10-node tetrahedra),
//!   `*ELEMENT_SOLID_H20`, `*ELEMENT_SHELL` and `*ELEMENT_BEAM`;
//! - `*SET_NODE_LIST` and `*SET_SEGMENT` for the sets boundary conditions, loads and couplings
//!   use (with the set's name as title);
//! - couplings: kinematic as `*CONSTRAINED_NODAL_RIGID_BODY` (the reference point first in its
//!   node set), distributing as `*CONSTRAINED_INTERPOLATION` (the reference point dependent on
//!   the set's translations, equal weights);
//! - per step: `*CONTROL_IMPLICIT_GENERAL`, `*CONTROL_IMPLICIT_SOLUTION` (linear) or
//!   `*CONTROL_IMPLICIT_EIGENVALUE`, `*CONTROL_TERMINATION` (a static step is one implicit step
//!   from time 0 to 1 with every load ramped by one curve, `*DEFINE_CURVE`); boundary conditions
//!   as `*BOUNDARY_SPC_*_ID` (named) and `*BOUNDARY_PRESCRIBED_MOTION_*_ID` for non-zero
//!   displacements; forces, moments and tractions as `*LOAD_NODE_SET`/`_POINT` from the
//!   resolved nodal values; pressure as `*LOAD_SEGMENT_SET`; gravity as `*LOAD_BODY_X/Y/Z`
//!   (with `*LOAD_BODY_PARTS` when it acts on a set); output as `*DATABASE_BINARY_D3PLOT`,
//!   `*DATABASE_SPCFORC` and `*DATABASE_EXTENT_BINARY`.
//!
//! Several steps are independent analyses in the model (nothing carries over), which is what
//! LS-DYNA's `*CASE` runs: each step is a case between `*CASE_BEGIN_n` and `*CASE_END_n`, run in
//! order when LS-DYNA is started with `CASE` on its command line. A single step is written
//! without cases.
//!
//! What LS-DYNA cannot take is an [`Error::Unsupported`] naming the item: beam sections (not yet
//! written), body forces per volume, kinematic couplings of some degrees of freedom only, boundary
//! conditions on a coupled reference point, gravity on different sets, hybrid formulations other
//! than on tet4, reduced integration of quadratic elements, materials without a density, more
//! than 10 shell integration points.

mod card;
mod elements;

use std::collections::{BTreeMap, BTreeSet};

use modelcraft_model::{
    BoundaryCondition, CouplingKind, Elastic, ElementKind, Load, Model, OutputVariable, Procedure, Resolved, SectionKind, Step, Target,
};

use crate::{Error, Result};
use card::{Deck, Field};
use elements::{FORMAT, Hourglass, Keyword};

use card::WIDTH;

/// A written deck.
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    /// The deck, ASCII.
    pub text: String,
    /// Reals that did not fit a 20-column field exactly and were rounded (to at least 13
    /// significant digits).
    pub rounded_values: usize,
    /// Number of `*CASE` analyses (0 when the model has one step and no cases are written).
    pub cases: usize,
}

/// Largest deck written, in bytes (a deck is built in memory).
pub const MAX_DECK_BYTES: usize = 1 << 31;

/// Most shell integration points LS-DYNA's Gauss rule takes.
const MAX_SHELL_POINTS: u32 = 10;

fn unsupported(what: impl Into<String>) -> Error {
    Error::Unsupported { format: FORMAT, what: what.into() }
}

/// One `*PART`: a section's elements of one kind.
struct Part<'a> {
    id: u32,
    section: &'a str,
    kind: ElementKind,
    elform: i32,
    material: u32,
    hourglass: Option<u32>,
    elements: Vec<u32>,
}

/// Every id the deck uses, decided before anything is written.
struct Plan<'a> {
    parts: Vec<Part<'a>>,
    /// Material name → MID, for the materials sections use.
    materials: BTreeMap<&'a str, u32>,
    hourglasses: BTreeMap<Hourglass, u32>,
    /// Set name → SID (node and segment sets have separate id spaces; both use this number).
    sets: BTreeMap<&'a str, u32>,
    /// Sets written as node sets, and as segment sets.
    node_sets: BTreeSet<&'a str>,
    segment_sets: BTreeSet<&'a str>,
    /// The part set gravity acts on, when it acts on a set.
    gravity_parts: Option<Vec<u32>>,
}

/// Write the model as an LS-DYNA keyword deck.
pub fn write(model: &Model) -> Result<Written> {
    let r = model.resolve()?;
    let estimate = (model.nodes().len() + model.reference_points().len()) * 130 + model.elements().len() * 260;
    if estimate > MAX_DECK_BYTES {
        return Err(Error::Limit(format!("the deck would be about {estimate} bytes; at most {MAX_DECK_BYTES} are written")));
    }
    let plan = plan(model, &r)?;
    let mut d = Deck::default();
    let steps = model.steps();
    let cases = if steps.len() > 1 { steps.len() } else { 0 };

    d.keyword("*KEYWORD long=y");
    d.comment("Written by ModelCraft: implicit linear analysis, long format (20-column fields).");
    d.comment(&format!("Units: {} (consistent; nothing is converted).", model.units().label()));
    if cases > 0 {
        d.comment(&format!("{cases} steps are written as *CASE analyses: run LS-DYNA with CASE on its command line."));
    }
    nodes(&mut d, model);
    materials(&mut d, model, &plan)?;
    parts(&mut d, model, &plan)?;
    element_cards(&mut d, model, &plan)?;
    sets(&mut d, model, &r, &plan)?;
    couplings(&mut d, model, &r, &plan)?;
    if let Some(parts) = &plan.gravity_parts {
        d.keyword("*SET_PART_LIST");
        d.header(&["sid"]);
        d.card(&[Field::from(1u32)]);
        d.list(parts);
        d.keyword("*LOAD_BODY_PARTS");
        d.header(&["psid"]);
        d.card(&[Field::from(1u32)]);
    }
    let mut ids = Ids::default();
    for (i, step) in steps.iter().enumerate() {
        let n = u32::try_from(i + 1).map_err(|_| Error::Limit("too many steps".into()))?;
        if cases > 0 {
            d.keyword(&format!("*CASE_BEGIN_{n}"));
        }
        write_step(&mut d, model, &r, &plan, step, n, &mut ids)?;
        if cases > 0 {
            d.keyword(&format!("*CASE_END_{n}"));
        }
    }
    d.keyword("*END");
    if d.text.len() > MAX_DECK_BYTES {
        return Err(Error::Limit(format!("the deck is {} bytes; at most {MAX_DECK_BYTES} are written", d.text.len())));
    }
    Ok(Written { text: d.text, rounded_values: d.rounded, cases })
}

fn plan<'a>(model: &'a Model, r: &Resolved) -> Result<Plan<'a>> {
    let mut materials = BTreeMap::new();
    for sec in model.sections().values() {
        materials.insert(sec.material.as_str(), 0);
    }
    for (i, mid) in materials.values_mut().enumerate() {
        *mid = id(i)?;
    }
    let mut parts = Vec::new();
    let mut hourglasses: BTreeMap<Hourglass, u32> = BTreeMap::new();
    for (name, sec) in model.sections() {
        let mut by_kind: BTreeMap<ElementKind, Vec<u32>> = BTreeMap::new();
        for e in r.sets.get(&sec.set).map(|s| s.elements.as_slice()).unwrap_or_default() {
            if let Some(el) = model.element(*e) {
                by_kind.entry(el.kind).or_default().push(*e);
            }
        }
        if let SectionKind::Shell { points, .. } = sec.kind
            && points > MAX_SHELL_POINTS
        {
            return Err(unsupported(format!(
                "section `{name}`: {points} integration points through the thickness (LS-DYNA's Gauss rule takes at most {MAX_SHELL_POINTS})"
            )));
        }
        for (kind, list) in by_kind {
            let f = elements::elform(kind, &sec.kind, sec.formulation, name)?;
            let hourglass = match f.hourglass {
                Some(h) => {
                    let next = id(hourglasses.len())?;
                    Some(*hourglasses.entry(h).or_insert(next))
                }
                None => None,
            };
            parts.push(Part {
                id: id(parts.len())?,
                section: name,
                kind,
                elform: f.elform,
                material: materials.get(sec.material.as_str()).copied().unwrap_or_default(),
                hourglass,
                elements: list,
            });
        }
    }
    let sets: BTreeMap<&str, u32> = model.sets().keys().enumerate().map(|(i, n)| Ok((n.as_str(), id(i)?))).collect::<Result<_>>()?;

    // Which sets are written, and what the steps need that LS-DYNA may not express.
    let coupled: BTreeSet<&str> = model.couplings().values().map(|c| c.reference_point.as_str()).collect();
    let mut node_sets = BTreeSet::new();
    let mut segment_sets = BTreeSet::new();
    for (name, c) in model.couplings() {
        if c.kind == CouplingKind::Kinematic && c.dofs.list().len() < 6 {
            return Err(unsupported(format!(
                "coupling `{name}`: a kinematic coupling of some degrees of freedom only (LS-DYNA's nodal rigid bodies release degrees of freedom reliably only for two nodes)"
            )));
        }
        if c.kind == CouplingKind::Distributing {
            node_sets.insert(c.set.as_str());
        }
    }
    let mut gravity_sets: BTreeSet<Option<&str>> = BTreeSet::new();
    for step in model.steps() {
        let is_static = matches!(step.procedure, Procedure::Static {});
        for b in &step.boundary_conditions {
            let Some(bc) = model.boundary_conditions().get(b) else { continue };
            match bc.target() {
                Target::Set(s) => {
                    node_sets.insert(s.as_str());
                }
                Target::ReferencePoint(p) if coupled.contains(p.as_str()) => {
                    return Err(unsupported(format!(
                        "boundary condition `{b}`: reference point `{p}` is coupled, and LS-DYNA takes no constraint on a node of a nodal rigid body or on an interpolation's dependent node; constrain the coupled set instead"
                    )));
                }
                Target::ReferencePoint(_) => {}
            }
        }
        if !is_static {
            continue;
        }
        for l in &step.loads {
            let Some(load) = model.loads().get(l) else { continue };
            match load {
                Load::Force { target, .. } | Load::Moment { target, .. } => {
                    if let Target::Set(s) = target
                        && uniform_on_set(r, l, s).is_some()
                    {
                        node_sets.insert(s.as_str());
                    }
                }
                Load::Traction { .. } => {}
                Load::Pressure { set, .. } => {
                    segment_sets.insert(set.as_str());
                }
                Load::Gravity { set, .. } => {
                    gravity_sets.insert(set.as_deref());
                }
                Load::BodyForce { .. } => {
                    return Err(unsupported(format!(
                        "load `{l}`: a body force per unit volume (LS-DYNA's body loads are accelerations; use gravity)"
                    )));
                }
            }
        }
    }
    let gravity_parts = match gravity_sets.into_iter().collect::<Vec<_>>().as_slice() {
        [] | [None] => None,
        [Some(set)] => Some(parts_of_set(r, &parts, set)?),
        _ => return Err(unsupported("gravity on different sets (LS-DYNA applies every body load to one part subset per deck)")),
    };
    for m in materials.keys() {
        if model.material(m).is_some_and(|m| m.density.is_none()) {
            return Err(unsupported(format!("material `{m}` has no density, and LS-DYNA requires a mass density on every material")));
        }
    }
    Ok(Plan { parts, materials, hourglasses, sets, node_sets, segment_sets, gravity_parts })
}

/// A 1-based id from a 0-based index.
fn id(i: usize) -> Result<u32> {
    u32::try_from(i + 1).map_err(|_| Error::Limit("more than 4294967295 items of one kind".into()))
}

/// The parts that make up a set's elements exactly, or why not.
fn parts_of_set(r: &Resolved, parts: &[Part], set: &str) -> Result<Vec<u32>> {
    let elements: BTreeSet<u32> = r.sets.get(set).map(|s| s.elements.iter().copied().collect()).unwrap_or_default();
    let mut ids = Vec::new();
    let mut covered = 0;
    for p in parts {
        let inside = p.elements.iter().filter(|e| elements.contains(e)).count();
        if inside == p.elements.len() {
            ids.push(p.id);
            covered += inside;
        } else if inside > 0 {
            return Err(unsupported(format!(
                "gravity on set `{set}`, which holds part of section `{}`'s elements (LS-DYNA applies body loads to whole parts)",
                p.section
            )));
        }
    }
    if covered != elements.len() || ids.is_empty() {
        return Err(unsupported(format!("gravity on set `{set}`, whose elements are not whole sections")));
    }
    Ok(ids)
}

/// The value every node of a force or moment gets, when it is one value on exactly the set's
/// nodes (then the load is written on the node set).
fn uniform_on_set(r: &Resolved, load: &str, set: &str) -> Option<[f64; 3]> {
    let values = r.nodal_loads.get(load)?;
    let first = values.first()?.1;
    let nodes = &r.sets.get(set)?.nodes;
    (values.len() == nodes.len() && values.iter().zip(nodes).all(|((n, v), m)| n == m && *v == first)).then_some(first)
}

fn nodes(d: &mut Deck, model: &Model) {
    let mut all: BTreeMap<u32, [f64; 3]> = model.nodes().iter().map(|(l, n)| (*l, n.xyz)).collect();
    for p in model.reference_points().values() {
        all.insert(p.label, p.xyz);
    }
    d.keyword("*NODE");
    d.header(&["nid", "x", "y", "z"]);
    for (label, [x, y, z]) in all {
        d.card(&[Field::from(label), Field::from(x), Field::from(y), Field::from(z)]);
    }
}

fn materials(d: &mut Deck, model: &Model, plan: &Plan) -> Result<()> {
    for (name, &mid) in &plan.materials {
        let m = model.material(name).ok_or_else(|| Error::Unsupported { format: FORMAT, what: format!("material `{name}` is missing") })?;
        let rho = m.density.unwrap_or_default();
        match m.elastic {
            Some(Elastic::Isotropic { young, poisson }) => {
                d.keyword("*MAT_ELASTIC_TITLE");
                d.text(name);
                d.header(&["mid", "ro", "e", "pr"]);
                d.card(&[Field::from(mid), Field::from(rho), Field::from(young), Field::from(poisson)]);
            }
            Some(Elastic::EngineeringConstants { young: [e1, e2, e3], poisson: [n12, n13, n23], shear: [g12, g13, g23] }) => {
                // LS-DYNA takes the minor ratios: ν_ba = ν_ab·E_b/E_a.
                d.keyword("*MAT_ORTHOTROPIC_ELASTIC_TITLE");
                d.text(name);
                d.header(&["mid", "ro", "ea", "eb", "ec", "prba", "prca", "prcb"]);
                let mut card = vec![Field::from(mid)];
                card.extend([rho, e1, e2, e3, n12 * e2 / e1, n13 * e3 / e1, n23 * e3 / e2].map(Field::from));
                d.card(&card);
                d.header(&["gab", "gbc", "gca", "aopt"]);
                d.card(&[g12, g23, g13, 2.0].map(Field::from));
                d.header(&["xp", "yp", "zp", "a1", "a2", "a3"]);
                d.card(&[Field::Blank, Field::Blank, Field::Blank, Field::from(1.0), Field::from(0.0), Field::from(0.0)]);
                d.header(&["v1", "v2", "v3", "d1", "d2", "d3"]);
                d.card(&[Field::Blank, Field::Blank, Field::Blank, Field::from(0.0), Field::from(1.0), Field::from(0.0)]);
            }
            None => return Err(unsupported(format!("material `{name}` has no elastic constants"))),
        }
    }
    Ok(())
}

fn parts(d: &mut Deck, model: &Model, plan: &Plan) -> Result<()> {
    for p in &plan.parts {
        let sec = model.sections().get(p.section).ok_or_else(|| unsupported(format!("section `{}` is missing", p.section)))?;
        match sec.kind {
            SectionKind::Solid {} => {
                d.keyword("*SECTION_SOLID");
                d.header(&["secid", "elform"]);
                d.card(&[Field::from(p.id), Field::from(p.elform)]);
            }
            SectionKind::Shell { thickness, offset, points } => {
                d.keyword("*SECTION_SHELL");
                d.header(&["secid", "elform", "shrf", "nip"]);
                d.card(&[Field::from(p.id), Field::from(p.elform), Field::from(5.0 / 6.0), Field::from(points)]);
                d.header(&["t1", "t2", "t3", "t4", "nloc"]);
                // The model's offset is the reference (node) surface's distance from the
                // mid-surface along the normal, as a fraction of the thickness; NLOC puts the
                // mid-surface at −NLOC·t/2 from the nodes.
                let nloc = if offset == 0.0 { Field::Blank } else { Field::from(2.0 * offset) };
                d.card(&[Field::from(thickness), Field::Blank, Field::Blank, Field::Blank, nloc]);
            }
            SectionKind::Rod { area } => {
                d.keyword("*SECTION_BEAM");
                d.header(&["secid", "elform"]);
                d.card(&[Field::from(p.id), Field::from(p.elform)]);
                d.header(&["a"]);
                d.card(&[Field::from(area)]);
            }
            SectionKind::Beam { .. } => return Err(unsupported(format!("section `{}`: beam sections are not written yet", p.section))),
        }
    }
    for (h, hgid) in &plan.hourglasses {
        d.keyword("*HOURGLASS");
        d.header(&["hgid", "ihq", "qm"]);
        match h {
            Hourglass::Solid => d.card(&[Field::from(*hgid), Field::from(6), Field::from(1.0)]),
            Hourglass::Shell => d.card(&[Field::from(*hgid), Field::from(4)]),
        }
    }
    let several: BTreeMap<&str, usize> = plan.parts.iter().fold(BTreeMap::new(), |mut m, p| {
        *m.entry(p.section).or_default() += 1;
        m
    });
    for p in &plan.parts {
        d.keyword("*PART");
        if several.get(p.section).copied().unwrap_or_default() > 1 {
            d.text(&format!("{} {}", p.section, p.kind.name()));
        } else {
            d.text(p.section);
        }
        d.header(&["pid", "secid", "mid", "eosid", "hgid"]);
        let hg = p.hourglass.map_or(Field::Blank, Field::from);
        d.card(&[Field::from(p.id), Field::from(p.id), Field::from(p.material), Field::Blank, hg]);
    }
    Ok(())
}

fn element_cards(d: &mut Deck, model: &Model, plan: &Plan) -> Result<()> {
    let mut by_keyword: BTreeMap<Keyword, BTreeMap<u32, u32>> = BTreeMap::new();
    for p in &plan.parts {
        for e in &p.elements {
            by_keyword.entry(Keyword::of(p.kind)).or_default().insert(*e, p.id);
        }
    }
    for (keyword, elements) in by_keyword {
        d.keyword(keyword.name());
        match keyword {
            Keyword::Shell | Keyword::Beam => d.header(&["eid", "pid", "n1", "n2", "n3", "n4", "n5", "n6", "n7", "n8"]),
            Keyword::Solid | Keyword::SolidH20 => d.header(&["eid", "pid", "/ n1", "n2", "n3", "n4", "n5", "n6", "n7", "n8", "n9", "n10"]),
        }
        for (label, pid) in elements {
            let el = model.element(label).ok_or_else(|| unsupported(format!("element {label} is missing")))?;
            let nodes = elements::connectivity(el.kind, &el.nodes)
                .ok_or_else(|| unsupported(format!("element {label} has {} nodes, not {}", el.nodes.len(), el.kind.node_count())))?;
            let fields = |ns: &[u32]| ns.iter().map(|n| Field::from(*n)).collect::<Vec<_>>();
            match keyword {
                Keyword::Shell | Keyword::Beam => {
                    let mut card = vec![Field::from(label), Field::from(pid)];
                    card.extend(fields(&nodes));
                    d.card(&card);
                }
                Keyword::Solid | Keyword::SolidH20 => {
                    d.card(&[Field::from(label), Field::from(pid)]);
                    for chunk in nodes.chunks(10) {
                        d.card(&fields(chunk));
                    }
                }
            }
        }
    }
    Ok(())
}

fn sets(d: &mut Deck, model: &Model, r: &Resolved, plan: &Plan) -> Result<()> {
    for name in &plan.node_sets {
        let (Some(sid), Some(set)) = (plan.sets.get(name), r.sets.get(*name)) else { continue };
        d.keyword("*SET_NODE_LIST_TITLE");
        d.text(name);
        d.header(&["sid"]);
        d.card(&[Field::from(*sid)]);
        d.list(&set.nodes);
    }
    for name in &plan.segment_sets {
        let (Some(sid), Some(set)) = (plan.sets.get(name), r.sets.get(*name)) else { continue };
        d.keyword("*SET_SEGMENT_TITLE");
        d.text(name);
        d.header(&["sid"]);
        d.card(&[Field::from(*sid)]);
        d.header(&["n1", "n2", "n3", "n4"]);
        for f in &set.faces {
            let nodes = model.face_nodes(*f).ok_or_else(|| unsupported(format!("element {} face {} is not on the mesh", f.element, f.face)))?;
            // Corner nodes only: LS-DYNA recognises the quadratic face from its corners and
            // spreads the pressure over the mid-side nodes too.
            let corners: Vec<u32> = match nodes.len() {
                3 | 6 => nodes.iter().take(3).copied().chain(nodes.get(2).copied()).collect(),
                _ => nodes.iter().take(4).copied().collect(),
            };
            let fields: Vec<Field> = corners.iter().map(|n| Field::from(*n)).collect();
            d.card(&fields);
        }
    }
    Ok(())
}

fn couplings(d: &mut Deck, model: &Model, r: &Resolved, plan: &Plan) -> Result<()> {
    let mut next_set = id(model.sets().len())?;
    let mut next_part = id(plan.parts.len())?;
    let mut next_interpolation = 1u32;
    for (name, c) in model.couplings() {
        let rp = r
            .reference_point_nodes
            .get(&c.reference_point)
            .copied()
            .ok_or_else(|| unsupported(format!("coupling `{name}`: no reference point node")))?;
        let nodes = r.sets.get(&c.set).map(|s| s.nodes.as_slice()).unwrap_or_default();
        match c.kind {
            CouplingKind::Kinematic => {
                // The reference point leads: it is the first node of the rigid body's set.
                d.keyword("*SET_NODE_LIST_TITLE");
                d.text(&format!("{name} (rigid body)"));
                d.header(&["sid"]);
                d.card(&[Field::from(next_set)]);
                let members: Vec<u32> = std::iter::once(rp).chain(nodes.iter().copied().filter(|n| *n != rp)).collect();
                d.list(&members);
                d.keyword("*CONSTRAINED_NODAL_RIGID_BODY_TITLE");
                d.text(name);
                d.header(&["pid", "cid", "nsid"]);
                d.card(&[Field::from(next_part), Field::Blank, Field::from(next_set)]);
                next_set = next_set.checked_add(1).ok_or_else(|| Error::Limit("too many node sets".into()))?;
                next_part = next_part.checked_add(1).ok_or_else(|| Error::Limit("too many parts".into()))?;
            }
            CouplingKind::Distributing => {
                let sid =
                    plan.sets.get(c.set.as_str()).copied().ok_or_else(|| unsupported(format!("coupling `{name}`: set `{}` is missing", c.set)))?;
                let ddof: i64 = c.dofs.list().iter().fold(0, |acc, d| acc * 10 + i64::from(*d));
                d.comment(&format!("coupling `{name}`"));
                d.keyword("*CONSTRAINED_INTERPOLATION");
                d.header(&["icid", "dnid", "ddof", "cidd", "ityp", "idnsw", "fgm"]);
                // FGM = 1: the dependent node is not part of the mesh.
                d.card(&[
                    Field::from(next_interpolation),
                    Field::from(rp),
                    Field::Int(ddof),
                    Field::Blank,
                    Field::from(1),
                    Field::Blank,
                    Field::from(1),
                ]);
                d.header(&["inid", "idof"]);
                d.card(&[Field::from(sid), Field::from(123)]);
                next_interpolation += 1;
            }
        }
    }
    Ok(())
}

/// Running ids across steps.
#[derive(Default)]
struct Ids {
    constraint: u32,
}

impl Ids {
    fn next(&mut self) -> u32 {
        self.constraint = self.constraint.saturating_add(1);
        self.constraint
    }
}

/// LS-DYNA's degree-of-freedom code for prescribed motion and nodal loads: 1–3 translations,
/// 5–7 rotations.
fn dof_code(dof: u8) -> i32 {
    if dof <= 3 { i32::from(dof) } else { i32::from(dof) + 1 }
}

fn write_step(d: &mut Deck, model: &Model, r: &Resolved, plan: &Plan, step: &Step, n: u32, ids: &mut Ids) -> Result<()> {
    d.keyword("*TITLE");
    d.text(&step.name);
    let is_static = matches!(step.procedure, Procedure::Static {});
    d.keyword("*CONTROL_IMPLICIT_GENERAL");
    d.header(&["imflag", "dt0"]);
    d.card(&[Field::from(1), Field::from(1.0)]);
    match step.procedure {
        Procedure::Static {} => {
            d.keyword("*CONTROL_IMPLICIT_SOLUTION");
            d.header(&["nsolvr"]);
            d.card(&[Field::from(1)]);
        }
        Procedure::Frequency { modes, min_hz, max_hz } => {
            d.keyword("*CONTROL_IMPLICIT_EIGENVALUE");
            d.header(&["neig", "center", "lflag", "lftend", "rflag", "rhtend"]);
            let bound = |b: Option<f64>| match b {
                Some(v) => [Field::from(1), Field::from(v)],
                None => [Field::Blank, Field::Blank],
            };
            let mut card = vec![Field::from(modes), Field::Blank];
            card.extend(bound(min_hz));
            card.extend(bound(max_hz));
            d.card(&card);
            if step.output.contains(&OutputVariable::Stress) || step.output.contains(&OutputVariable::Strain) {
                d.header(&["isolid", "ibeam", "ishell", "itshell", "mstres"]);
                d.card(&[Field::Blank, Field::Blank, Field::Blank, Field::Blank, Field::from(1)]);
            }
        }
    }
    d.keyword("*CONTROL_TERMINATION");
    d.header(&["endtim"]);
    d.card(&[Field::from(1.0)]);

    // Every load and prescribed motion of a static step follows one ramp from 0 to 1.
    let prescribes = step
        .boundary_conditions
        .iter()
        .filter_map(|b| model.boundary_conditions().get(b))
        .any(|b| (1..=6).any(|dof| b.value(dof).is_some_and(|v| v != 0.0)));
    let loads: Vec<(&String, &Load)> = step.loads.iter().filter_map(|l| model.loads().get_key_value(l)).collect();
    if is_static && (prescribes || !loads.is_empty()) {
        d.keyword("*DEFINE_CURVE_TITLE");
        d.text(&format!("{} ramp", step.name));
        d.header(&["lcid"]);
        d.card(&[Field::from(n)]);
        d.header(&["a1", "o1"]);
        d.card(&[Field::from(0.0), Field::from(0.0)]);
        d.card(&[Field::from(1.0), Field::from(1.0)]);
    }

    for b in &step.boundary_conditions {
        let Some(bc) = model.boundary_conditions().get(b) else { continue };
        boundary_condition(d, r, plan, b, bc, is_static, n, ids)?;
    }
    if is_static {
        let mut gravity = [0.0f64; 3];
        for (name, load) in &loads {
            match load {
                Load::Force { target, .. } | Load::Moment { target, .. } => {
                    let moment = matches!(load, Load::Moment { .. });
                    let uniform = match target {
                        Target::Set(s) => uniform_on_set(r, name, s).zip(plan.sets.get(s.as_str())),
                        Target::ReferencePoint(_) => None,
                    };
                    nodal_load(d, r, name, moment, uniform, n);
                }
                Load::Traction { .. } => nodal_load(d, r, name, false, None, n),
                Load::Pressure { set, magnitude } => {
                    let sid = plan.sets.get(set.as_str()).copied().unwrap_or_default();
                    d.comment(&format!("load `{name}`"));
                    d.keyword("*LOAD_SEGMENT_SET");
                    d.header(&["ssid", "lcid", "sf"]);
                    d.card(&[Field::from(sid), Field::from(n), Field::from(*magnitude)]);
                }
                Load::Gravity { acceleration, .. } => {
                    for (g, a) in gravity.iter_mut().zip(acceleration) {
                        *g += a;
                    }
                }
                Load::BodyForce { .. } => return Err(unsupported(format!("load `{name}`: a body force per unit volume"))),
            }
        }
        // A base acceleration loads the model against its direction, so gravity g is written
        // with scale factor −g.
        for (axis, g) in ["X", "Y", "Z"].iter().zip(gravity) {
            if g != 0.0 {
                d.keyword(&format!("*LOAD_BODY_{axis}"));
                d.header(&["lcid", "sf"]);
                d.card(&[Field::from(n), Field::from(-g)]);
            }
        }
        d.keyword("*DATABASE_BINARY_D3PLOT");
        d.header(&["dt"]);
        d.card(&[Field::from(1.0)]);
        if step.output.contains(&OutputVariable::ReactionForce) {
            d.keyword("*DATABASE_SPCFORC");
            d.header(&["dt"]);
            d.card(&[Field::from(1.0)]);
        }
        if step.output.contains(&OutputVariable::Strain) {
            d.keyword("*DATABASE_EXTENT_BINARY");
            d.header(&["neiph", "neips", "maxint", "strflg"]);
            d.card(&[Field::Blank, Field::Blank, Field::Blank, Field::from(1)]);
        }
    } else if !loads.is_empty() {
        d.comment("Loads have no effect on natural frequencies and are not written in this step.");
    }
    Ok(())
}

fn boundary_condition(
    d: &mut Deck,
    r: &Resolved,
    plan: &Plan,
    name: &str,
    bc: &BoundaryCondition,
    is_static: bool,
    lcid: u32,
    ids: &mut Ids,
) -> Result<()> {
    let (entity, suffix) = match bc.target() {
        Target::Set(s) => {
            (plan.sets.get(s.as_str()).copied().ok_or_else(|| unsupported(format!("boundary condition `{name}`: set `{s}` is missing")))?, "SET")
        }
        Target::ReferencePoint(p) => (
            r.reference_point_nodes
                .get(p)
                .copied()
                .ok_or_else(|| unsupported(format!("boundary condition `{name}`: reference point `{p}` is missing")))?,
            "NODE",
        ),
    };
    let dofs = bc.dofs();
    // Held degrees of freedom (in a frequency step every constrained one is held).
    let held: Vec<u8> = dofs.list().into_iter().filter(|dof| !is_static || bc.value(*dof).is_none_or(|v| v == 0.0)).collect();
    if !held.is_empty() {
        d.keyword(&format!("*BOUNDARY_SPC_{suffix}_ID"));
        id_card(d, ids.next(), name);
        d.header(&["nsid", "cid", "dofx", "dofy", "dofz", "dofrx", "dofry", "dofrz"]);
        let mut card = vec![Field::from(entity), Field::Blank];
        card.extend((1..=6u8).map(|dof| Field::from(i32::from(held.contains(&dof)))));
        d.card(&card);
    }
    if is_static {
        for dof in dofs.list() {
            let Some(v) = bc.value(dof).filter(|v| *v != 0.0) else { continue };
            d.keyword(&format!("*BOUNDARY_PRESCRIBED_MOTION_{suffix}_ID"));
            id_card(d, ids.next(), name);
            d.header(&["nsid", "dof", "vad", "lcid", "sf"]);
            d.card(&[Field::from(entity), Field::from(dof_code(dof)), Field::from(2), Field::from(lcid), Field::from(v)]);
        }
    }
    Ok(())
}

/// The ID card of an `_ID` keyword: an id and a heading.
fn id_card(d: &mut Deck, id: u32, heading: &str) {
    d.text(&format!("{id:>WIDTH$}{heading}"));
}

/// A force, moment or traction from its resolved nodal values: on the node set when every node
/// of the set takes the same value, else node by node.
fn nodal_load(d: &mut Deck, r: &Resolved, name: &str, moment: bool, uniform: Option<([f64; 3], &u32)>, lcid: u32) {
    let offset = if moment { 4 } else { 0 };
    d.comment(&format!("load `{name}`"));
    if let Some((v, sid)) = uniform {
        d.keyword("*LOAD_NODE_SET");
        d.header(&["nsid", "dof", "lcid", "sf"]);
        for (i, c) in v.iter().enumerate() {
            if *c != 0.0 {
                d.card(&[Field::from(*sid), Field::Int(i as i64 + 1 + offset), Field::from(lcid), Field::from(*c)]);
            }
        }
        return;
    }
    let values = r.nodal_loads.get(name).map(Vec::as_slice).unwrap_or_default();
    d.keyword("*LOAD_NODE_POINT");
    d.header(&["nid", "dof", "lcid", "sf"]);
    for (node, v) in values {
        for (i, c) in v.iter().enumerate() {
            if *c != 0.0 {
                d.card(&[Field::from(*node), Field::Int(i as i64 + 1 + offset), Field::from(lcid), Field::from(*c)]);
            }
        }
    }
}
