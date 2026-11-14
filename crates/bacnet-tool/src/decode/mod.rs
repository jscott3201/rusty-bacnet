//! BACnet frames decoded for people: what the TUI's capture screen and the
//! MCP server's `bacnet_decode` tool show, with no network traffic.
//!
//! - [`frame`] reads a frame of any link the stack taps (B/IP, B/IPv6,
//!   BACnet/SC, MS/TP), or a bare NPDU, as far as a one-line summary needs.
//! - [`tree`] decodes a frame, an NPDU or an APDU into a tree of fields,
//!   each with the octets it came from; service parameters are walked tag
//!   by tag.
//! - [`decode_npdu_layer`] reads the network and application layers that
//!   every link carries the same way.
//!
//! Every decoder takes untrusted octets: a malformed frame becomes a
//! problem in the result, never a panic. A character string in a frame is
//! whatever its sender wrote, so a tree field quoting one says so
//! ([`tree::Field::device_text`]).

pub mod frame;
mod layers;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod testing;
pub mod tree;

pub use layers::{DecodedApdu, DecodedNpdu, decode_npdu_layer};
