//! BTL Test Plan Section 7 — Trending BIBBs.
//!
//! 13 subsections (7.1–7.13), 219 BTL test references total.
//! Covers: TrendLog View/Modify/Internal/External, Automated Retrieval,
//! TrendLogMultiple, Archival.

pub mod retrieval;
pub mod trend_log;
pub mod trend_log_multiple;
pub mod view;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    view::register(registry);
    trend_log::register(registry);
    retrieval::register(registry);
    trend_log_multiple::register(registry);
}
