//! Private shared endpoint primitives for workspace role crates.

/// Device-wide outbound invoke-ID leasing and response admission.
pub mod coordinator;
/// Single-owner network ingress and bounded APDU classification.
pub mod endpoint_ingress;
