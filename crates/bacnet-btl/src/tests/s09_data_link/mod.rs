//! BTL Test Plan Section 9 — Data Link Layer Tests.
//!
//! 12 subsections (9.1–9.12), 494 BTL test references total.
//! Covers: MS/TP Manager/Subordinate, IPv4 (BIP+BBMD+FD),
//! ZigBee, Ethernet, ARCNET, LonTalk, IPv6, Secure Connect,
//! Virtual Network, B/IP PAD, Proprietary.

pub mod ethernet;
pub mod ipv4;
pub mod ipv6;
pub mod mstp;
pub mod other_dll;
pub mod sc;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    mstp::register(registry);
    ipv4::register(registry);
    ethernet::register(registry);
    ipv6::register(registry);
    sc::register(registry);
    other_dll::register(registry);
}
