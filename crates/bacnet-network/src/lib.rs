//! BACnet network layer: packet assembly, dispatch, and routing.

pub mod layer;
pub mod priority_channel;
pub mod router;
pub mod router_table;

#[cfg(test)]
#[path = "rb07_provenance_tests.rs"]
mod rb07_provenance_tests;

#[cfg(test)]
#[path = "rb08_origin_provenance_tests.rs"]
mod rb08_origin_provenance_tests;
