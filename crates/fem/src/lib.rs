//! Finite element kernels shared by ModelCraft's structural solvers.
//!
//! Pure functions over element geometry and material constants, with no solver loop and no
//! global state, so the implicit solver (`modelcraft-solver`) and the explicit solver that follows
//! it use the same element technology:
//! - [`quadrature`]: Gauss–Legendre and tetrahedron rules;
//! - [`shape`]: shape functions of tet4, tet10, wedge6, hex8, hex20;
//! - [`material`]: isotropic and orthotropic elasticity matrices, von Mises stress;
//! - [`element`]: the element table, stiffness (full, reduced, incompatible modes), consistent
//!   and lumped mass, body forces, integration-point stresses and their extrapolation to nodes;
//! - [`face`]: consistent nodal forces of pressure on element faces;
//! - [`uniform`]: uniform-strain (one-point) hexahedron and tetrahedron kinematics, hourglass
//!   shape vectors and element frequency bounds for explicit dynamics;
//! - [`dense`]: the small dense linear algebra inside an element.
//!
//! Conventions: three translational degrees of freedom per node, ordered node by node; stress
//! and strain 6-vectors in the order xx, yy, zz, xy, xz, yz with engineering shear strains.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
// Matrix kernels over fixed small dimensions read as the textbook formulas with index loops.
#![allow(clippy::needless_range_loop)]

pub mod dense;
pub mod element;
pub mod face;
pub mod material;
pub mod quadrature;
pub mod shape;
pub mod uniform;

pub use element::{ELEMENT_TYPES, ElementType, Family, Formulation, PointStress, Solid, element_type};
pub use material::{Elasticity, elasticity, von_mises};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not supported yet: {0}")]
    NotSupported(String),
    /// An element's geometry is inverted or collapsed.
    #[error("{0}")]
    Degenerate(String),
    /// A matrix that must be positive definite is not.
    #[error("{0}")]
    Singular(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests;
