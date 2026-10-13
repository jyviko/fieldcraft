//! Solver decks: the solver-neutral model written as Abaqus input, Nastran bulk data and LS-DYNA
//! keyword files. Each format is its own module (`abaqus`, `nastran`, `lsdyna`), written from the
//! vendor's public reference manual in our own words; examples and expected files are our own.
//! Writers read [`modelcraft_model::Model`] and its `resolve()` result, never another format.
//!
//! Writers produce text; saving it is the caller's business (the engine's `deck.write`), so the
//! crate stays free of the file system. [`FORMATS`] lists every writer by the name `deck.write`
//! takes.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod abaqus;
pub mod lsdyna;
pub mod nastran;

use std::collections::{BTreeMap, BTreeSet};

use modelcraft_model::{Diagnostic, Load, Model, Resolved};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// The model has something this format cannot express (named, with the format).
    #[error("not supported by {format}: {what}")]
    Unsupported { format: &'static str, what: String },
    /// The model is not ready to write (its check failed).
    #[error(transparent)]
    Model(#[from] modelcraft_model::Error),
    #[error("limit reached: {0}")]
    Limit(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A deck format.
pub struct Format {
    /// The name commands use (`abaqus`, …).
    pub name: &'static str,
    /// The usual file extension, without the dot.
    pub extension: &'static str,
    writer: fn(&Model, &Resolved) -> Result<String>,
}

/// A written deck: its text and the model check's warnings.
#[derive(Debug, Clone, PartialEq)]
pub struct Deck {
    pub text: String,
    pub warnings: Vec<Diagnostic>,
}

impl Format {
    /// Resolve the model once and write it in this format.
    pub fn write(&self, model: &Model) -> Result<Deck> {
        let resolved = model.resolve()?;
        let text = (self.writer)(model, &resolved)?;
        Ok(Deck { text, warnings: resolved.warnings })
    }

    /// Write only the named steps (in the model's order), or every step when `steps` is `None`;
    /// for runs that hold some steps only, such as a Nastran run (static or frequency steps).
    pub fn write_steps(&self, model: &Model, steps: Option<&[String]>) -> Result<Deck> {
        match steps {
            None => self.write(model),
            Some(names) => self.write(&with_steps(model, names)?),
        }
    }
}

/// The model with only the named steps, in its own order. Definitions the dropped steps used
/// stay (the check warns about the ones no step uses).
pub fn with_steps(model: &Model, names: &[String]) -> Result<Model> {
    if names.is_empty() {
        return Err(Error::Model(modelcraft_model::Error::Invalid("choose at least one step to write".into())));
    }
    if let Some(n) = names.iter().find(|n| model.step(n).is_none()) {
        let shown: String = n.chars().take(modelcraft_model::MAX_NAME_LEN).collect();
        return Err(Error::Model(modelcraft_model::Error::NotFound(format!("step `{shown}`"))));
    }
    let mut out = model.clone();
    let dropped: Vec<String> = model.steps().iter().map(|s| s.name.clone()).filter(|s| !names.contains(s)).collect();
    for name in dropped {
        out.remove_step(&name)?;
    }
    Ok(out)
}

/// Every deck format, one row per writer.
pub const FORMATS: &[Format] = &[
    Format { name: "abaqus", extension: "inp", writer: abaqus::write_resolved },
    Format { name: "nastran", extension: "bdf", writer: nastran::write_resolved },
];

/// The format called `name`.
pub fn format(name: &str) -> Option<&'static Format> {
    FORMATS.iter().find(|f| f.name == name)
}

/// Fail with [`Error::Unsupported`] naming every item, or succeed when there are none. Writers
/// collect everything their format cannot express and report it at once.
pub(crate) fn unsupported(format: &'static str, items: BTreeSet<String>) -> Result<()> {
    if items.is_empty() { Ok(()) } else { Err(Error::Unsupported { format, what: items.into_iter().collect::<Vec<_>>().join("; ") }) }
}

/// The concentrated loads of a set of active loads: the resolved nodal values of every force,
/// traction (degrees of freedom 1–3) and moment (4–6), summed per node and degree of freedom so
/// loads sharing a node add up; zero sums are left out.
pub fn concentrated_loads<'a>(model: &Model, r: &Resolved, active: impl IntoIterator<Item = &'a String>) -> BTreeMap<(u32, u8), f64> {
    let mut out: BTreeMap<(u32, u8), f64> = BTreeMap::new();
    for name in active {
        let first_dof = match model.load(name) {
            Some(Load::Moment { .. }) => 4,
            Some(Load::Force { .. } | Load::Traction { .. }) => 1,
            _ => continue,
        };
        for (node, v) in r.nodal_loads.get(name).map(Vec::as_slice).unwrap_or_default() {
            for (dof, c) in (first_dof..).zip(v) {
                *out.entry((*node, dof)).or_default() += c;
            }
        }
    }
    out.retain(|_, v| *v != 0.0);
    out
}
