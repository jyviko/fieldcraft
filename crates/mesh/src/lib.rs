//! Meshing: robust geometric predicates, boundary surface preparation, tetrahedral and
//! hexahedral volume meshing, and element quality.
//!
//! Every algorithm here is implemented from the papers cited in its module docs (see
//! `docs/meshing/literature.md`); no GPL-family code is read or used. Meshing decisions use exact
//! predicates, results are deterministic, and every iterative stage is bounded.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod block;
pub mod cdt;
pub mod delaunay;
pub mod hex;
pub mod join;
pub mod mirror;
pub mod nodes;
pub mod octree;
pub mod planar;
pub mod quad;
pub mod quality;
pub mod seeds;
pub mod size;
pub mod surface;
pub mod tet;
mod vec3;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("limit reached: {0}")]
    Limit(String),
    #[error("meshing failed: {0}")]
    Failed(String),
    /// The input is valid but outside what the mesher handles yet (a body that is not
    /// sweepable, …); the message says why.
    #[error("not supported yet: {0}")]
    Unsupported(String),
    /// The boundary surface has defects ([`surface::BoundarySurface::new`]).
    #[error("invalid boundary surface: {0}")]
    Surface(surface::Defects),
    #[error(transparent)]
    Model(#[from] fieldcraft_model::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub mod predicates;
