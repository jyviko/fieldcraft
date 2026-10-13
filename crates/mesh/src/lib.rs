//! Meshing: robust geometric predicates, boundary surface preparation, tetrahedral and
//! hexahedral volume meshing, and element quality.
//!
//! Every algorithm here is implemented from the papers cited in its module docs (see
//! `docs/meshing/literature.md`); no GPL-family code is read or used. Meshing decisions use exact
//! predicates, results are deterministic, and every iterative stage is bounded.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("limit reached: {0}")]
    Limit(String),
    #[error("meshing failed: {0}")]
    Failed(String),
    #[error(transparent)]
    Model(#[from] modelcraft_model::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub mod predicates;
