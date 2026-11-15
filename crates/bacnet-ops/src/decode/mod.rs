//! Offline frame summaries shared by Python, the CLI and the MCP server.
//!
//! [`frame`] reads B/IP, B/IPv6, BACnet/SC and MS/TP frame headers;
//! [`decode_npdu_layer`] reads their common network and application headers.
//! Malformed octets produce diagnostic data rather than sending any traffic.
//! Detailed field trees remain in `bacnet-tool`.

pub mod frame;
mod layers;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod testing;

pub use layers::{DecodedApdu, DecodedNpdu, decode_npdu_layer};
