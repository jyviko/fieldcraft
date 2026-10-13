//! Solver decks: the solver-neutral model written as Abaqus input, Nastran bulk data and LS-DYNA
//! keyword files. Each format is its own module (`abaqus`, `nastran`, `lsdyna`), written from the
//! vendor's public reference manual in our own words; examples and expected files are our own.
//! Writers read [`modelcraft_model::Model`] and its `resolve()` result, never another format.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

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
