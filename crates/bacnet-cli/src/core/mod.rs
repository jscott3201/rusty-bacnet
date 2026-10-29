//! Code shared by the one-shot commands and the TUI.
//!
//! Anything that both front ends need lives here, so the TUI does not grow a
//! second copy of a parser or a lookup that the CLI already has. The parts
//! with no CLI dependency (the class table, the target grammar, the value
//! grammar and formatter, the request limits and the read engine) are in
//! the `bacnet-ops` crate, which the MCP server shares too.

pub(crate) mod guard;
// Only the TUI picks an interface; a one-shot binds --interface or all.
#[cfg(feature = "tui")]
pub(crate) mod interfaces;
pub(crate) mod range;

#[cfg(test)]
mod class_tests;
