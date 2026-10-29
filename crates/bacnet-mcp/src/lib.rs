//! A read-only [Model Context Protocol](https://modelcontextprotocol.io)
//! server for BACnet sites (#2066), so an AI agent host (Claude Code,
//! Codex and others) can discover devices and read them through the same
//! engine and request limits as the `bacnet` tool's terminal UI.
//!
//! The server is built on the official `rmcp` SDK and targets the
//! 2026-07-28 revision of the protocol, while still answering clients that
//! open with the older `initialize` handshake. `bacnet mcp serve --stdio`
//! (the CLI's `mcp` feature) runs it; `docs/design/mcp.md` has the design.
//!
//! - Every tool is read-only and reaches the network only through
//!   [`bacnet_ops::engine::Engine`], under its per-device gate and global
//!   rate, a rate per network, and a token bucket per principal
//!   ([`Policy`]). Each result reports what it cost.
//! - Device-supplied text comes back in an untrusted envelope: cleaned of
//!   control and bidirectional characters, defanged and capped.
//! - Results are paged with cursors signed by a per-process key.
//! - Passive mode builds no client and lists only the tools that send
//!   nothing.
//!
//! Stdout carries only JSON-RPC: this crate never prints, and its logs go
//! through `tracing`, which the host process points at stderr.

mod context;
mod cursor;
mod errors;
mod follow;
mod legacy;
mod listen;
pub mod policy;
mod prompts;
mod resources;
mod server;
mod tools;
mod untrusted;
mod value;

pub use policy::Policy;
pub use server::{McpServer, ServeError};

#[cfg(test)]
mod tests;
