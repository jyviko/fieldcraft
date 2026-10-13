//! Nastran bulk data: the model as an input file for a linear static (SOL 101) or normal modes
//! (SOL 103) run, written from the public Quick Reference Guide in our own words.
//!
//! A deck has three parts: executive control (the solution sequence), case control (one subcase
//! per step, selecting its constraint set, load set or eigenvalue method and its output) and bulk
//! data (the entries). What the model holds becomes:
//!
//! - nodes and reference points: `GRID`;
//! - solids: `CTETRA` (tet4, tet10), `CPENTA` (wedge6), `CHEXA` (hex8, hex20) with `PSOLID`;
//!   shells: `CTRIA3`, `CQUAD4`, `CTRIA6`, `CQUAD8` with `PSHELL`; beams: `CBEAM` with `PBEAML`;
//!   rods: `CROD` with `PROD`. Nastran numbers a 20-node hexahedron's mid-side nodes bottom
//!   edges, vertical edges, top edges, where the model has bottom, top, vertical
//!   ([`grids`] reorders them); every other kind keeps the model's order;
//! - the element formulation: `PSOLID`'s integration network and scheme (see [`solid_rule`]);
//!   formulations Nastran's linear elements have no counterpart for are reported, not
//!   approximated;
//! - isotropic materials: `MAT1`; orthotropic engineering constants: `MAT9` (the stiffness matrix
//!   in the basic axes) for solids and `MAT8` for shells lying in planes of constant Z, with the
//!   material X axis tied to the basic X axis through a `CORD2R`;
//! - boundary conditions: per step, one constraint set of `SPC1` (held degrees of freedom) and
//!   `SPC` (prescribed values); in a modes step every constrained degree of freedom is held;
//! - forces, moments and tractions: `FORCE` and `MOMENT` at the nodal values the model resolves,
//!   so every writer and the solver share one distribution; pressure: `PLOAD4` per element face;
//!   gravity: `GRAV`; a step with several loads combines them with `LOAD`;
//! - kinematic coupling: `RBE2` (the reference point independent); distributing coupling: `RBE3`
//!   (equal weights on the set's translations; the reference point dependent);
//! - frequency steps: `EIGRL` (Lanczos) with the step's mode count and range.
//!
//! Field widths, continuations and number forms are in [`card`]. Model content Nastran cannot
//! express is reported as [`Error::Unsupported`], every item at once, and nothing is written.
//! The same model always gives the same bytes.

mod card;

use std::collections::{BTreeMap, BTreeSet};

use modelcraft_model::{
    BeamProfile, Coupling, CouplingKind, Elastic, ElementKind, Formulation, Integration, Load, MAX_LABEL, Model, Procedure, Resolved, SectionKind,
    Step,
};

use crate::{Error, Result};
pub(crate) use card::Card;

/// The format's name in errors.
pub const FORMAT: &str = "Nastran";

/// Most problems listed in one [`Error::Unsupported`].
const LISTED: usize = 20;

/// Most `GRAV` entries one run may select.
const MAX_GRAV: usize = 9;

/// Grids per `SPC1` entry; longer lists are split over entries of the same set.
const SPC1_GRIDS: usize = 1000;

/// What to write.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    /// The steps to write, by name (written in model order). `None` writes every step. A run has
    /// one solution sequence, so static and frequency steps go in separate decks.
    pub steps: Option<Vec<String>>,
}

/// The model as a Nastran deck, with every step.
pub fn write(model: &Model) -> Result<String> {
    write_with(model, &Options::default())
}

/// The model as a Nastran deck, with the steps `options` selects.
pub fn write_with(model: &Model, options: &Options) -> Result<String> {
    let resolved = model.resolve()?;
    let steps = select_steps(model, options)?;
    let mut deck = Deck::new(model, &resolved, steps);
    deck.plan()?;
    deck.emit()
}

/// The Nastran grid order of an element's nodes (model order in, Nastran order out).
pub fn grids(kind: ElementKind, nodes: &[u32]) -> Vec<u32> {
    order(kind).iter().filter_map(|&i| nodes.get(i).copied()).collect()
}

/// For each Nastran grid position, the position in the model's node list.
pub fn order(kind: ElementKind) -> &'static [usize] {
    const IDENTITY: [usize; 20] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19];
    // Model: 9–12 bottom edges, 13–16 top edges, 17–20 vertical edges. Nastran: bottom, vertical, top.
    const HEX20: [usize; 20] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 16, 17, 18, 19, 12, 13, 14, 15];
    match kind {
        ElementKind::Hex20 => &HEX20,
        k => IDENTITY.get(..k.node_count()).unwrap_or(&IDENTITY),
    }
}

/// The connection entry's name for an element kind.
pub fn element_card(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Rod2 => "CROD",
        ElementKind::Beam2 => "CBEAM",
        ElementKind::Tri3 => "CTRIA3",
        ElementKind::Tri6 => "CTRIA6",
        ElementKind::Quad4 => "CQUAD4",
        ElementKind::Quad8 => "CQUAD8",
        ElementKind::Tet4 | ElementKind::Tet10 => "CTETRA",
        ElementKind::Wedge6 => "CPENTA",
        ElementKind::Hex8 | ElementKind::Hex20 => "CHEXA",
    }
}

/// `PSOLID`'s integration network (`IN`) and scheme (`ISOP`) for an element kind and
/// formulation, or why Nastran has none.
///
/// - Full integration is the standard isoparametric rule: two points per direction for linear
///   kinds, three for quadratic ones (`TWO`/`THREE`, `FULL`).
/// - Reduced integration of a 20-node hexahedron is the 2 × 2 × 2 standard rule.
/// - Incompatible modes of an 8-node hexahedron are Nastran's bubble functions (`BUBBLE`, with
///   its default reduced-shear scheme).
/// - One-point reduced integration of linear solids (with hourglass control) and the hybrid
///   (displacement–pressure) formulation have no linear Nastran counterpart.
pub fn solid_rule(kind: ElementKind, f: Formulation) -> std::result::Result<(Option<&'static str>, Option<&'static str>), String> {
    if f.hybrid {
        return Err("the hybrid (displacement–pressure) formulation".into());
    }
    let quadratic = matches!(kind, ElementKind::Tet10 | ElementKind::Hex20);
    match (kind, f.incompatible_modes, f.integration) {
        (ElementKind::Hex8, true, _) => Ok((Some("BUBBLE"), None)),
        (_, true, _) => Err(format!("incompatible modes on {} elements", kind.name())),
        (ElementKind::Hex20, false, Integration::Reduced) => Ok((Some("TWO"), Some("FULL"))),
        (_, false, Integration::Reduced) => Err(format!("reduced integration of {} elements", kind.name())),
        (_, false, Integration::Full) if quadratic => Ok((Some("THREE"), Some("FULL"))),
        (_, false, Integration::Full) => Ok((Some("TWO"), Some("FULL"))),
    }
}

/// The stiffness of orthotropic engineering constants for Nastran's `MAT9`, as
/// `[[C11, C12, C13], [C22, C23, C33], [G12, G23, G13]]` (the normal block's upper triangle, then
/// the shear moduli for xy, yz and zx in `MAT9`'s order), or `None` when it is not finite.
pub fn orthotropic_stiffness(young: [f64; 3], poisson: [f64; 3], shear: [f64; 3]) -> Option<[[f64; 3]; 3]> {
    let [e1, e2, e3] = young;
    let [n12, n13, n23] = poisson;
    // Compliance of the normal block (symmetric: ν_ij / E_i = ν_ji / E_j).
    let (s11, s22, s33) = (1.0 / e1, 1.0 / e2, 1.0 / e3);
    let (s12, s13, s23) = (-n12 / e1, -n13 / e1, -n23 / e2);
    let det = s11 * (s22 * s33 - s23 * s23) - s12 * (s12 * s33 - s23 * s13) + s13 * (s12 * s23 - s22 * s13);
    let c = [
        [(s22 * s33 - s23 * s23) / det, (s13 * s23 - s12 * s33) / det, (s12 * s23 - s13 * s22) / det],
        [0.0, (s11 * s33 - s13 * s13) / det, (s12 * s13 - s11 * s23) / det],
        [0.0, 0.0, (s11 * s22 - s12 * s12) / det],
    ];
    let [g12, g13, g23] = shear;
    let out = [[c[0][0], c[0][1], c[0][2]], [c[1][1], c[1][2], c[2][2]], [g12, g23, g13]];
    out.iter().flatten().all(|v| v.is_finite()).then_some(out)
}

fn select_steps<'m>(model: &'m Model, options: &Options) -> Result<Vec<&'m Step>> {
    let Some(names) = &options.steps else { return Ok(model.steps().iter().collect()) };
    if names.is_empty() {
        return Err(Error::Model(modelcraft_model::Error::Invalid("choose at least one step to write".into())));
    }
    for n in names {
        if model.step(n).is_none() {
            let shown: String = n.chars().take(modelcraft_model::MAX_NAME_LEN).collect();
            return Err(Error::Model(modelcraft_model::Error::NotFound(format!("step `{shown}`"))));
        }
    }
    Ok(model.steps().iter().filter(|s| names.contains(&s.name)).collect())
}

/// The component code of a set of degrees of freedom, such as 123 or 156 (0 for none).
fn component(mask: u8) -> i64 {
    (1..=6u8).filter(|d| mask & (1 << (d - 1)) != 0).fold(0, |c, d| c * 10 + i64::from(d))
}

fn mask_of(dofs: &[u8]) -> u8 {
    dofs.iter().filter(|d| (1..=6).contains(*d)).fold(0, |m, d| m | (1 << (d - 1)))
}

/// A material's entry and identification numbers.
#[derive(Debug, Clone, Copy, Default)]
struct MaterialIds {
    /// `MAT1`, for isotropic materials (every element kind).
    isotropic: Option<u32>,
    /// `MAT9`, for orthotropic materials of solids.
    solid: Option<u32>,
    /// `MAT8`, for orthotropic materials of shells.
    shell: Option<u32>,
}

/// One property entry.
enum Property {
    Solid { mid: u32, rule: (Option<&'static str>, Option<&'static str>) },
    Shell { mid: u32, thickness: f64 },
    Beam { mid: u32, profile: BeamProfile },
    Rod { mid: u32, area: f64 },
}

/// What an element's connection entry needs besides its nodes.
#[derive(Clone, Copy)]
enum Extra {
    None,
    Shell { offset: Option<f64>, mcid: Option<u32> },
    Beam { orientation: [f64; 3] },
}

/// A constraint set: per node, the prescribed value of each degree of freedom.
type Constraints = BTreeMap<u32, [Option<f64>; 6]>;

struct Deck<'m> {
    model: &'m Model,
    r: &'m Resolved,
    steps: Vec<&'m Step>,
    problems: Vec<String>,
    next_sid: u32,
    /// Material name → ids.
    materials: BTreeMap<&'m str, MaterialIds>,
    /// The coordinate system tying shell material axes to the basic axes, when one is needed.
    material_csys: Option<u32>,
    /// Property id → (section name, entry).
    properties: BTreeMap<u32, (&'m str, Property)>,
    /// Element → (property id, extra fields).
    element_props: BTreeMap<u32, (u32, Extra)>,
    /// Rigid elements: (id, coupling name, coupling).
    rigid: Vec<(u32, &'m str, &'m Coupling)>,
    /// Node → (dependent degrees of freedom, coupling that makes them dependent).
    dependent: BTreeMap<u32, (u8, &'m str)>,
    /// Constraint sets by id, and each step's set.
    spc_sets: Vec<(u32, Constraints)>,
    step_spc: Vec<Option<u32>>,
    /// Load name → load set id.
    load_sids: BTreeMap<&'m str, u32>,
    /// Combinations: (id, load set ids).
    combinations: Vec<(u32, Vec<u32>)>,
    step_load: Vec<Option<u32>>,
    /// Eigenvalue methods: (id, step index).
    methods: Vec<(u32, usize)>,
}

impl<'m> Deck<'m> {
    fn new(model: &'m Model, r: &'m Resolved, steps: Vec<&'m Step>) -> Deck<'m> {
        Deck {
            model,
            r,
            steps,
            problems: Vec::new(),
            next_sid: 1,
            materials: BTreeMap::new(),
            material_csys: None,
            properties: BTreeMap::new(),
            element_props: BTreeMap::new(),
            rigid: Vec::new(),
            dependent: BTreeMap::new(),
            spc_sets: Vec::new(),
            step_spc: Vec::new(),
            load_sids: BTreeMap::new(),
            combinations: Vec::new(),
            step_load: Vec::new(),
            methods: Vec::new(),
        }
    }

    fn problem(&mut self, p: String) {
        self.problems.push(p);
    }

    fn sid(&mut self) -> Result<u32> {
        let s = self.next_sid;
        if s > MAX_LABEL {
            return Err(Error::Limit(format!("more than {MAX_LABEL} set identification numbers")));
        }
        self.next_sid += 1;
        Ok(s)
    }

    /// The solution sequence, or why there is none.
    fn solution(&self) -> std::result::Result<u32, String> {
        let statics: Vec<&str> = self.steps.iter().filter(|s| matches!(s.procedure, Procedure::Static {})).map(|s| s.name.as_str()).collect();
        let modes: Vec<&str> = self.steps.iter().filter(|s| matches!(s.procedure, Procedure::Frequency { .. })).map(|s| s.name.as_str()).collect();
        match (statics.first(), modes.first()) {
            (Some(_), None) => Ok(101),
            (None, Some(_)) => Ok(103),
            (Some(s), Some(m)) => Err(format!(
                "one run solves either static steps (SOL 101) or frequency steps (SOL 103), and step `{s}` is static while step `{m}` extracts frequencies; write them as separate decks (choose the steps)"
            )),
            (None, None) => Err("there is no step to write".into()),
        }
    }

    fn plan(&mut self) -> Result<()> {
        if let Err(p) = self.solution() {
            self.problem(p);
        }
        self.plan_sections()?;
        self.plan_couplings()?;
        self.plan_constraints()?;
        self.plan_loads()?;
        for i in 0..self.steps.len() {
            if matches!(self.steps.get(i).map(|s| s.procedure), Some(Procedure::Frequency { .. })) {
                let sid = self.sid()?;
                self.methods.push((sid, i));
            }
        }
        if self.problems.is_empty() {
            return Ok(());
        }
        let more = self.problems.len().saturating_sub(LISTED);
        let mut what = self.problems.iter().take(LISTED).cloned().collect::<Vec<_>>().join("; ");
        if more > 0 {
            what.push_str(&format!("; and {more} more"));
        }
        Err(Error::Unsupported { format: FORMAT, what })
    }

    /// Materials, properties and each element's property.
    fn plan_sections(&mut self) -> Result<()> {
        let model = self.model;
        // Which forms each material is needed in: (solid, shell, line).
        let mut uses: BTreeMap<&str, (bool, bool, bool)> = BTreeMap::new();
        for s in model.sections().values() {
            let u = uses.entry(s.material.as_str()).or_default();
            match s.kind {
                SectionKind::Solid {} => u.0 = true,
                SectionKind::Shell { .. } => u.1 = true,
                SectionKind::Beam { .. } | SectionKind::Rod { .. } => u.2 = true,
            }
        }
        let mut next_mid = 1u32;
        for (name, (solid, shell, line)) in uses {
            let Some(material) = model.material(name) else { continue };
            let mut ids = MaterialIds::default();
            match material.elastic {
                Some(Elastic::Isotropic { .. }) => {
                    ids.isotropic = Some(next_mid);
                    next_mid += 1;
                }
                Some(Elastic::EngineeringConstants { .. }) => {
                    if solid {
                        ids.solid = Some(next_mid);
                        next_mid += 1;
                    }
                    if shell {
                        ids.shell = Some(next_mid);
                        next_mid += 1;
                    }
                    if line {
                        self.problem(format!(
                            "material `{name}`: orthotropic constants on beams or rods (their properties take isotropic materials)"
                        ));
                    }
                }
                None => {}
            }
            self.materials.insert(name, ids);
        }

        let r = self.r;
        let mut next_pid = 1u32;
        for (sname, sec) in model.sections() {
            let ids = self.materials.get(sec.material.as_str()).copied();
            let elements = r.sets.get(&sec.set).map(|s| s.elements.as_slice()).unwrap_or_default();
            match sec.kind {
                SectionKind::Solid {} => {
                    let Some(mid) = ids.and_then(|i| i.isotropic.or(i.solid)) else { continue };
                    // One property per integration rule the section's element kinds need.
                    let mut rules: BTreeMap<(Option<&'static str>, Option<&'static str>), Vec<u32>> = BTreeMap::new();
                    let mut bad: BTreeSet<String> = BTreeSet::new();
                    for &e in elements {
                        let Some(el) = model.element(e) else { continue };
                        match solid_rule(el.kind, sec.formulation) {
                            Ok(rule) => rules.entry(rule).or_default().push(e),
                            Err(why) => {
                                bad.insert(why);
                            }
                        }
                    }
                    for why in bad {
                        self.problem(format!("section `{sname}`: {why} (Nastran's linear solids have no such formulation)"));
                    }
                    for (rule, elems) in rules {
                        let pid = next_pid;
                        next_pid += 1;
                        self.properties.insert(pid, (sname.as_str(), Property::Solid { mid, rule }));
                        for e in elems {
                            self.element_props.insert(e, (pid, Extra::None));
                        }
                    }
                }
                SectionKind::Shell { thickness, offset, points } => {
                    if sec.formulation.integration == Integration::Reduced {
                        self.problem(format!("section `{sname}`: reduced integration of shells (Nastran's shells have one formulation)"));
                    }
                    if points < 2 {
                        self.problem(format!(
                            "section `{sname}`: one integration point through the thickness (no bending stiffness; Nastran integrates a linear section exactly)"
                        ));
                    }
                    let (mid, mcid) = match ids {
                        Some(MaterialIds { isotropic: Some(m), .. }) => (m, None),
                        Some(MaterialIds { shell: Some(m), .. }) => {
                            let flat = elements.iter().all(|e| model.element(*e).is_some_and(|el| self.in_constant_z(&el.nodes, el.kind)));
                            if !flat {
                                self.problem(format!(
                                    "section `{sname}`: orthotropic shells that do not lie in planes of constant Z (the material axes are the global ones)"
                                ));
                            }
                            let csys = *self.material_csys.get_or_insert(1);
                            (m, Some(csys))
                        }
                        _ => continue,
                    };
                    let pid = next_pid;
                    next_pid += 1;
                    self.properties.insert(pid, (sname.as_str(), Property::Shell { mid, thickness }));
                    // The nodes lie on the reference surface; Nastran's offset runs from them to
                    // the mid-surface along the element normal.
                    let zoffs = (offset != 0.0).then_some(-offset * thickness);
                    for &e in elements {
                        self.element_props.insert(e, (pid, Extra::Shell { offset: zoffs, mcid }));
                    }
                }
                SectionKind::Beam { profile, orientation } => {
                    let Some(mid) = ids.and_then(|i| i.isotropic) else { continue };
                    let pid = next_pid;
                    next_pid += 1;
                    self.properties.insert(pid, (sname.as_str(), Property::Beam { mid, profile }));
                    for &e in elements {
                        self.element_props.insert(e, (pid, Extra::Beam { orientation }));
                    }
                }
                SectionKind::Rod { area } => {
                    let Some(mid) = ids.and_then(|i| i.isotropic) else { continue };
                    let pid = next_pid;
                    next_pid += 1;
                    self.properties.insert(pid, (sname.as_str(), Property::Rod { mid, area }));
                    for &e in elements {
                        self.element_props.insert(e, (pid, Extra::None));
                    }
                }
            }
        }
        if next_mid > MAX_LABEL || next_pid > MAX_LABEL {
            return Err(Error::Limit("too many materials or properties".into()));
        }
        Ok(())
    }

    /// Whether a shell element's corners lie in a plane of constant Z.
    fn in_constant_z(&self, nodes: &[u32], kind: ElementKind) -> bool {
        let corners = if matches!(kind, ElementKind::Tri3 | ElementKind::Tri6) { 3 } else { 4 };
        let z: Vec<f64> = nodes.iter().take(corners).filter_map(|n| self.model.node(*n)).map(|n| n.xyz[2]).collect();
        z.len() == corners && z.iter().all(|v| *v == z[0])
    }

    /// Rigid element ids and the degrees of freedom couplings make dependent.
    fn plan_couplings(&mut self) -> Result<()> {
        let model = self.model;
        let mut next = model.elements().keys().next_back().copied().unwrap_or(0);
        for (name, c) in model.couplings() {
            next = next
                .checked_add(1)
                .filter(|n| *n <= MAX_LABEL)
                .ok_or_else(|| Error::Limit(format!("coupling `{name}`: no element identification number left below {}", MAX_LABEL as u64 + 1)))?;
            self.rigid.push((next, name.as_str(), c));
            let mask = mask_of(&c.dofs.list());
            let dependents: Vec<(u32, u8)> = match c.kind {
                CouplingKind::Kinematic => self.r.sets.get(&c.set).map(|s| s.nodes.iter().map(|n| (*n, mask)).collect()).unwrap_or_default(),
                CouplingKind::Distributing => self.r.reference_point_nodes.get(&c.reference_point).map(|n| vec![(*n, mask)]).unwrap_or_default(),
            };
            for (node, m) in dependents {
                match self.dependent.get(&node).copied() {
                    Some((had, other)) if had & m != 0 => {
                        let p = format!(
                            "couplings `{other}` and `{name}` both make degrees of freedom {} of node {node} follow another node",
                            component(had & m)
                        );
                        self.problem(p);
                    }
                    Some((had, other)) => {
                        self.dependent.insert(node, (had | m, other));
                    }
                    None => {
                        self.dependent.insert(node, (m, name));
                    }
                }
            }
        }
        Ok(())
    }

    /// One constraint set per distinct step constraint, checked against coupled nodes.
    fn plan_constraints(&mut self) -> Result<()> {
        let model = self.model;
        let r = self.r;
        // Conflicts, one problem per pair of definitions: the nodes they meet on.
        let mut clashes: BTreeMap<(&str, &str, &str), BTreeSet<u32>> = BTreeMap::new();
        let mut on_dependent: BTreeMap<(&str, &str), BTreeSet<u32>> = BTreeMap::new();
        for i in 0..self.steps.len() {
            let Some(step) = self.steps.get(i).copied() else { continue };
            let modes = matches!(step.procedure, Procedure::Frequency { .. });
            let mut set: Constraints = BTreeMap::new();
            let mut by: BTreeMap<(u32, u8), &str> = BTreeMap::new();
            for name in &step.boundary_conditions {
                let Some(bc) = model.boundary_condition(name) else { continue };
                for &node in r.target_nodes(bc.target()) {
                    for dof in bc.dofs().list() {
                        let value = if modes { Some(0.0) } else { bc.value(dof) };
                        let Some(value) = value else { continue };
                        let slot = set.entry(node).or_default();
                        let Some(slot) = usize::from(dof).checked_sub(1).and_then(|i| slot.get_mut(i)) else { continue };
                        match *slot {
                            Some(v) if v != value => {
                                let other = by.get(&(node, dof)).copied().unwrap_or_default();
                                clashes.entry((step.name.as_str(), other, name.as_str())).or_default().insert(node);
                            }
                            _ => {
                                *slot = Some(value);
                                by.insert((node, dof), name);
                            }
                        }
                        if let Some((mask, coupling)) = self.dependent.get(&node).copied()
                            && mask & (1 << (dof - 1)) != 0
                        {
                            on_dependent.entry((name.as_str(), coupling)).or_default().insert(node);
                        }
                    }
                }
            }
            if set.is_empty() {
                self.step_spc.push(None);
                continue;
            }
            let sid = match self.spc_sets.iter().find(|(_, s)| *s == set) {
                Some((sid, _)) => *sid,
                None => {
                    let sid = self.sid()?;
                    self.spc_sets.push((sid, set));
                    sid
                }
            };
            self.step_spc.push(Some(sid));
        }
        for ((step, a, b), nodes) in clashes {
            self.problem(format!(
                "step `{step}`: boundary conditions `{a}` and `{b}` prescribe different values at nodes {} (Nastran takes one value per degree of freedom)",
                labels(&nodes)
            ));
        }
        for ((bc, coupling), nodes) in on_dependent {
            self.problem(format!(
                "boundary condition `{bc}` constrains nodes {} whose motion coupling `{coupling}` prescribes (Nastran cannot constrain a dependent degree of freedom)",
                labels(&nodes)
            ));
        }
        Ok(())
    }

    /// Load set ids for the static steps' loads, and their combinations.
    fn plan_loads(&mut self) -> Result<()> {
        let model = self.model;
        let r = self.r;
        let mut gravity = 0usize;
        for i in 0..self.steps.len() {
            let Some(step) = self.steps.get(i).copied() else { continue };
            if !matches!(step.procedure, Procedure::Static {}) || step.loads.is_empty() {
                self.step_load.push(None);
                continue;
            }
            let mut sids = Vec::new();
            for name in &step.loads {
                if let Some(sid) = self.load_sids.get(name.as_str()) {
                    sids.push(*sid);
                    continue;
                }
                let Some(load) = model.load(name) else { continue };
                match load {
                    Load::BodyForce { .. } => {
                        self.problem(format!("load `{name}`: a body force (Nastran has no force-per-volume load for these elements)"));
                    }
                    Load::Gravity { set: Some(s), .. } if r.sets.get(s).map(|s| s.elements.len()) != Some(model.elements().len()) => {
                        self.problem(format!("load `{name}`: gravity on part of the model (set `{s}`); Nastran's gravity acts on all of it"));
                    }
                    Load::Gravity { .. } => gravity += 1,
                    _ => {}
                }
                let sid = self.sid()?;
                self.load_sids.insert(name.as_str(), sid);
                sids.push(sid);
            }
            let case = match sids.as_slice() {
                [] => None,
                [one] => Some(*one),
                _ => match self.combinations.iter().find(|(_, c)| *c == sids) {
                    Some((sid, _)) => Some(*sid),
                    None => {
                        let sid = self.sid()?;
                        self.combinations.push((sid, sids));
                        Some(sid)
                    }
                },
            };
            self.step_load.push(case);
        }
        if gravity > MAX_GRAV {
            self.problem(format!("{gravity} gravity loads (a run selects at most {MAX_GRAV})"));
        }
        Ok(())
    }

    fn emit(&self) -> Result<String> {
        let sol = self.solution().map_err(|what| Error::Unsupported { format: FORMAT, what })?;
        let mut out = String::new();
        let mut rounded = 0usize;
        out.push_str("$ Nastran bulk data written by ModelCraft\n");
        out.push_str(&format!("$ units: {}\n", self.model.units().label()));
        out.push_str(&format!("SOL {sol}\nCEND\nTITLE = ModelCraft model\nECHO = NONE\n"));
        for (i, step) in self.steps.iter().enumerate() {
            out.push_str(&format!("SUBCASE {}\n", i + 1));
            let title: String = format!("  SUBTITLE = {}", step.name).chars().take(72).collect();
            out.push_str(&title);
            out.push('\n');
            if let Some(Some(sid)) = self.step_spc.get(i) {
                out.push_str(&format!("  SPC = {sid}\n"));
            }
            if let Some(Some(sid)) = self.step_load.get(i) {
                out.push_str(&format!("  LOAD = {sid}\n"));
            }
            if let Some((sid, _)) = self.methods.iter().find(|(_, s)| *s == i) {
                out.push_str(&format!("  METHOD = {sid}\n"));
            }
            for v in &step.output {
                let request = match v {
                    modelcraft_model::OutputVariable::Displacement => "DISPLACEMENT",
                    modelcraft_model::OutputVariable::ReactionForce => "SPCFORCES",
                    modelcraft_model::OutputVariable::Stress => "STRESS",
                    modelcraft_model::OutputVariable::Strain => "STRAIN",
                };
                out.push_str(&format!("  {request} = ALL\n"));
            }
        }
        out.push_str("BEGIN BULK\n");
        let mut put = |card: Card, out: &mut String| card.write(out, &mut rounded);
        put(Card::new("PARAM").text("POST").int(-1), &mut out)?;
        put(Card::new("PARAM").text("AUTOSPC").text("YES"), &mut out)?;
        if sol == 103 {
            // Consistent (coupled) mass, as the other formats use by default.
            put(Card::new("PARAM").text("COUPMASS").int(1), &mut out)?;
        }

        comment(&mut out, "nodes");
        for (label, node) in self.model.nodes() {
            put(Card::new("GRID").int(*label).blank().reals(node.xyz), &mut out)?;
        }
        if !self.model.reference_points().is_empty() {
            comment(&mut out, "reference points");
            for (name, rp) in self.model.reference_points() {
                comment(&mut out, &format!("reference point {name}: GRID {}", rp.label));
                put(Card::new("GRID").int(rp.label).blank().reals(rp.xyz), &mut out)?;
            }
        }
        if let Some(cid) = self.material_csys {
            comment(&mut out, "material axes of orthotropic shells: the basic axes");
            put(Card::new("CORD2R").int(cid).blank().reals([0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0]), &mut out)?;
        }

        comment(&mut out, "materials");
        for (name, ids) in &self.materials {
            let Some(m) = self.model.material(name) else { continue };
            let rho = m.density;
            match m.elastic {
                Some(Elastic::Isotropic { young, poisson }) => {
                    if let Some(mid) = ids.isotropic {
                        comment(&mut out, &format!("material {name}: MAT1 {mid}"));
                        put(Card::new("MAT1").int(mid).real(young).blank().real(poisson).opt_real(rho), &mut out)?;
                    }
                }
                Some(Elastic::EngineeringConstants { young, poisson, shear }) => {
                    if let Some(mid) = ids.solid {
                        let c = orthotropic_stiffness(young, poisson, shear)
                            .ok_or_else(|| Error::Limit(format!("material `{name}`: its stiffness is not a finite number")))?;
                        let [[c11, c12, c13], [c22, c23, c33], [g12, g23, g13]] = c;
                        comment(&mut out, &format!("material {name}: MAT9 {mid}"));
                        let card = Card::new("MAT9")
                            .int(mid)
                            .reals([c11, c12, c13, 0.0, 0.0, 0.0, c22, c23, 0.0, 0.0, 0.0, c33, 0.0, 0.0, 0.0, g12, 0.0, 0.0, g23, 0.0, g13])
                            .opt_real(rho);
                        put(card, &mut out)?;
                    }
                    if let Some(mid) = ids.shell {
                        comment(&mut out, &format!("material {name}: MAT8 {mid}"));
                        let [e1, e2, _] = young;
                        let [g12, g13, g23] = shear;
                        put(Card::new("MAT8").int(mid).reals([e1, e2, poisson[0], g12, g13, g23]).opt_real(rho), &mut out)?;
                    }
                }
                None => {}
            }
        }

        comment(&mut out, "properties");
        for (pid, (section, p)) in &self.properties {
            let card = match p {
                Property::Solid { mid, rule: (network, scheme) } => {
                    Card::new("PSOLID").int(*pid).int(*mid).blank().opt_text(*network).blank().opt_text(*scheme)
                }
                Property::Shell { mid, thickness } => Card::new("PSHELL").int(*pid).int(*mid).real(*thickness).int(*mid).blank().int(*mid),
                Property::Beam { mid, profile } => {
                    let (shape, dims) = beam_dimensions(*profile);
                    Card::new("PBEAML").int(*pid).int(*mid).blank().text(shape).end_line().reals(dims)
                }
                Property::Rod { mid, area } => Card::new("PROD").int(*pid).int(*mid).real(*area),
            };
            comment(&mut out, &format!("section {section}: {} {pid}", card_name(p)));
            put(card, &mut out)?;
        }

        comment(&mut out, "elements");
        for (label, el) in self.model.elements() {
            let Some(&(pid, extra)) = self.element_props.get(label) else { continue };
            let g = grids(el.kind, &el.nodes);
            let mut card = Card::new(element_card(el.kind)).int(*label).int(pid);
            match (el.kind, extra) {
                (ElementKind::Beam2, Extra::Beam { orientation }) => {
                    let v = self.beam_vector(&el.nodes, orientation);
                    card = card.ints(g).reals(v);
                }
                (ElementKind::Quad8, Extra::Shell { offset, mcid }) => {
                    card = card.ints(g).blank().blank().blank().blank().opt_int(mcid).opt_real(offset);
                }
                (_, Extra::Shell { offset, mcid }) => card = card.ints(g).opt_int(mcid).opt_real(offset),
                _ => card = card.ints(g),
            }
            put(card, &mut out)?;
        }

        if !self.rigid.is_empty() {
            comment(&mut out, "couplings");
        }
        for (eid, name, c) in &self.rigid {
            let rp = self.r.reference_point_nodes.get(&c.reference_point).copied().unwrap_or_default();
            let nodes = self.r.sets.get(&c.set).map(|s| s.nodes.as_slice()).unwrap_or_default();
            let dofs = component(mask_of(&c.dofs.list()));
            let card = match c.kind {
                CouplingKind::Kinematic => {
                    comment(&mut out, &format!("coupling {name}: RBE2 {eid}"));
                    Card::new("RBE2").int(*eid).int(rp).int(dofs).ints(nodes.iter().copied())
                }
                CouplingKind::Distributing => {
                    comment(&mut out, &format!("coupling {name}: RBE3 {eid}"));
                    Card::new("RBE3").int(*eid).blank().int(rp).int(dofs).real(1.0).int(123).ints(nodes.iter().copied())
                }
            };
            put(card, &mut out)?;
        }

        if !self.spc_sets.is_empty() {
            comment(&mut out, "boundary conditions");
        }
        for (sid, set) in &self.spc_sets {
            for name in self.steps.iter().zip(&self.step_spc).filter(|(_, s)| **s == Some(*sid)).map(|(st, _)| &st.name) {
                comment(&mut out, &format!("step {name}: SPC {sid}"));
            }
            let mut held: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
            let mut prescribed: Vec<(u32, u8, f64)> = Vec::new();
            for (node, values) in set {
                let zero = values.iter().enumerate().filter(|(_, v)| **v == Some(0.0)).fold(0u8, |m, (i, _)| m | (1 << i));
                if zero != 0 {
                    held.entry(zero).or_default().push(*node);
                }
                // Nonzero values: one entry per value, with every degree of freedom it applies to.
                let mut by_value: Vec<(f64, u8)> = Vec::new();
                for (i, v) in values.iter().enumerate() {
                    let Some(v) = *v else { continue };
                    if v == 0.0 {
                        continue;
                    }
                    match by_value.iter_mut().find(|(w, _)| *w == v) {
                        Some((_, m)) => *m |= 1 << i,
                        None => by_value.push((v, 1 << i)),
                    }
                }
                prescribed.extend(by_value.into_iter().map(|(v, m)| (*node, m, v)));
            }
            for (mask, nodes) in &held {
                for chunk in nodes.chunks(SPC1_GRIDS) {
                    put(Card::new("SPC1").int(*sid).int(component(*mask)).ints(chunk.iter().copied()), &mut out)?;
                }
            }
            for (node, mask, value) in prescribed {
                put(Card::new("SPC").int(*sid).int(node).int(component(mask)).real(value), &mut out)?;
            }
        }

        if !self.load_sids.is_empty() {
            comment(&mut out, "loads");
        }
        for (name, sid) in &self.load_sids {
            let Some(load) = self.model.load(name) else { continue };
            comment(&mut out, &format!("load {name}: {} {sid}", load_card(load)));
            match load {
                Load::Force { .. } | Load::Traction { .. } | Load::Moment { .. } => {
                    let entry = if matches!(load, Load::Moment { .. }) { "MOMENT" } else { "FORCE" };
                    let values = self.r.nodal_loads.get(*name).map(Vec::as_slice).unwrap_or_default();
                    for (node, v) in values.iter().filter(|(_, v)| v.iter().any(|c| *c != 0.0)) {
                        put(Card::new(entry).int(*sid).int(*node).blank().real(1.0).reals(*v), &mut out)?;
                    }
                }
                Load::Pressure { set, magnitude } => {
                    let faces = self.r.sets.get(set).map(|s| s.faces.as_slice()).unwrap_or_default();
                    for f in faces {
                        if let Some(card) = self.pressure(*sid, *f, *magnitude) {
                            put(card, &mut out)?;
                        }
                    }
                }
                Load::Gravity { acceleration, .. } => {
                    put(Card::new("GRAV").int(*sid).blank().real(1.0).reals(*acceleration), &mut out)?;
                }
                Load::BodyForce { .. } => {}
            }
        }
        for (sid, loads) in &self.combinations {
            let mut card = Card::new("LOAD").int(*sid).real(1.0);
            for l in loads {
                card = card.real(1.0).int(*l);
            }
            put(card, &mut out)?;
        }

        if !self.methods.is_empty() {
            comment(&mut out, "eigenvalue extraction");
        }
        for (sid, i) in &self.methods {
            if let Some(Procedure::Frequency { modes, min_hz, max_hz }) = self.steps.get(*i).map(|s| s.procedure) {
                put(Card::new("EIGRL").int(*sid).opt_real(min_hz).opt_real(max_hz).int(modes), &mut out)?;
            }
        }
        out.push_str("ENDDATA\n");
        Ok(out)
    }

    /// A `PLOAD4` for one element face, positive pushing into the element.
    fn pressure(&self, sid: u32, f: modelcraft_model::ElementFace, magnitude: f64) -> Option<Card> {
        let el = self.model.element(f.element)?;
        let card = Card::new("PLOAD4").int(sid).int(f.element);
        if el.kind.dimension() == 2 {
            // Nastran pushes along the element normal; face 1 is the side the normal points to.
            let p = if f.face == 1 { -magnitude } else { magnitude };
            return Some(card.real(p));
        }
        let local = el.kind.faces().get(usize::from(f.face).checked_sub(1)?)?;
        let corners: Vec<usize> = local.iter().copied().filter(|&i| i < corner_count(el.kind)).collect();
        let g1 = el.nodes.get(*corners.first()?).copied()?;
        let card = card.real(magnitude).blank().blank().blank().int(g1);
        Some(match (el.kind, corners.len()) {
            // Tetrahedra: the corner off the face.
            (ElementKind::Tet4 | ElementKind::Tet10, _) => {
                let off = (0..4).find(|i| !corners.contains(i))?;
                card.int(el.nodes.get(off).copied()?)
            }
            // Quadrilateral faces: the corner diagonally opposite G1.
            (_, 4) => card.int(el.nodes.get(*corners.get(2)?).copied()?),
            // Triangular faces of wedges: G3 stays blank.
            _ => card,
        })
    }

    /// The `CBEAM` orientation vector: the profile's local 1 direction (the section's
    /// orientation) is Nastran's element z axis, so v = axis × orientation.
    fn beam_vector(&self, nodes: &[u32], o: [f64; 3]) -> [f64; 3] {
        let p = |i: usize| nodes.get(i).and_then(|n| self.model.node(*n)).map(|n| n.xyz).unwrap_or_default();
        let (a, b) = (p(0), p(1));
        let t = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        [t[1] * o[2] - t[2] * o[1], t[2] * o[0] - t[0] * o[2], t[0] * o[1] - t[1] * o[0]]
    }
}

fn corner_count(kind: ElementKind) -> usize {
    match kind {
        ElementKind::Tet4 | ElementKind::Tet10 => 4,
        ElementKind::Wedge6 => 6,
        ElementKind::Hex8 | ElementKind::Hex20 => 8,
        ElementKind::Tri3 | ElementKind::Tri6 => 3,
        ElementKind::Quad4 | ElementKind::Quad8 => 4,
        ElementKind::Rod2 | ElementKind::Beam2 => 2,
    }
}

/// `PBEAML`'s shape and dimensions. Nastran's sections have their height along the element y
/// axis and their width (flanges) along z, which [`Deck::beam_vector`] makes the profile's
/// local 1 direction.
fn beam_dimensions(p: BeamProfile) -> (&'static str, Vec<f64>) {
    match p {
        BeamProfile::Rectangle { width, height } => ("BAR", vec![width, height]),
        BeamProfile::Circle { radius } => ("ROD", vec![radius]),
        BeamProfile::Pipe { radius, thickness } if radius - thickness > 0.0 => ("TUBE", vec![radius, radius - thickness]),
        BeamProfile::Pipe { radius, .. } => ("ROD", vec![radius]),
        BeamProfile::I { height, width, web, flange } => ("I", vec![height, width, width, web, flange, flange]),
        BeamProfile::Box { width, height, thickness } => ("BOX", vec![width, height, thickness, thickness]),
    }
}

fn card_name(p: &Property) -> &'static str {
    match p {
        Property::Solid { .. } => "PSOLID",
        Property::Shell { .. } => "PSHELL",
        Property::Beam { .. } => "PBEAML",
        Property::Rod { .. } => "PROD",
    }
}

fn load_card(l: &Load) -> &'static str {
    match l {
        Load::Force { .. } | Load::Traction { .. } => "FORCE",
        Load::Moment { .. } => "MOMENT",
        Load::Pressure { .. } => "PLOAD4",
        Load::Gravity { .. } => "GRAV",
        Load::BodyForce { .. } => "-",
    }
}

/// The first few labels of a list, and how many more there are.
fn labels(nodes: &BTreeSet<u32>) -> String {
    const SHOWN: usize = 5;
    let shown: Vec<String> = nodes.iter().take(SHOWN).map(u32::to_string).collect();
    match nodes.len().saturating_sub(SHOWN) {
        0 => shown.join(", "),
        more => format!("{}, and {more} more", shown.join(", ")),
    }
}

/// A comment line, at most 80 columns.
fn comment(out: &mut String, text: &str) {
    let line: String = format!("$ {text}").chars().take(80).collect();
    out.push_str(&line);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orthotropic_stiffness_of_isotropic_constants_is_lames() {
        let (e, nu) = (210_000.0, 0.3);
        let g = e / (2.0 * (1.0 + nu));
        let c = orthotropic_stiffness([e; 3], [nu; 3], [g; 3]).unwrap();
        let k = e / ((1.0 + nu) * (1.0 - 2.0 * nu));
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9 * b.abs();
        assert!(close(c[0][0], k * (1.0 - nu)) && close(c[1][0], k * (1.0 - nu)) && close(c[1][2], k * (1.0 - nu)));
        assert!(close(c[0][1], k * nu) && close(c[0][2], k * nu) && close(c[1][1], k * nu));
        assert_eq!(c[2], [g; 3]);
    }

    #[test]
    fn hex20_mid_side_nodes_swap_blocks() {
        let nodes: Vec<u32> = (1..=20).collect();
        let g = grids(ElementKind::Hex20, &nodes);
        assert_eq!(&g[..12], &nodes[..12]);
        assert_eq!(&g[12..16], &[17, 18, 19, 20]);
        assert_eq!(&g[16..], &[13, 14, 15, 16]);
        for k in ElementKind::ALL {
            let mut sorted = order(k).to_vec();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..k.node_count()).collect::<Vec<_>>(), "{k:?} is a permutation");
        }
    }

    #[test]
    fn formulations_map_or_are_named() {
        let f = |integration, incompatible_modes, hybrid| Formulation { integration, incompatible_modes, hybrid };
        assert_eq!(solid_rule(ElementKind::Hex8, f(Integration::Full, false, false)), Ok((Some("TWO"), Some("FULL"))));
        assert_eq!(solid_rule(ElementKind::Hex8, f(Integration::Full, true, false)), Ok((Some("BUBBLE"), None)));
        assert_eq!(solid_rule(ElementKind::Hex20, f(Integration::Reduced, false, false)), Ok((Some("TWO"), Some("FULL"))));
        assert_eq!(solid_rule(ElementKind::Tet10, f(Integration::Full, false, false)), Ok((Some("THREE"), Some("FULL"))));
        assert!(solid_rule(ElementKind::Hex8, f(Integration::Reduced, false, false)).is_err());
        assert!(solid_rule(ElementKind::Tet4, f(Integration::Full, false, true)).is_err());
    }
}
