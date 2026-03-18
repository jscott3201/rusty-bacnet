//! BTL Test Plan Section 13 — Audit Reporting BIBBs.
//!
//! 6 subsections (13.1–13.6), 80 BTL test references total.
//! Covers: Audit Log, Audit Reporter, Reporter Simple,
//! Forwarder, View, Advanced View+Modify.

pub mod logging;
pub mod reporter;
pub mod view;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    logging::register(registry);
    reporter::register(registry);
    view::register(registry);
}
