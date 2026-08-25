//! BACnet endpoint ownership and transaction coordination primitives.

/// Device-wide outbound invoke-ID leasing and response admission.
pub mod coordinator;
/// Single-owner network ingress and bounded APDU classification.
pub mod endpoint_ingress;
