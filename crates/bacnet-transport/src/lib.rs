//! BACnet transport layer: data-link transports and framing.
//!
//! - `port`: TransportPort trait for data-link abstraction
//! - `bvll`: BVLL (BACnet Virtual Link Layer) encode/decode for BACnet/IP (Annex J)
//! - `bip`: BACnet/IP over UDP transport
//! - `bbmd`: BBMD state tables and forwarding logic
//! - `mstp_frame`: MS/TP frame encode/decode with CRC-8/CRC-16 (Clause 9)
//! - `mstp`: MS/TP token-passing transport over RS-485
//! - `sc_frame`: BACnet/SC BVLC-SC frame encode/decode (Annex AB)
//! - `sc`: BACnet/SC hub-and-spoke transport over WebSocket
//! - `any`: Type-erased transport enum for mixed-transport routing

pub mod any;
pub mod bbmd;
pub mod bip;
#[cfg(feature = "ipv6")]
pub mod bip6;
pub mod bvll;
#[cfg(feature = "ethernet")]
pub mod ethernet;
pub mod loopback;
pub mod mstp;
pub mod mstp_frame;
#[cfg(feature = "serial")]
pub mod mstp_serial;
pub mod port;
pub mod sc;
pub mod sc_frame;
#[cfg(feature = "sc-tls")]
pub mod sc_hub;
#[cfg(feature = "sc-tls")]
pub mod sc_tls;
