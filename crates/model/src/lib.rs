//! The solver-neutral finite element model.
//!
//! A [`Model`] holds what every solver deck describes: nodes and elements addressed by the
//! positive integer labels the decks use. Materials, sections, sets, boundary conditions, loads
//! and steps join it as their tasks land (plan/architecture.md §4). Solver formats (Abaqus input,
//! Nastran bulk data, LS-DYNA keyword) translate to and from this model; nothing here knows any
//! of them.
//!
//! The model is unit-consistent: it stores numbers in whatever consistent system the user chose
//! (for example mm, tonne, s, N, MPa) and never converts them.
//!
//! Every mutation validates its input, because labels and coordinates come from files, scripts
//! and the control channel: a bad value is an [`Error`], never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod mesh;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use mesh::{Element, ElementKind, Node, check_coordinates};

/// The largest node or element label. Nastran's GRID and element ids stop at 99 999 999 (eight
/// digits in a small field), the tightest of the three formats.
pub const MAX_LABEL: u32 = 99_999_999;

/// Caps on model size, so hostile input cannot exhaust memory.
pub const MAX_NODES: usize = 50_000_000;
pub const MAX_ELEMENTS: usize = 50_000_000;

/// Coordinates beyond this magnitude are rejected (1e9 model units).
pub const MAX_COORD: f64 = 1e9;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("limit reached: {0}")]
    Limit(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The finite element model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    nodes: BTreeMap<u32, Node>,
    elements: BTreeMap<u32, Element>,
}
