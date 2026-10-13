//! The solver-neutral finite element model.
//!
//! A [`Model`] holds what every solver deck describes, in no solver's vocabulary:
//! - the mesh: nodes and elements addressed by the positive integer labels the decks use
//!   ([`mesh`]), and how the mesh lies on the geometry ([`geometry`]: each element, element face,
//!   element edge and node tagged with the persistent name of the body, face, edge or vertex it
//!   was meshed on);
//! - named sets ([`sets`]) defined by geometry names or explicit labels, resolved to nodes,
//!   elements and element faces against the current mesh, so they survive remeshing;
//! - materials and the unit system ([`materials`], [`units`]), sections with their element
//!   formulation ([`sections`]), reference points and couplings ([`constraints`]);
//! - boundary conditions ([`bcs`]), loads ([`loads`]) and steps with an explicit table of which
//!   boundary conditions and loads are active in each step ([`steps`]);
//! - the check that names every missing or bad item, and the resolved view the deck writers and
//!   the solver read ([`check`]).
//!
//! Solver formats (Abaqus input, Nastran bulk data, LS-DYNA keyword) translate to and from this
//! model; nothing here knows any of them.
//!
//! The model is unit-consistent: it stores numbers in whatever consistent system the user chose
//! (for example mm, tonne, s, N, MPa), never converts them, and only warns when a magnitude looks
//! wrong for the chosen system.
//!
//! Every mutation validates its input, because labels, names and numbers come from files, scripts
//! and the control channel: a bad value is an [`Error`], never a panic. Definitions that refer to
//! other definitions (a section to its set and material, a load to its set) must refer to ones
//! that exist, and a definition still referred to cannot be deleted. What depends on the mesh (a
//! set's geometry names and labels) is checked when the model is resolved, because remeshing
//! replaces the mesh under the definitions.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod bcs;
pub mod check;
pub mod constraints;
pub mod geometry;
pub mod loads;
pub mod materials;
mod mesh;
pub mod sections;
pub mod sets;
pub mod steps;
pub mod units;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use bcs::{BoundaryCondition, Dofs, Symmetry, Target};
pub use check::{Diagnostic, Resolved, ResolvedSet, Severity};
pub use constraints::{Coupling, CouplingKind, ReferencePoint};
pub use geometry::{Classification, Entity, GeometryRef};
pub use loads::Load;
pub use materials::{Elastic, Material};
pub use mesh::{Element, ElementFace, ElementKind, Node, check_coordinates};
pub use sections::{BeamProfile, Formulation, Integration, Section, SectionKind};
pub use sets::Set;
pub use steps::{OutputVariable, Procedure, Step};
pub use units::UnitSystem;

/// The largest node or element label. Nastran's GRID and element ids stop at 99 999 999 (eight
/// digits in a small field), the tightest of the three formats.
pub const MAX_LABEL: u32 = 99_999_999;

/// Caps on model size, so hostile input cannot exhaust memory.
pub const MAX_NODES: usize = 50_000_000;
pub const MAX_ELEMENTS: usize = 50_000_000;

/// Coordinates beyond this magnitude are rejected (1e9 model units).
pub const MAX_COORD: f64 = 1e9;

/// Most definitions of one kind (sets, materials, sections, …) a model holds.
pub const MAX_DEFINITIONS: usize = 100_000;

/// Longest name of a definition. Names are ASCII: a letter, then letters, digits, `_` or `-`, so
/// every deck format can carry them unquoted.
pub const MAX_NAME_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("limit reached: {0}")]
    Limit(String),
    /// A definition cannot be deleted while another refers to it.
    #[error("in use: {0}")]
    InUse(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The finite element model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    nodes: BTreeMap<u32, Node>,
    elements: BTreeMap<u32, Element>,
    #[serde(default)]
    classification: Classification,
    #[serde(default)]
    units: UnitSystem,
    #[serde(default)]
    sets: BTreeMap<String, Set>,
    #[serde(default)]
    materials: BTreeMap<String, Material>,
    #[serde(default)]
    sections: BTreeMap<String, Section>,
    #[serde(default)]
    reference_points: BTreeMap<String, ReferencePoint>,
    #[serde(default)]
    couplings: BTreeMap<String, Coupling>,
    #[serde(default)]
    boundary_conditions: BTreeMap<String, BoundaryCondition>,
    #[serde(default)]
    loads: BTreeMap<String, Load>,
    /// In analysis order.
    #[serde(default)]
    steps: Vec<Step>,
}

/// Check a definition's name: 1..=[`MAX_NAME_LEN`] ASCII characters, a letter first, then
/// letters, digits, `_` or `-`.
pub fn check_name(what: &str, name: &str) -> Result<()> {
    let mut chars = name.chars();
    let ok = name.len() <= MAX_NAME_LEN
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        let shown: String = name.chars().take(MAX_NAME_LEN).collect();
        Err(Error::Invalid(format!("{what} name `{shown}` must be 1 to {MAX_NAME_LEN} characters: a letter, then letters, digits, `_` or `-`")))
    }
}

/// Check that a value is a finite number, naming it in the error.
pub(crate) fn finite(what: &str, v: f64) -> Result<f64> {
    if v.is_finite() { Ok(v) } else { Err(Error::Invalid(format!("{what} must be a finite number, got {v}"))) }
}

/// Check that a value is a finite number above zero, naming it in the error.
pub(crate) fn positive(what: &str, v: f64) -> Result<f64> {
    if v > 0.0 && v.is_finite() { Ok(v) } else { Err(Error::Invalid(format!("{what} must be a positive number, got {v}"))) }
}

/// Add a new definition under a fresh, valid name.
fn insert_new<T>(map: &mut BTreeMap<String, T>, what: &str, name: &str, value: T) -> Result<()> {
    check_name(what, name)?;
    if map.contains_key(name) {
        return Err(Error::Invalid(format!("{what} `{name}` already exists")));
    }
    if map.len() >= MAX_DEFINITIONS {
        return Err(Error::Limit(format!("a model holds at most {MAX_DEFINITIONS} of each kind of definition")));
    }
    map.insert(name.to_string(), value);
    Ok(())
}

/// Replace an existing definition.
fn replace_existing<T>(map: &mut BTreeMap<String, T>, what: &str, name: &str, value: T) -> Result<()> {
    let slot = map.get_mut(name).ok_or_else(|| Error::NotFound(format!("{what} `{name}`")))?;
    *slot = value;
    Ok(())
}

/// Fail with [`Error::InUse`] when any referrer is listed.
fn not_in_use(what: &str, name: &str, referrers: Vec<String>) -> Result<()> {
    if referrers.is_empty() { Ok(()) } else { Err(Error::InUse(format!("{what} `{name}` is used by {}", referrers.join(", ")))) }
}

fn require<T>(map: &BTreeMap<String, T>, what: &str, name: &str) -> Result<()> {
    if map.contains_key(name) { Ok(()) } else { Err(Error::NotFound(format!("{what} `{name}`"))) }
}

impl Model {
    pub fn units(&self) -> UnitSystem {
        self.units
    }

    /// Label the unit system the model's numbers are in (nothing is converted).
    pub fn set_units(&mut self, units: UnitSystem) {
        self.units = units;
    }

    /// Counts of each kind of definition, for summaries.
    pub fn definition_counts(&self) -> [(&'static str, usize); 8] {
        [
            ("sets", self.sets.len()),
            ("materials", self.materials.len()),
            ("sections", self.sections.len()),
            ("reference_points", self.reference_points.len()),
            ("couplings", self.couplings.len()),
            ("boundary_conditions", self.boundary_conditions.len()),
            ("loads", self.loads.len()),
            ("steps", self.steps.len()),
        ]
    }
}
