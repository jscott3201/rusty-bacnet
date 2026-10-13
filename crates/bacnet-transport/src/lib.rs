//! BACnet transport layer: data-link transports and framing.
//!
//! - `port`: TransportPort trait for data-link abstraction
//! - `bvll`: BVLL (BACnet Virtual Link Layer) encode/decode for BACnet/IP (Annex J)
//! - `bip`: BACnet/IP over UDP transport
//! - `bbmd`: BBMD state tables and forwarding logic
//! - `mstp`: MS/TP token-passing transport over RS-485, driving the portable
//!   `bacnet-mstp-core` master with frames from `bacnet-mstp-codec` (Clause 9)
//! - `sc_frame`: BACnet/SC BVLC-SC frame encode/decode (Annex AB)
//! - `sc`: BACnet/SC hub-and-spoke transport over WebSocket
//! - `any`: Type-erased transport enum for mixed-transport routing

pub mod any;
pub mod bbmd;
pub mod bip;
#[cfg(feature = "ipv6")]
pub mod bip6;
pub mod bvll;
mod direct_response;
#[cfg(feature = "ethernet")]
pub mod ethernet;
mod local_addresses;
pub mod loopback;
pub mod mstp;
#[cfg(feature = "serial")]
pub mod mstp_serial;
mod npdu_channel;
pub mod port;
mod port_ownership;
pub mod sc;
pub mod sc_frame;
#[cfg(feature = "sc-tls")]
pub mod sc_hub;
mod sc_limits;
#[cfg(feature = "sc-tls")]
pub mod sc_tls;
#[cfg(feature = "sc-tls")]
mod tcp_connect;
#[cfg(feature = "sc-tls")]
mod tls_reject;
mod udp_metadata;
pub mod warn_throttle;
#[cfg(windows)]
mod windows_adapters;

#[cfg(test)]
mod log_capture;
#[cfg(test)]
#[path = "rb07_provenance_tests.rs"]
mod rb07_provenance_tests;
