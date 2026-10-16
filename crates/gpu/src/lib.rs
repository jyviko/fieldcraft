//! FieldCraft on the GPU without a window toolkit: the viewport's pipelines ([`view`]), the
//! choice of adapter ([`select`]) and offscreen pictures drawn with them ([`capture`]). The
//! desktop viewport draws through [`view`] in its paint callback; `fieldcraft-cli snapshot` draws
//! through [`capture`], so a headless picture is the one the window draws.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
pub mod capture;
#[cfg(not(target_arch = "wasm32"))]
pub mod select;
pub mod view;

pub use wgpu;

/// What can go wrong on the GPU. Never a panic: device errors are caught in error scopes.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GpuError {
    #[error("{0}")]
    NoAdapter(String),
    #[error("FIELDCRAFT_GPU must be auto, hardware, software or cpu, not `{0}`")]
    Policy(String),
    #[error("the GPU device could not be opened: {0}")]
    Device(String),
    #[error("{0}")]
    Size(String),
    #[error("the GPU picture failed: {0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, GpuError>;
