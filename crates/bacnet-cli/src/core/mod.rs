//! Code shared by the one-shot commands and the TUI.
//!
//! Anything that both front ends need lives here, so the TUI does not grow a
//! second copy of a parser or a lookup that the CLI already has.

pub(crate) mod class;
pub(crate) mod destination;
pub(crate) mod guard;
// Only the TUI picks an interface; a one-shot binds --interface or all.
#[cfg(feature = "tui")]
pub(crate) mod interfaces;
pub(crate) mod range;
pub(crate) mod target;
