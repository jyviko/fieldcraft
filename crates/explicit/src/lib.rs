//! ModelCraft's explicit dynamics solver: transient response of a model's explicit dynamic step
//! by central-difference time integration.
//!
//! The method is the explicit central-difference scheme of T. Belytschko, W. K. Liu, B. Moran,
//! K. Elkhodary, *Nonlinear Finite Elements for Continua and Structures*, 2nd ed., Wiley 2014
//! (ch. 6, the flowchart of Box 6.1): with a diagonal (lumped) mass M,
//! a = M⁻¹(f_ext − f_int), v^{n+½} = v^{n−½} + Δt^n a^n, u^{n+1} = u^n + Δt^{n+½} v^{n+½},
//! so a time increment is one pass over the elements and a division per degree of freedom.
//!
//! - **System** ([`system`]): the model is resolved; one-point hexahedra and nodal-pressure
//!   tetrahedra with their materials, row-sum lumped masses, prescribed velocities, rigid walls,
//!   initial velocities and the constant load vector of the step.
//! - **Elements** ([`elements`]): the uniform-strain hexahedron with Flanagan–Belytschko
//!   hourglass control (viscous or stiffness), the Bonet–Burton average nodal pressure
//!   tetrahedron, von Neumann–Richtmyer/Wilkins bulk viscosity, and the element critical time
//!   increments.
//! - **Integration** ([`integrate`]): the time loop, kinematic constraints, energy accounting
//!   with a balance check, history samples and field frames ([`result`]).
//!
//! This first version is geometrically linear (small displacements and strains; the elements'
//! gradients are those of the undeformed mesh) with linear elastic materials, which is what the
//! analytic verification problems need. Large deformation kinematics with objective stress rates
//! and inelastic materials come next (docs/solvers/literature.md, Part F, X2).
//!
//! Everything is sequential, with elements in label order and no parallel reductions, so the
//! same model gives the same numbers bit for bit; sizes, increments and output are capped by
//! [`Limits`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
// Small fixed-size vector kernels read as the textbook formulas with index loops.
#![allow(clippy::needless_range_loop)]

pub mod elements;
pub mod integrate;
pub mod result;
pub mod system;

pub use integrate::solve_explicit;
pub use result::{ControllingElement, Energies, ExplicitResult, Frame, History};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Model(#[from] modelcraft_model::Error),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not supported yet: {0}")]
    NotSupported(String),
    /// An element kernel failed (inverted geometry, …).
    #[error("element {label}: {source}")]
    Element { label: u32, source: modelcraft_fem::Error },
    #[error("limit reached: {0}")]
    Limit(String),
    /// The solution stopped being finite.
    #[error("the solution became unstable: {0}")]
    Unstable(String),
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

/// Caps on the size and length of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Degrees of freedom of the mesh.
    pub max_dofs: usize,
    /// Time increments; the step's own cap applies too, whichever is lower.
    pub max_increments: u64,
    /// Numbers kept in field frames and histories together (8 bytes each).
    pub max_output_values: usize,
}

impl Limits {
    pub const MAX_DOFS: usize = 6_000_000;
    pub const MAX_INCREMENTS: u64 = modelcraft_model::steps::MAX_INCREMENTS;
    pub const MAX_OUTPUT_VALUES: usize = 50_000_000;
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_dofs: Limits::MAX_DOFS, max_increments: Limits::MAX_INCREMENTS, max_output_values: Limits::MAX_OUTPUT_VALUES }
    }
}
