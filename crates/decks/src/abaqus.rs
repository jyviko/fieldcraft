//! Abaqus input (`.inp`) writer.
//!
//! The deck is written from the public Abaqus keyword and element references, in our own words;
//! nothing here comes from a deck produced by the product.
//!
//! # Layout
//!
//! The deck is *flat*: nodes, elements, sets, materials, sections and couplings sit at model level,
//! with no parts, assembly or instances. ModelCraft has one mesh with model-wide labels and no
//! instance transforms, so a part/assembly wrapper would only prefix every set name with an
//! instance name (`Part-1-1.root`) and bring the 80-character limit on the joined name closer. When
//! the model gains several parts or instances the writer switches to the part/assembly form.
//!
//! In order: heading, nodes (mesh nodes, then reference points), elements grouped by element type,
//! sets (a node set, an element set and an element-face surface per model set, each written when
//! the set resolves to some), materials, sections, couplings, the initial state's boundary
//! conditions, then the steps.
//!
//! # Steps and the activity table
//!
//! The model lists, for every step, exactly the boundary conditions and loads active in it; Abaqus
//! carries them from step to step. The writer bridges the two:
//! - a boundary condition active in every step that holds its degrees of freedom at zero is written
//!   once, in the initial state (model data);
//! - a general (static) step whose active boundary conditions equal the previous general step's
//!   writes none; one that only adds writes the additions (`OP=MOD`); otherwise it writes
//!   `*BOUNDARY, OP=NEW` and restates every active one;
//! - loads in a general step are restated, keyword by keyword, with `OP=NEW` whenever the active set
//!   changes, so a load not listed is removed;
//! - a frequency step is a perturbation of the last general step: its boundary conditions are
//!   written against that base state and do not carry on to later steps; loads have no effect on
//!   natural frequencies and are not written in it;
//! - every step restates its output request (`*OUTPUT, FIELD, OP=NEW`).
//!
//! # Loads
//!
//! - Forces, moments and tractions are written as concentrated loads from the model's resolved
//!   nodal values (the same distribution every writer and the solver use), summed per node and
//!   degree of freedom so loads sharing a node add up.
//! - Pressure is a surface load on the set's element-face surface, summed per set.
//! - Gravity and body forces are element loads on element sets (the section sets when gravity acts
//!   on the whole model); when two active loads of one kind share elements they are summed and
//!   written per element.
//!
//! # Numbers and labels
//!
//! Real numbers are written in the shortest form that reads back to the same `f64` (exponent marker
//! `E`). Lines stay within 256 characters: element connectivity puts at most 15 nodes after the
//! label on the first line and 16 per continuation line; sets list 16 labels per line. Labels are
//! the model's names (a letter, then letters, digits, `_` or `-`, at most 64 characters), which
//! Abaqus reads unquoted. Abaqus ignores case in names, so two names that differ only in case are
//! reported as [`Error::Unsupported`], as is everything else the format cannot express.
//!
//! # Element types
//!
//! | ModelCraft | Solid section | Shell section | Beam | Rod |
//! |---|---|---|---|---|
//! | tet4 | C3D4 (H) | | | |
//! | tet10 | C3D10 (H) | | | |
//! | wedge6 | C3D6 (H) | | | |
//! | hex8 | C3D8, C3D8R, C3D8I (H) | | | |
//! | hex20 | C3D20, C3D20R (H) | | | |
//! | tri3, quad4 | | S3 / S3R, S4 / S4R | | |
//! | quad8 | | S8R (reduced only) | | |
//! | beam2, rod2 | | | B31 | T3D2 |
//!
//! `H` marks the hybrid variant. A four-node tetrahedron integrates at one point either way, so
//! reduced integration writes C3D4. Not expressible: reduced integration of ten-node tetrahedra and
//! six-node wedges, full integration of eight-node shells, six-node triangular shells, orthotropic
//! materials outside solid sections (shell and beam material axes need orientations), and
//! distributing couplings that leave a translation free.
//!
//! Node order: the model stores elements in Abaqus order, so connectivity passes through; the
//! model's face numbers are Abaqus's S1–S6 for solids, and SPOS/SNEG for shell faces 1/2.

use std::collections::{BTreeMap, BTreeSet};

use modelcraft_model::{
    BeamProfile, BoundaryCondition, CouplingKind, Elastic, ElementKind, Integration, Load, Model, OutputVariable, Procedure, Resolved, Section,
    SectionKind, Step, Symmetry, Target,
};

use crate::{Error, Result};

const FORMAT: &str = "Abaqus";

/// Longest line Abaqus reads.
pub const MAX_LINE: usize = 256;

/// Labels per set data line.
const SET_LABELS_PER_LINE: usize = 16;

/// Nodes on an element's first data line (after its label), and on each continuation line.
const FIRST_LINE_NODES: usize = 15;
const CONTINUATION_NODES: usize = 16;

/// Write the model as an Abaqus input deck. Fails when the model does not resolve (its check
/// finds an error) or has something the format cannot express ([`Error::Unsupported`], every such
/// item named).
pub fn write(model: &Model) -> Result<String> {
    let resolved = model.resolve()?;
    let types = element_types(model, &resolved)?;
    check_expressible(model)?;
    let mut d = Deck::default();
    d.heading(model);
    d.nodes(model)?;
    d.elements(model, &types)?;
    d.sets(model, &resolved)?;
    d.materials(model)?;
    d.sections(model)?;
    let surfaces = coupling_surfaces(model, &resolved);
    d.couplings(model, &resolved, &surfaces)?;
    d.steps(model, &resolved)?;
    Ok(d.out)
}

fn unsupported(items: BTreeSet<String>) -> Result<()> {
    if items.is_empty() { Ok(()) } else { Err(Error::Unsupported { format: FORMAT, what: items.into_iter().collect::<Vec<_>>().join("; ") }) }
}

/// The Abaqus element type of an element of `kind` in `section`, or why there is none.
pub fn element_type(kind: ElementKind, section: &Section) -> std::result::Result<&'static str, String> {
    use ElementKind as K;
    let f = section.formulation;
    let reduced = f.integration == Integration::Reduced;
    let hybrid = |plain: &'static str, h: &'static str| if f.hybrid { h } else { plain };
    let t = match (&section.kind, kind) {
        (SectionKind::Solid {}, K::Tet4) if !f.incompatible_modes => hybrid("C3D4", "C3D4H"),
        (SectionKind::Solid {}, K::Tet10 | K::Wedge6) if reduced => {
            return Err(format!("reduced integration of {} elements (Abaqus has no reduced-integration variant)", kind.name()));
        }
        (SectionKind::Solid {}, K::Tet10) if !f.incompatible_modes => hybrid("C3D10", "C3D10H"),
        (SectionKind::Solid {}, K::Wedge6) if !f.incompatible_modes => hybrid("C3D6", "C3D6H"),
        (SectionKind::Solid {}, K::Hex8) => match (reduced, f.incompatible_modes) {
            (false, false) => hybrid("C3D8", "C3D8H"),
            (true, false) => hybrid("C3D8R", "C3D8RH"),
            (false, true) => hybrid("C3D8I", "C3D8IH"),
            (true, true) => return Err("incompatible modes with reduced integration".into()),
        },
        (SectionKind::Solid {}, K::Hex20) if !f.incompatible_modes => {
            if reduced {
                hybrid("C3D20R", "C3D20RH")
            } else {
                hybrid("C3D20", "C3D20H")
            }
        }
        (SectionKind::Shell { .. }, K::Tri3) if !f.hybrid && !f.incompatible_modes => {
            if reduced {
                "S3R"
            } else {
                "S3"
            }
        }
        (SectionKind::Shell { .. }, K::Quad4) if !f.hybrid && !f.incompatible_modes => {
            if reduced {
                "S4R"
            } else {
                "S4"
            }
        }
        (SectionKind::Shell { .. }, K::Quad8) if !f.hybrid && !f.incompatible_modes => {
            if !reduced {
                return Err("full integration of quad8 shells (Abaqus's eight-node shell, S8R, uses reduced integration)".into());
            }
            "S8R"
        }
        (SectionKind::Shell { .. }, K::Tri6) => return Err("tri6 shells (Abaqus has no general-purpose six-node triangular shell)".into()),
        (SectionKind::Beam { .. }, K::Beam2) if !f.hybrid && !f.incompatible_modes => "B31",
        (SectionKind::Rod { .. }, K::Rod2) if !f.hybrid && !f.incompatible_modes => "T3D2",
        (k, _) => return Err(format!("{} elements in a {} section with this formulation", kind.name(), k.name())),
    };
    Ok(t)
}

/// The element type of every element, from its section.
fn element_types(model: &Model, r: &Resolved) -> Result<BTreeMap<u32, &'static str>> {
    let mut types = BTreeMap::new();
    let mut bad = BTreeSet::new();
    for (name, section) in model.sections() {
        let Some(set) = r.sets.get(&section.set) else { continue };
        for &e in &set.elements {
            let Some(element) = model.element(e) else { continue };
            match element_type(element.kind, section) {
                Ok(t) => {
                    types.insert(e, t);
                }
                Err(why) => {
                    bad.insert(format!("section `{name}`: {why}"));
                }
            }
        }
    }
    unsupported(bad)?;
    // A resolved model gives every element a section; this guards the writer, not the model.
    if let Some(e) = model.elements().keys().find(|e| !types.contains_key(e)) {
        return Err(Error::Unsupported { format: FORMAT, what: format!("element {e} has no section") });
    }
    Ok(types)
}

/// Everything else the format cannot express: case-only name clashes, orthotropic materials
/// outside solids, distributing couplings with a free translation.
fn check_expressible(model: &Model) -> Result<()> {
    let mut bad = BTreeSet::new();
    let namespaces: [(&str, Vec<&str>); 4] = [
        ("sets", model.sets().keys().map(String::as_str).collect()),
        ("materials", model.materials().keys().map(String::as_str).collect()),
        ("couplings", model.couplings().keys().map(String::as_str).collect()),
        ("steps", model.steps().iter().map(|s| s.name.as_str()).collect()),
    ];
    for (what, names) in namespaces {
        let mut seen: BTreeMap<String, &str> = BTreeMap::new();
        for n in names {
            if let Some(other) = seen.insert(n.to_ascii_uppercase(), n) {
                bad.insert(format!("{what} `{other}` and `{n}` differ only in case, and Abaqus ignores case in names"));
            }
        }
    }
    for (name, section) in model.sections() {
        let ortho = matches!(model.material(&section.material).and_then(|m| m.elastic), Some(Elastic::EngineeringConstants { .. }));
        if ortho && section.kind.dimension() != 3 {
            bad.insert(format!(
                "section `{name}`: orthotropic material `{}` in a {} section (its material axes need an orientation, not supported yet)",
                section.material,
                section.kind.name()
            ));
        }
    }
    for (name, c) in model.couplings() {
        if c.kind == CouplingKind::Distributing && !(1..=3).all(|d| c.dofs.contains(d)) {
            bad.insert(format!("coupling `{name}`: an Abaqus distributing coupling always couples translations 1-3"));
        }
    }
    unsupported(bad)
}

/// A real number in the shortest form that reads back to the same value: `0.1`, `210000.0`,
/// `7.85E-9`, `1.0E300`.
pub fn number(v: f64) -> Result<String> {
    if !v.is_finite() {
        return Err(Error::Unsupported { format: FORMAT, what: format!("the value {v} (not a finite number)") });
    }
    // Rust's `Debug` for f64 is the shortest round-trip form, switching to an exponent for very
    // large and small magnitudes.
    let s = format!("{v:?}");
    Ok(match s.split_once('e') {
        Some((mantissa, exponent)) if mantissa.contains('.') => format!("{mantissa}E{exponent}"),
        Some((mantissa, exponent)) => format!("{mantissa}.0E{exponent}"),
        None => s,
    })
}

/// The surface face identifier of face `face` (1-based) of an element of `kind`.
pub fn face_id(kind: ElementKind, face: u8) -> Option<String> {
    let count = kind.faces().len();
    if face == 0 || usize::from(face) > count {
        return None;
    }
    Some(match kind.dimension() {
        3 => format!("S{face}"),
        2 if face == 1 => "SPOS".into(),
        2 => "SNEG".into(),
        _ => return None,
    })
}

/// The node surface each coupling ties its reference point to: a fresh name per coupling that
/// clashes with no set (sets with faces own surfaces of their name) and no other coupling surface.
fn coupling_surfaces(model: &Model, r: &Resolved) -> BTreeMap<String, String> {
    let mut taken: BTreeSet<String> = r.sets.iter().filter(|(_, s)| !s.faces.is_empty()).map(|(n, _)| n.to_ascii_uppercase()).collect();
    let mut out = BTreeMap::new();
    for name in model.couplings().keys() {
        let base = format!("{name}-nodes");
        let mut candidate = base.clone();
        let mut k = 2u32;
        while taken.contains(&candidate.to_ascii_uppercase()) {
            candidate = format!("{base}-{k}");
            k = k.saturating_add(1);
        }
        taken.insert(candidate.to_ascii_uppercase());
        out.insert(name.clone(), candidate);
    }
    out
}

/// Consecutive runs of degrees of freedom: `[1, 2, 3, 5]` → `[(1, 3), (5, 5)]`.
fn dof_runs(dofs: &[u8]) -> Vec<(u8, u8)> {
    let mut runs: Vec<(u8, u8)> = Vec::new();
    for &d in dofs {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == d => *last = d,
            _ => runs.push((d, d)),
        }
    }
    runs
}

/// Whether a boundary condition holds every degree of freedom it constrains at zero.
fn holds_at_zero(bc: &BoundaryCondition) -> bool {
    (1..=6).all(|d| bc.value(d).is_none_or(|v| v == 0.0))
}

/// The loads of one step, as the deck writes them.
#[derive(Default)]
struct StepLoads {
    /// (node, degree of freedom) → summed value.
    concentrated: BTreeMap<(u32, u8), f64>,
    /// Surface (set name) → summed pressure.
    pressure: BTreeMap<String, f64>,
    /// Element-load data lines (gravity and body forces), already formatted.
    element: Vec<String>,
}

/// A gravity or body force on some elements, before it is written.
struct ElementLoad<'a> {
    /// The element sets that hold exactly its elements.
    elsets: Vec<&'a str>,
    elements: Vec<u32>,
    vector: [f64; 3],
}

/// Whether no element is in two of the loads.
fn disjoint(loads: &[ElementLoad]) -> bool {
    let mut all: Vec<u32> = loads.iter().flat_map(|l| l.elements.iter().copied()).collect();
    let n = all.len();
    all.sort_unstable();
    all.dedup();
    all.len() == n
}

fn gravity_line(target: &str, a: [f64; 3]) -> Result<Option<String>> {
    let g = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if !(g > 0.0) {
        return Ok(None);
    }
    Ok(Some(format!("{target}, GRAV, {}, {}, {}, {}", number(g)?, number(a[0] / g)?, number(a[1] / g)?, number(a[2] / g)?)))
}

fn body_lines(target: &str, v: [f64; 3], out: &mut Vec<String>) -> Result<()> {
    for (label, c) in ["BX", "BY", "BZ"].into_iter().zip(v) {
        if c != 0.0 {
            out.push(format!("{target}, {label}, {}", number(c)?));
        }
    }
    Ok(())
}

/// Element-load lines for one kind of load (gravity or body force): by element set when the loads
/// share no element, otherwise summed per element.
fn element_load_lines(loads: &[ElementLoad], gravity: bool, out: &mut Vec<String>) -> Result<()> {
    let emit = |target: &str, v: [f64; 3], out: &mut Vec<String>| -> Result<()> {
        if gravity {
            if let Some(l) = gravity_line(target, v)? {
                out.push(l);
            }
            Ok(())
        } else {
            body_lines(target, v, out)
        }
    };
    if disjoint(loads) {
        for l in loads {
            for s in &l.elsets {
                emit(s, l.vector, out)?;
            }
        }
    } else {
        let mut summed: BTreeMap<u32, [f64; 3]> = BTreeMap::new();
        for l in loads {
            for &e in &l.elements {
                let v = summed.entry(e).or_default();
                for (c, add) in v.iter_mut().zip(l.vector) {
                    *c += add;
                }
            }
        }
        for (e, v) in summed {
            emit(&e.to_string(), v, out)?;
        }
    }
    Ok(())
}

impl StepLoads {
    fn new(model: &Model, r: &Resolved, step: &Step) -> Result<StepLoads> {
        let mut s = StepLoads::default();
        let mut gravity = Vec::new();
        let mut body = Vec::new();
        let section_sets: Vec<&str> = model.sections().values().map(|sec| sec.set.as_str()).collect();
        let all_elements: Vec<u32> = model.elements().keys().copied().collect();
        for name in &step.loads {
            let Some(load) = model.load(name) else { continue };
            match load {
                Load::Force { .. } | Load::Moment { .. } | Load::Traction { .. } => {
                    let first_dof = if matches!(load, Load::Moment { .. }) { 4 } else { 1 };
                    for (node, v) in r.nodal_loads.get(name).map(Vec::as_slice).unwrap_or_default() {
                        for (k, c) in (first_dof..).zip(v) {
                            *s.concentrated.entry((*node, k)).or_default() += c;
                        }
                    }
                }
                Load::Pressure { set, magnitude } => *s.pressure.entry(set.clone()).or_default() += magnitude,
                Load::Gravity { acceleration, set } => gravity.push(match set {
                    Some(set) => ElementLoad {
                        elsets: vec![set.as_str()],
                        elements: r.sets.get(set).map(|x| x.elements.clone()).unwrap_or_default(),
                        vector: *acceleration,
                    },
                    None => ElementLoad { elsets: section_sets.clone(), elements: all_elements.clone(), vector: *acceleration },
                }),
                Load::BodyForce { set, vector } => body.push(ElementLoad {
                    elsets: vec![set.as_str()],
                    elements: r.sets.get(set).map(|x| x.elements.clone()).unwrap_or_default(),
                    vector: *vector,
                }),
            }
        }
        element_load_lines(&gravity, true, &mut s.element)?;
        element_load_lines(&body, false, &mut s.element)?;
        s.concentrated.retain(|_, v| *v != 0.0);
        s.pressure.retain(|_, v| *v != 0.0);
        Ok(s)
    }
}

#[derive(Default)]
struct Deck {
    out: String,
}

impl Deck {
    fn line(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn comment(&mut self, s: &str) {
        self.out.push_str("** ");
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// Labels, [`SET_LABELS_PER_LINE`] per line.
    fn labels(&mut self, labels: &[u32]) {
        for chunk in labels.chunks(SET_LABELS_PER_LINE) {
            let line: Vec<String> = chunk.iter().map(u32::to_string).collect();
            self.line(&line.join(", "));
        }
    }

    fn heading(&mut self, model: &Model) {
        let units = match model.units().length() {
            Some(_) => format!("units {}", model.units().label()),
            None => "units not specified".to_string(),
        };
        self.line("*HEADING");
        self.line(&format!("ModelCraft model, {units}"));
        self.comment("Written by ModelCraft. The numbers are in one consistent unit system and are not converted.");
    }

    fn node_line(&mut self, label: u32, xyz: [f64; 3]) -> Result<()> {
        let line = format!("{label}, {}, {}, {}", number(xyz[0])?, number(xyz[1])?, number(xyz[2])?);
        self.line(&line);
        Ok(())
    }

    fn nodes(&mut self, model: &Model) -> Result<()> {
        self.comment("Nodes");
        self.line("*NODE");
        for (label, node) in model.nodes() {
            self.node_line(*label, node.xyz)?;
        }
        if !model.reference_points().is_empty() {
            self.comment("Reference points");
            let mut points: Vec<(&String, _)> = model.reference_points().iter().collect();
            points.sort_by_key(|(_, p)| p.label);
            self.line("*NODE");
            for (name, p) in points {
                self.comment(&format!("reference point {name}"));
                self.node_line(p.label, p.xyz)?;
            }
        }
        Ok(())
    }

    fn elements(&mut self, model: &Model, types: &BTreeMap<u32, &'static str>) -> Result<()> {
        let mut by_type: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
        for (e, t) in types {
            by_type.entry(t).or_default().push(*e);
        }
        self.comment("Elements");
        for (t, labels) in by_type {
            self.line(&format!("*ELEMENT, TYPE={t}"));
            for e in labels {
                let Some(element) = model.element(e) else { continue };
                let (first, rest) = element.nodes.split_at(element.nodes.len().min(FIRST_LINE_NODES));
                let mut line = e.to_string();
                for n in first {
                    line.push_str(&format!(", {n}"));
                }
                for chunk in rest.chunks(CONTINUATION_NODES) {
                    // A trailing comma continues the data line.
                    line.push(',');
                    self.line(&line);
                    line = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
                }
                self.line(&line);
            }
        }
        Ok(())
    }

    fn sets(&mut self, model: &Model, r: &Resolved) -> Result<()> {
        if r.sets.is_empty() {
            return Ok(());
        }
        self.comment("Sets");
        for (name, s) in &r.sets {
            if s.is_empty() {
                self.comment(&format!("set {name} resolves to nothing on this mesh"));
                continue;
            }
            if !s.nodes.is_empty() {
                self.line(&format!("*NSET, NSET={name}"));
                self.labels(&s.nodes);
            }
            if !s.elements.is_empty() {
                self.line(&format!("*ELSET, ELSET={name}"));
                self.labels(&s.elements);
            }
            if !s.faces.is_empty() {
                self.line(&format!("*SURFACE, TYPE=ELEMENT, NAME={name}"));
                for f in &s.faces {
                    let id = model.element(f.element).and_then(|e| face_id(e.kind, f.face)).ok_or_else(|| Error::Unsupported {
                        format: FORMAT,
                        what: format!("set `{name}`: element {} has no face {}", f.element, f.face),
                    })?;
                    self.line(&format!("{}, {id}", f.element));
                }
            }
        }
        Ok(())
    }

    fn materials(&mut self, model: &Model) -> Result<()> {
        if model.materials().is_empty() {
            return Ok(());
        }
        self.comment("Materials");
        for (name, m) in model.materials() {
            self.line(&format!("*MATERIAL, NAME={name}"));
            match m.elastic {
                Some(Elastic::Isotropic { young, poisson }) => {
                    self.line("*ELASTIC, TYPE=ISOTROPIC");
                    self.line(&format!("{}, {}", number(young)?, number(poisson)?));
                }
                Some(Elastic::EngineeringConstants { young, poisson, shear }) => {
                    // E1, E2, E3, ν12, ν13, ν23, G12, G13 on the first line, G23 on the second.
                    self.line("*ELASTIC, TYPE=ENGINEERING CONSTANTS");
                    let first: Vec<String> = young.iter().chain(&poisson).chain(&shear[..2]).map(|v| number(*v)).collect::<Result<_>>()?;
                    self.line(&first.join(", "));
                    self.line(&number(shear[2])?);
                }
                None => {}
            }
            if let Some(rho) = m.density {
                self.line("*DENSITY");
                self.line(&number(rho)?);
            }
        }
        Ok(())
    }

    fn sections(&mut self, model: &Model) -> Result<()> {
        if model.sections().is_empty() {
            return Ok(());
        }
        self.comment("Sections");
        for (name, s) in model.sections() {
            self.comment(&format!("section {name}"));
            let (set, material) = (&s.set, &s.material);
            match s.kind {
                SectionKind::Solid {} => {
                    self.line(&format!("*SOLID SECTION, ELSET={set}, MATERIAL={material}"));
                    self.line(",");
                }
                SectionKind::Rod { area } => {
                    self.line(&format!("*SOLID SECTION, ELSET={set}, MATERIAL={material}"));
                    self.line(&number(area)?);
                }
                SectionKind::Shell { thickness, offset, points } => {
                    let offset = if offset == 0.0 { String::new() } else { format!(", OFFSET={}", number(offset)?) };
                    self.line(&format!("*SHELL SECTION, ELSET={set}, MATERIAL={material}{offset}"));
                    self.line(&format!("{}, {points}", number(thickness)?));
                }
                SectionKind::Beam { profile, orientation } => {
                    // Abaqus profiles: RECT a, b; CIRC r; PIPE r, t; I l, h, b1, b2, t1, t2, t3
                    // (l: from the section origin to the bottom; centred here); BOX a, b, t1–t4.
                    let (shape, dims) = match profile {
                        BeamProfile::Rectangle { width, height } => ("RECT", vec![width, height]),
                        BeamProfile::Circle { radius } => ("CIRC", vec![radius]),
                        BeamProfile::Pipe { radius, thickness } => ("PIPE", vec![radius, thickness]),
                        BeamProfile::I { height, width, web, flange } => ("I", vec![height / 2.0, height, width, width, flange, flange, web]),
                        BeamProfile::Box { width, height, thickness } => ("BOX", vec![width, height, thickness, thickness, thickness, thickness]),
                    };
                    self.line(&format!("*BEAM SECTION, SECTION={shape}, ELSET={set}, MATERIAL={material}"));
                    let dims: Vec<String> = dims.into_iter().map(number).collect::<Result<_>>()?;
                    self.line(&dims.join(", "));
                    let n1: Vec<String> = orientation.into_iter().map(number).collect::<Result<_>>()?;
                    self.line(&n1.join(", "));
                }
            }
        }
        Ok(())
    }

    fn couplings(&mut self, model: &Model, r: &Resolved, surfaces: &BTreeMap<String, String>) -> Result<()> {
        if model.couplings().is_empty() {
            return Ok(());
        }
        self.comment("Couplings: every node of the set weighted equally");
        for (name, c) in model.couplings() {
            let (Some(surface), Some(node)) = (surfaces.get(name), r.reference_point_nodes.get(&c.reference_point)) else { continue };
            self.line(&format!("*SURFACE, TYPE=NODE, NAME={surface}"));
            self.line(&format!("{}, 1.0", c.set));
            self.line(&format!("*COUPLING, CONSTRAINT NAME={name}, REF NODE={node}, SURFACE={surface}"));
            self.line(match c.kind {
                CouplingKind::Kinematic => "*KINEMATIC",
                CouplingKind::Distributing => "*DISTRIBUTING, WEIGHTING METHOD=UNIFORM",
            });
            for (a, b) in dof_runs(&c.dofs.list()) {
                self.line(&format!("{a}, {b}"));
            }
        }
        Ok(())
    }

    fn boundary_lines(&mut self, model: &Model, r: &Resolved, names: &BTreeSet<String>) -> Result<()> {
        for name in names {
            let Some(bc) = model.boundary_condition(name) else { continue };
            let target = match bc.target() {
                Target::Set(s) => s.clone(),
                Target::ReferencePoint(p) => match r.reference_point_nodes.get(p) {
                    Some(n) => n.to_string(),
                    None => continue,
                },
            };
            self.comment(&format!("boundary condition {name}"));
            match bc {
                BoundaryCondition::Fixed { .. } => self.line(&format!("{target}, ENCASTRE")),
                BoundaryCondition::Symmetry { plane, .. } => {
                    let s = match plane {
                        Symmetry::X => "XSYMM",
                        Symmetry::Y => "YSYMM",
                        Symmetry::Z => "ZSYMM",
                    };
                    self.line(&format!("{target}, {s}"));
                }
                BoundaryCondition::Displacement { values, .. } => {
                    for (dof, v) in (1u8..).zip(values) {
                        if let Some(v) = v {
                            self.line(&format!("{target}, {dof}, {dof}, {}", number(*v)?));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Boundary conditions of a step against the state it starts from.
    fn step_boundaries(&mut self, model: &Model, r: &Resolved, base: &BTreeSet<String>, active: &BTreeSet<String>) -> Result<()> {
        if active == base {
            self.comment("boundary conditions: as in the base state");
        } else if active.is_superset(base) {
            self.line("*BOUNDARY");
            let added: BTreeSet<String> = active.difference(base).cloned().collect();
            self.boundary_lines(model, r, &added)?;
        } else {
            self.line("*BOUNDARY, OP=NEW");
            self.boundary_lines(model, r, active)?;
        }
        Ok(())
    }

    fn step_loads(&mut self, loads: &StepLoads, had: &mut [bool; 3]) -> Result<()> {
        let has = [!loads.concentrated.is_empty(), !loads.pressure.is_empty(), !loads.element.is_empty()];
        if has[0] || had[0] {
            self.line("*CLOAD, OP=NEW");
            for ((node, dof), v) in &loads.concentrated {
                self.line(&format!("{node}, {dof}, {}", number(*v)?));
            }
        }
        if has[1] || had[1] {
            self.line("*DSLOAD, OP=NEW");
            for (surface, p) in &loads.pressure {
                self.line(&format!("{surface}, P, {}", number(*p)?));
            }
        }
        if has[2] || had[2] {
            self.line("*DLOAD, OP=NEW");
            for l in &loads.element {
                self.line(l);
            }
        }
        *had = has;
        Ok(())
    }

    fn output(&mut self, step: &Step) {
        let node: Vec<&str> = step
            .output
            .iter()
            .filter_map(|v| match v {
                OutputVariable::Displacement => Some("U"),
                OutputVariable::ReactionForce => Some("RF"),
                _ => None,
            })
            .collect();
        let element: Vec<&str> = step
            .output
            .iter()
            .filter_map(|v| match v {
                OutputVariable::Stress => Some("S"),
                OutputVariable::Strain => Some("E"),
                _ => None,
            })
            .collect();
        if node.is_empty() && element.is_empty() {
            self.line("*OUTPUT, FIELD, OP=NEW, FREQUENCY=0");
            return;
        }
        self.line("*OUTPUT, FIELD, OP=NEW");
        if !node.is_empty() {
            self.line("*NODE OUTPUT");
            self.line(&node.join(", "));
        }
        if !element.is_empty() {
            self.line("*ELEMENT OUTPUT");
            self.line(&element.join(", "));
        }
    }

    fn steps(&mut self, model: &Model, r: &Resolved) -> Result<()> {
        let steps = model.steps();
        // The initial state: zero-valued boundary conditions active in every step.
        let initial: BTreeSet<String> = model
            .boundary_conditions()
            .iter()
            .filter(|(n, bc)| holds_at_zero(bc) && !steps.is_empty() && steps.iter().all(|s| s.boundary_conditions.contains(*n)))
            .map(|(n, _)| n.clone())
            .collect();
        if !initial.is_empty() {
            self.comment("Initial state: boundary conditions active in every step");
            self.line("*BOUNDARY");
            self.boundary_lines(model, r, &initial)?;
        }
        let mut base_bcs = initial;
        let mut base_loads: BTreeSet<String> = BTreeSet::new();
        let mut had = [false; 3];
        for step in steps {
            self.comment(&format!("Step {}", step.name));
            match step.procedure {
                Procedure::Static {} => {
                    self.line(&format!("*STEP, NAME={}, NLGEOM=NO", step.name));
                    self.line("*STATIC");
                    self.line("1.0, 1.0");
                    self.step_boundaries(model, r, &base_bcs, &step.boundary_conditions)?;
                    if step.loads == base_loads {
                        self.comment("loads: as in the previous general step");
                    } else {
                        let loads = StepLoads::new(model, r, step)?;
                        self.step_loads(&loads, &mut had)?;
                    }
                    base_bcs = step.boundary_conditions.clone();
                    base_loads = step.loads.clone();
                }
                Procedure::Frequency { modes, min_hz, max_hz } => {
                    self.line(&format!("*STEP, NAME={}", step.name));
                    self.line("*FREQUENCY, EIGENSOLVER=LANCZOS");
                    // Modes, then the frequency range; empty trailing fields are left off.
                    let mut fields = vec![modes.to_string()];
                    for f in [min_hz, max_hz] {
                        fields.push(f.map(number).transpose()?.unwrap_or_default());
                    }
                    while fields.last().is_some_and(String::is_empty) {
                        fields.pop();
                    }
                    self.line(&fields.join(", "));
                    self.step_boundaries(model, r, &base_bcs, &step.boundary_conditions)?;
                    if !step.loads.is_empty() {
                        self.comment("loads have no effect on natural frequencies and are not written");
                    }
                }
            }
            self.output(step);
            self.line("*END STEP");
        }
        Ok(())
    }
}
