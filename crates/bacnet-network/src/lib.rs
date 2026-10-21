#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
pub mod layer;
pub mod network_number;
pub mod priority_channel;
pub mod response_route;
pub mod router;
pub mod router_table;
mod totals;

#[cfg(test)]
mod address_bound_tests;

#[cfg(test)]
mod broadcast_pdu_type_tests;
#[cfg(test)]
mod global_broadcast_dadr_tests;
#[cfg(test)]
mod group_dadr_tests;

#[cfg(test)]
mod link_source_bound_tests;
#[cfg(test)]
mod router_reply_capability_tests;

#[cfg(test)]
mod loopback_fixture;

#[cfg(test)]
mod node_receive_tests;

#[cfg(test)]
mod reject_route_tests;

#[cfg(test)]
#[path = "rb07_provenance_tests.rs"]
mod rb07_provenance_tests;

#[cfg(test)]
#[path = "rb08_origin_provenance_tests.rs"]
mod rb08_origin_provenance_tests;
