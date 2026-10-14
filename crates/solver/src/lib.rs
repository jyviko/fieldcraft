//! ModelCraft's implicit solver: linear static and frequency analysis of a model's steps.
//!
//! The pipeline, for one step of a [`modelcraft_model::Model`]:
//! 1. **System** ([`system`]): the model is resolved; every section's elements become solid
//!    elements with their material's elasticity and the section's formulation
//!    ([`modelcraft_fem`] kernels); the step's boundary conditions give the prescribed degrees of
//!    freedom, its loads the load vector (forces and tractions as the model distributes them,
//!    consistent pressure loads, gravity and body forces).
//! 2. **Assembly** ([`sparse`]) on the node graph, elements in label order.
//! 3. **Ordering** ([`ordering`]): nested dissection of the nodes; the free degrees of freedom
//!    are numbered in that order and the prescribed ones eliminated (u_p moved to the right-hand
//!    side), with no penalty numbers.
//! 4. **Solution** ([`cholesky`]): sparse Cholesky; reactions K u − f at the prescribed degrees
//!    of freedom; stresses at the integration points, extrapolated and averaged at the nodes, and
//!    von Mises. Frequency analysis ([`eigen`]) runs shift-invert block Lanczos on the same
//!    kind of factorization.
//!
//! Everything is sequential and in a fixed order, so the same model gives the same numbers bit
//! for bit; sizes and iterations are capped by [`Limits`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
// Matrix kernels read as the textbook formulas with index loops.
#![allow(clippy::needless_range_loop)]

pub mod analysis;
pub mod cholesky;
pub mod constraints;
pub mod discrete;
pub mod eigen;
pub mod heat;
pub mod ordering;
pub mod sparse;
pub mod system;

pub use analysis::{
    ElementStress, FrequencyOptions, FrequencyResult, Location, MassKind, Mode, PointResult, StaticResult, Stats, StepResult, solve_frequency,
    solve_static,
};
pub use heat::{HeatResult, solve_heat};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Model(#[from] modelcraft_model::Error),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not supported yet: {0}")]
    NotSupported(String),
    /// An element kernel failed (inverted geometry, a formulation it lacks, …).
    #[error("element {label}: {source}")]
    Element { label: u32, source: modelcraft_fem::Error },
    /// The stiffness (or shifted matrix) is singular.
    #[error("{0}")]
    Singular(String),
    #[error("limit reached: {0}")]
    Limit(String),
    #[error("did not converge: {0}")]
    NotConverged(String),
    /// A broken internal invariant (a bug), reported instead of a panic.
    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A kernel error, naming the element.
    pub fn element(label: u32, source: modelcraft_fem::Error) -> Error {
        Error::Element { label, source }
    }
}

/// Caps on the size of a solution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Degrees of freedom of the mesh.
    pub max_dofs: usize,
    /// Entries of the Cholesky factor (8 bytes each); also bounds the Lanczos basis memory.
    pub max_factor_entries: usize,
    /// Lanczos vectors (iterations × block size).
    pub max_lanczos_vectors: usize,
}

impl Limits {
    pub const MAX_DOFS: usize = 6_000_000;
    pub const MAX_FACTOR_ENTRIES: usize = 400_000_000;
    pub const MAX_LANCZOS_VECTORS: usize = 3_000;
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_dofs: Limits::MAX_DOFS, max_factor_entries: Limits::MAX_FACTOR_ENTRIES, max_lanczos_vectors: Limits::MAX_LANCZOS_VECTORS }
    }
}
