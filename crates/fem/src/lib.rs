//! Finite element kernels shared by ModelCraft's structural solvers.
//!
//! Pure functions over element geometry and material constants, with no solver loop and no
//! global state, so the implicit solver (`modelcraft-solver`) and the explicit solver that follows
//! it use the same element technology:
//! - [`dof`]: degree-of-freedom kinds, what elements declare, and the global numbering
//!   ([`DofLayout`]) every solver assembles on;
//! - [`quadrature`]: Gauss–Legendre and tetrahedron rules;
//! - [`basis`]: the shape-function library of every family (lines, triangles, quadrilaterals,
//!   tetrahedra, wedges, pyramids, hexahedra) with first and second derivatives and their
//!   physical gradients, Hessians and Laplacians; [`jet`] is the automatic differentiation behind
//!   the second derivatives;
//! - [`shape`]: the hand-written first-order functions of tet4, tet10, wedge6, hex8, hex20 that
//!   the solid kernels evaluate;
//! - [`material`]: isotropic and orthotropic elasticity matrices, von Mises stress;
//! - [`element`]: the element table, stiffness (full, reduced, incompatible modes), consistent
//!   and lumped mass, body forces, integration-point stresses and their extrapolation to nodes;
//! - [`plane`]: 2D elements (plane stress, plane strain, axisymmetric) on tri3, tri6, quad4 and
//!   quad8, with two displacement components per node;
//! - [`shell`]: flat MITC4 quadrilateral and MITC3 triangle shells with a drilling rotation, six
//!   degrees of freedom per node;
//! - [`beam`]: two-node Timoshenko beams with profile section properties;
//! - [`frame`]: local axes of shells and beams and the rotation of their matrices;
//! - [`face`]: consistent nodal forces of pressure on element faces;
//! - [`uniform`]: uniform-strain (one-point) hexahedron and tetrahedron kinematics, hourglass
//!   shape vectors and element frequency bounds for explicit dynamics;
//! - [`rotation`]: incrementally objective stress updates (Hughes–Winget Jaumann, Flanagan–Taylor
//!   Green–Naghdi);
//! - [`plasticity`]: J2 radial return with isotropic, kinematic and Johnson–Cook hardening,
//!   Johnson–Cook damage;
//! - [`truss`]: two- and three-node trusses (straight or curved), their stiffness, mass, body
//!   force and axial stress;
//! - [`dense`]: the small dense linear algebra inside an element.
//!
//! Conventions: element matrices are ordered node by node, each node's degrees of freedom in
//! [`DofKind`] order (three translations for solids, six for shells and beams; docs/fem.md); stress
//! and strain 6-vectors in the order xx, yy, zz, xy, xz, yz with engineering shear strains.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
// Matrix kernels over fixed small dimensions read as the textbook formulas with index loops.
#![allow(clippy::needless_range_loop)]

pub mod basis;
pub mod beam;
pub mod dense;
pub mod dof;
pub mod element;
pub mod face;
pub mod frame;
pub mod jet;
pub mod material;
pub mod plane;
pub mod plasticity;
pub mod quadrature;
pub mod rotation;
pub mod shape;
pub mod shell;
pub mod truss;
pub mod uniform;

pub use basis::Basis;
pub use dof::{DofKind, DofLayout, DofSet, ElementDofs};
pub use element::{ELEMENT_TYPES, ElementType, Family, Formulation, PointStress, Solid, element_type};
pub use material::{Elasticity, elasticity, principal, von_mises};

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
