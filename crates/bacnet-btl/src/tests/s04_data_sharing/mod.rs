//! BTL Test Plan Section 4 — Data Sharing BIBBs.
//!
//! 55 subsections (4.1–4.55), 799 BTL test references total.
//! Covers: RP, RPM, WP, WPM, COV, ReadRange, WriteGroup,
//! Value Source, View/Modify, domain-specific data sharing.

pub mod cov_a;
pub mod cov_b;
pub mod cov_multiple;
pub mod cov_property;
pub mod cov_unsub;
pub mod domain_specific;
pub mod read_range;
pub mod rp_a;
pub mod rp_b;
pub mod rpm_a;
pub mod rpm_b;
pub mod view_modify;
pub mod wp_a;
pub mod wp_b;
pub mod wpm_a;
pub mod wpm_b;
pub mod write_group;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    rp_a::register(registry);
    rp_b::register(registry);
    rpm_a::register(registry);
    rpm_b::register(registry);
    wp_a::register(registry);
    wp_b::register(registry);
    wpm_a::register(registry);
    wpm_b::register(registry);
    cov_a::register(registry);
    cov_b::register(registry);
    view_modify::register(registry);
    read_range::register(registry);
    cov_unsub::register(registry);
    cov_property::register(registry);
    write_group::register(registry);
    cov_multiple::register(registry);
    domain_specific::register(registry);
}
