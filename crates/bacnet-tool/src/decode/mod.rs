//! Detailed offline field trees for the capture screen and MCP decoder.
//!
//! [`tree`] decodes frames, NPDUs and APDUs into fields with their source
//! octet ranges. Shared header summaries live in [`bacnet_ops::decode`].

pub mod tree;
