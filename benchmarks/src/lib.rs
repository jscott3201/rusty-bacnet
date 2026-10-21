//! Shared helpers for the Criterion suites, the `stress-test` binary and the Docker
//! cross-network test binaries.

// `publish = false` benchmark and stress plumbing with no external API, so it is
// exempt from the missing_docs policy (#902).
#![allow(missing_docs)]
#![forbid(unsafe_code)]

pub mod helpers;
pub mod sc_helpers;
pub mod stress;
