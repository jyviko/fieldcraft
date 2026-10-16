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

/// The kind of device a picture was drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Hardware,
    Software,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Hardware => "hardware GPU",
            Tier::Software => "software GPU",
        }
    }
}

/// What drew a picture.
#[derive(Clone, Debug, PartialEq)]
pub enum Renderer {
    Gpu {
        adapter: String,
        tier: Tier,
        samples: u32,
    },
    /// The CPU rasterizer, and why when a GPU was wanted (`auto` with no usable GPU).
    Cpu {
        why: Option<String>,
    },
}

impl Renderer {
    /// `"hardware"`, `"software"` or `"cpu"`.
    pub fn kind(&self) -> &'static str {
        match self {
            Renderer::Gpu { tier: Tier::Hardware, .. } => "hardware",
            Renderer::Gpu { tier: Tier::Software, .. } => "software",
            Renderer::Cpu { .. } => "cpu",
        }
    }

    /// What drew a picture, for a reply: `{kind, adapter, msaa}` on a GPU, `{kind, why}` on
    /// the CPU (`why` null when the CPU was asked for).
    pub fn report(&self) -> serde_json::Value {
        match self {
            Renderer::Gpu { adapter, samples, .. } => serde_json::json!({"kind": self.kind(), "adapter": adapter, "msaa": samples}),
            Renderer::Cpu { why } => serde_json::json!({"kind": self.kind(), "why": why}),
        }
    }
}
