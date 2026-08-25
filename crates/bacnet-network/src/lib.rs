//! BACnet network layer: packet assembly, dispatch, and routing.

#[allow(dead_code)]
mod endpoint_ingress;
pub mod layer;
pub mod priority_channel;
pub mod router;
pub mod router_table;
