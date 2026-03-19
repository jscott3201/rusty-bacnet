//! BACnet HTTP REST API and MCP server gateway.
//!
//! The gateway crate provides optional HTTP REST API and MCP server interfaces
//! for BACnet networks. By default, **no HTTP or MCP dependencies are compiled**.
//!
//! # Feature flags
//!
//! - `http` — REST API module (`api`) with Axum routes and auth middleware
//! - `mcp` — MCP server module (`mcp`) with tools, resources, and knowledge base
//! - `bin` — enables both `http` and `mcp` plus the CLI binary entry point
//!
//! # Always available (no web dependencies)
//!
//! - `config` — TOML configuration parsing and validation
//! - `state` — shared gateway state (wraps BACnet client/server)
//! - `builder` — constructs the BACnet stack from config
//! - `parse` — BACnet value parsing and formatting utilities

#[cfg(feature = "http")]
pub mod api;
#[cfg(feature = "http")]
pub mod auth;
pub mod builder;
pub mod config;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod parse;
pub mod state;
