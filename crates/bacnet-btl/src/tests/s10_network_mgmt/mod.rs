//! BTL Test Plan Section 10 — Network Management BIBBs.
//!
//! 9 subsections (10.1–10.9), 96 BTL test references total.
//! Covers: Routing, Router Config, Connection Establishment,
//! BBMD Config, Foreign Device Registration, SC Hub.
//!
//! Note: Most routing tests require multi-network topology (Docker mode).
//! Tests verify routing-related properties and basic capabilities.

pub mod bbmd_config;
pub mod routing;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    routing::register(registry);
    bbmd_config::register(registry);
}
