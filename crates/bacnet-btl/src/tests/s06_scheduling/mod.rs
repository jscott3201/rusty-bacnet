//! BTL Test Plan Section 6 — Scheduling BIBBs.
//!
//! 10 subsections (6.1–6.10), 222 BTL test references total.
//! Covers: Schedule View/Modify, Weekly Schedule, Internal/External-B,
//! Readonly-B, Schedule-A, Timer Internal/External-B.

pub mod internal_b;
pub mod readonly_b;
pub mod timer;
pub mod view_modify;
pub mod weekly_external;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    view_modify::register(registry);
    internal_b::register(registry);
    weekly_external::register(registry);
    readonly_b::register(registry);
    timer::register(registry);
}
