//! ModelCraft's MCP server (docs/mcp.md).
//!
//! JSON-RPC 2.0 over stdio, one message per line, implementing the Model Context Protocol's
//! lifecycle, tools and resources. Every tool runs engine commands (the same ones the UI,
//! scripts, the CLI and the control channel run) and never opens a dialog. A tool call is served
//! by a [`Backend`]: an in-process headless session ([`Headless`], pictures drawn with the CPU
//! rasterizer) or a running app reached through its control channel ([`Remote`],
//! docs/control-protocol.md), so a person can watch the agent work.
//!
//! The server treats its input as hostile: a message line is capped at [`MAX_LINE`] bytes and
//! answered with an error past it, a JSON-RPC batch at [`MAX_BATCH_MESSAGES`] messages, a
//! `batch` tool call at [`tools::MAX_STEPS`] commands, and tool arguments are checked against
//! the tool's schema before anything runs. Read and write roots ([`modelcraft_io::roots`])
//! confine every file a tool touches.
//!
//! Adapted from SolidCraft's `crates/mcp` (MIT OR Apache-2.0), see ATTRIBUTION.md.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod backend;
mod server;
pub mod tools;

pub use backend::{Backend, Headless, Picture, PictureRequest, Remote};
pub use server::{MAX_BATCH_MESSAGES, MAX_LINE, PROTOCOL_VERSION, SUPPORTED_VERSIONS, Server};

use modelcraft_io::roots::Roots;

/// How the server is started (`modelcraft-cli mcp`).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The control channel of a running app (`127.0.0.1:7070`); `None` runs headless.
    pub connect: Option<String>,
    /// Folders file access is confined to; unconfined by default.
    pub roots: Roots,
}

/// The backend `options` ask for, and a note for stderr when the server falls back to headless
/// because the app cannot be reached. Confined roots with `connect` are refused: the app would
/// read and write files the roots do not cover.
pub fn backend(options: Options) -> Result<(Box<dyn Backend>, Option<String>), String> {
    let headless = |roots: Roots| -> Box<dyn Backend> { Box::new(Headless::new(roots)) };
    match options.connect {
        Some(addr) if options.roots.is_confined() => Err(format!(
            "--connect {addr} cannot be combined with --automation-read-root or --automation-write-root: the app is not confined by them (they imply --headless)"
        )),
        Some(addr) => match Remote::connect(&addr) {
            Ok(r) => Ok((Box::new(r), None)),
            Err(e) => Ok((headless(options.roots), Some(format!("ModelCraft at {addr} cannot be reached ({e}); running headless")))),
        },
        None => Ok((headless(options.roots), None)),
    }
}

#[cfg(test)]
mod tests;
