//! The model check and the resolved view.
//!
//! [`Model::check`] lists everything that keeps the model from being analysed (errors) or looks
//! wrong (warnings), each naming the item it is about. [`Model::resolve`] gives the view the deck
//! writers and the solver read: every set resolved against the current mesh, the section of every
//! element and the node label of every reference point; it fails when the check finds an error.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{BoundaryCondition, Elastic, Error, Load, Model, Procedure, Result, SectionKind, Target};

/// How serious a [`Diagnostic`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The model cannot be analysed until it is fixed.
    Error,
    /// The model can be analysed but something looks wrong.
    Warning,
}

/// One finding of the check.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// The item it is about, such as ``section `web` `` or `model`.
    pub subject: String,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{s}: {}: {}", self.subject, self.message)
    }
}

/// A set resolved against the mesh. Every list is sorted and has no repeats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSet {
    pub nodes: Vec<u32>,
    pub elements: Vec<u32>,
    pub faces: Vec<crate::ElementFace>,
}

impl ResolvedSet {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.elements.is_empty() && self.faces.is_empty()
    }
}

/// The model resolved against its mesh: what writers and the solver read besides the
/// definitions themselves.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    /// Every set, by name.
    pub sets: BTreeMap<String, ResolvedSet>,
    /// The section of every element.
    pub element_sections: BTreeMap<u32, String>,
    /// The node label of every reference point ([`Model::reference_point_nodes`]).
    pub reference_point_nodes: BTreeMap<String, u32>,
    /// The check's warnings (a resolved model has no errors).
    pub warnings: Vec<Diagnostic>,
}

impl Resolved {
    /// The nodes a boundary condition, force or moment acts on.
    pub fn target_nodes(&self, target: &Target) -> Vec<u32> {
        match target {
            Target::Set(s) => self.sets.get(s).map(|r| r.nodes.clone()).unwrap_or_default(),
            Target::ReferencePoint(r) => self.reference_point_nodes.get(r).map(|n| vec![*n]).unwrap_or_default(),
        }
    }
}

/// Labels listed in a message before it says how many more there are.
const LISTED: usize = 5;

fn first_labels(labels: &[u32]) -> String {
    let shown: Vec<String> = labels.iter().take(LISTED).map(u32::to_string).collect();
    match labels.len().saturating_sub(LISTED) {
        0 => shown.join(", "),
        more => format!("{}, and {more} more", shown.join(", ")),
    }
}

#[derive(Default)]
struct Diagnostics(Vec<Diagnostic>);

impl Diagnostics {
    fn error(&mut self, subject: impl Into<String>, message: impl Into<String>) {
        self.0.push(Diagnostic { severity: Severity::Error, subject: subject.into(), message: message.into() });
    }

    fn warning(&mut self, subject: impl Into<String>, message: impl Into<String>) {
        self.0.push(Diagnostic { severity: Severity::Warning, subject: subject.into(), message: message.into() });
    }
}

impl Model {
    /// Everything that keeps the model from being analysed or looks wrong, errors first, each
    /// naming its item.
    pub fn check(&self) -> Vec<Diagnostic> {
        self.analyze().1
    }

    /// Resolve the model for writing or solving. Fails with every error the check finds.
    pub fn resolve(&self) -> Result<Resolved> {
        let (mut resolved, diagnostics) = self.analyze();
        let errors: Vec<String> =
            diagnostics.iter().filter(|d| d.severity == Severity::Error).map(|d| format!("{}: {}", d.subject, d.message)).collect();
        if !errors.is_empty() {
            return Err(Error::Invalid(format!("the model is not ready for analysis: {}", errors.join("; "))));
        }
        resolved.warnings = diagnostics;
        Ok(resolved)
    }

    fn analyze(&self) -> (Resolved, Vec<Diagnostic>) {
        let mut d = Diagnostics::default();
        let mut resolved = Resolved::default();
        let units = self.units;

        if self.elements.is_empty() {
            d.error("model", "the mesh has no elements");
        }
        if units.length().is_none() {
            d.warning("model", "no unit system is chosen, so magnitudes are not checked");
        }

        // Sets.
        for (name, set) in &self.sets {
            let (r, missing) = self.resolve_definition(set);
            for m in missing {
                d.error(format!("set `{name}`"), m);
            }
            if r.is_empty() {
                if self.set_users(name).is_empty() {
                    d.warning(format!("set `{name}`"), "resolves to nothing on the current mesh");
                } else {
                    d.error(format!("set `{name}`"), "resolves to nothing on the current mesh");
                }
            }
            resolved.sets.insert(name.clone(), r);
        }
        let empty = crate::ResolvedSet::default();
        let set_of = |name: &str| resolved.sets.get(name).unwrap_or(&empty);

        // Materials: magnitudes for the unit system.
        for (name, m) in &self.materials {
            let subject = format!("material `{name}`");
            match m.elastic {
                Some(Elastic::Isotropic { young, .. }) => d.0.extend(units.check_modulus("Young's modulus", young).map(|w| warn(&subject, w))),
                Some(Elastic::EngineeringConstants { young, shear, .. }) => {
                    for (i, e) in young.iter().enumerate() {
                        d.0.extend(units.check_modulus(&format!("E{}", i + 1), *e).map(|w| warn(&subject, w)));
                    }
                    for (n, g) in ["G12", "G13", "G23"].iter().zip(shear) {
                        d.0.extend(units.check_modulus(n, g).map(|w| warn(&subject, w)));
                    }
                }
                None => {}
            }
            if let Some(rho) = m.density {
                d.0.extend(units.check_density(rho).map(|w| warn(&subject, w)));
            }
        }

        // Sections: element kinds, formulation, one section per element.
        let mut owner: BTreeMap<u32, &str> = BTreeMap::new();
        for (name, sec) in &self.sections {
            let subject = format!("section `{name}`");
            let r = set_of(&sec.set);
            if r.elements.is_empty() {
                d.error(&subject, format!("set `{}` has no elements", sec.set));
            }
            let fits = |k: crate::ElementKind| match sec.kind {
                SectionKind::Solid {} => k.dimension() == 3,
                SectionKind::Shell { .. } => k.dimension() == 2,
                SectionKind::Beam { .. } => k == crate::ElementKind::Beam2,
                SectionKind::Rod { .. } => k == crate::ElementKind::Rod2,
            };
            let mut wrong = Vec::new();
            let mut not_hex8 = Vec::new();
            let mut along = Vec::new();
            let mut taken: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
            for &e in &r.elements {
                let Some(el) = self.elements.get(&e) else { continue };
                if !fits(el.kind) {
                    wrong.push(e);
                }
                if sec.formulation.incompatible_modes && el.kind != crate::ElementKind::Hex8 {
                    not_hex8.push(e);
                }
                if let SectionKind::Beam { orientation, .. } = sec.kind
                    && self.parallel_to_axis(&el.nodes, orientation)
                {
                    along.push(e);
                }
                match owner.get(&e) {
                    Some(other) => taken.entry(other).or_default().push(e),
                    None => {
                        owner.insert(e, name);
                    }
                }
            }
            if !wrong.is_empty() {
                d.error(&subject, format!("a {} section cannot hold elements {}", sec.kind.name(), first_labels(&wrong)));
            }
            if !not_hex8.is_empty() {
                d.error(&subject, format!("incompatible modes apply to hex8 elements only, not elements {}", first_labels(&not_hex8)));
            }
            if !along.is_empty() {
                d.error(&subject, format!("the orientation vector is parallel to elements {}", first_labels(&along)));
            }
            for (other, labels) in taken {
                d.error(&subject, format!("elements {} are also in section `{other}`", first_labels(&labels)));
            }
            if self.materials.get(&sec.material).is_some_and(|m| m.elastic.is_none()) {
                d.error(&subject, format!("material `{}` has no elastic constants", sec.material));
            }
        }
        let unsectioned: Vec<u32> = self.elements.keys().filter(|e| !owner.contains_key(e)).copied().collect();
        if !unsectioned.is_empty() {
            d.error("model", format!("elements {} have no section", first_labels(&unsectioned)));
        }
        resolved.element_sections = owner.into_iter().map(|(e, s)| (e, s.to_string())).collect();

        // Mass: frequency steps and gravity need a density in every material in use.
        let needs_mass = self
            .steps
            .iter()
            .find(|s| matches!(s.procedure, Procedure::Frequency { .. }))
            .map(|s| format!("step `{}` extracts frequencies", s.name))
            .or_else(|| self.loads.iter().find(|(_, l)| matches!(l, Load::Gravity { .. })).map(|(n, _)| format!("load `{n}` is gravity")));
        if let Some(why) = needs_mass {
            let used: BTreeSet<&str> = self.sections.values().map(|s| s.material.as_str()).collect();
            for m in used {
                if self.materials.get(m).is_some_and(|m| m.density.is_none()) {
                    d.error(format!("material `{m}`"), format!("has no density, and {why}"));
                }
            }
        }

        // Reference points and couplings.
        match self.reference_point_nodes() {
            Ok(nodes) => resolved.reference_point_nodes = nodes,
            Err(e) => d.error("model", e.to_string()),
        }
        for (name, c) in &self.couplings {
            if set_of(&c.set).nodes.is_empty() {
                d.error(format!("coupling `{name}`"), format!("set `{}` has no nodes", c.set));
            }
        }

        // Nodes with rotational degrees of freedom: those of shells and beams, and reference points.
        let rotational: BTreeSet<u32> = self.elements.values().filter(|e| e.kind.dimension() < 3).flat_map(|e| e.nodes.iter().copied()).collect();
        let has_rotations = |t: &Target| match t {
            Target::ReferencePoint(_) => true,
            Target::Set(s) => set_of(s).nodes.iter().any(|n| rotational.contains(n)),
        };

        // Boundary conditions.
        for (name, bc) in &self.boundary_conditions {
            let subject = format!("boundary condition `{name}`");
            if let Target::Set(s) = bc.target()
                && set_of(s).nodes.is_empty()
            {
                d.error(&subject, format!("set `{s}` has no nodes"));
            }
            if let BoundaryCondition::Displacement { values, target } = bc
                && values.get(3..).is_some_and(|r| r.iter().any(Option::is_some))
                && !has_rotations(target)
            {
                d.warning(&subject, format!("{target} has only solid-element nodes, which have no rotations to prescribe"));
            }
        }

        // Loads.
        for (name, load) in &self.loads {
            let subject = format!("load `{name}`");
            match load {
                Load::Force { target: Target::Set(s), .. } if set_of(s).nodes.is_empty() => d.error(&subject, format!("set `{s}` has no nodes")),
                Load::Moment { target, .. } if !has_rotations(target) => {
                    d.error(&subject, format!("{target} has no nodes with rotations; apply the moment to a reference point coupled to it"))
                }
                Load::Pressure { set, .. } if set_of(set).faces.is_empty() => {
                    d.error(&subject, format!("set `{set}` has no element faces to press on"))
                }
                Load::BodyForce { set, .. } if set_of(set).elements.is_empty() => d.error(&subject, format!("set `{set}` has no elements")),
                Load::Gravity { acceleration, set } => {
                    let g = acceleration.iter().map(|c| c * c).sum::<f64>().sqrt();
                    d.0.extend(units.check_gravity(g).map(|w| warn(&subject, w)));
                    if let Some(s) = set
                        && set_of(s).elements.is_empty()
                    {
                        d.error(&subject, format!("set `{s}` has no elements"));
                    }
                }
                _ => {}
            }
        }

        // Steps and the activity table.
        if self.steps.is_empty() {
            d.error("model", "there is no analysis step");
        }
        for s in &self.steps {
            let subject = format!("step `{}`", s.name);
            match s.procedure {
                Procedure::Static {} => {
                    let prescribes_motion = s
                        .boundary_conditions
                        .iter()
                        .filter_map(|b| self.boundary_conditions.get(b))
                        .any(|b| (1..=6).any(|dof| b.value(dof).is_some_and(|v| v != 0.0)));
                    if s.loads.is_empty() && !prescribes_motion {
                        d.warning(&subject, "nothing loads the model in this step");
                    }
                }
                Procedure::Frequency { .. } => {
                    if !s.loads.is_empty() {
                        d.warning(&subject, "loads have no effect on natural frequencies");
                    }
                }
            }
            if s.boundary_conditions.is_empty() {
                d.warning(&subject, "no boundary condition is active, so the model is free to move as a rigid body");
            }
        }
        let active_bcs: BTreeSet<&String> = self.steps.iter().flat_map(|s| &s.boundary_conditions).collect();
        let active_loads: BTreeSet<&String> = self.steps.iter().flat_map(|s| &s.loads).collect();
        for n in self.boundary_conditions.keys().filter(|n| !active_bcs.contains(n)) {
            d.warning(format!("boundary condition `{n}`"), "is not active in any step");
        }
        for n in self.loads.keys().filter(|n| !active_loads.contains(n)) {
            d.warning(format!("load `{n}`"), "is not active in any step");
        }

        let mut out = d.0;
        out.sort_by_key(|a| a.severity);
        (resolved, out)
    }

    /// Whether a vector is parallel to the axis of a two-node element (or the element has no
    /// length).
    fn parallel_to_axis(&self, nodes: &[u32], v: [f64; 3]) -> bool {
        let [Some(a), Some(b)] = [nodes.first(), nodes.get(1)].map(|n| n.and_then(|n| self.nodes.get(n))) else { return false };
        let t = [b.xyz[0] - a.xyz[0], b.xyz[1] - a.xyz[1], b.xyz[2] - a.xyz[2]];
        let c = [t[1] * v[2] - t[2] * v[1], t[2] * v[0] - t[0] * v[2], t[0] * v[1] - t[1] * v[0]];
        let norm = |x: [f64; 3]| (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
        !(norm(c) > 1e-9 * norm(t) * norm(v))
    }
}

fn warn(subject: &str, message: String) -> Diagnostic {
    Diagnostic { severity: Severity::Warning, subject: subject.to_string(), message }
}
