#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

/// Device-wide outbound invoke-ID leasing and response admission.
pub mod coordinator;
/// Single-owner network ingress and bounded APDU classification.
pub mod endpoint_ingress;
