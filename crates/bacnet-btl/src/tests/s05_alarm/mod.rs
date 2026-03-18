//! BTL Test Plan Section 5 — Alarm and Event Management BIBBs.
//!
//! 36 subsections (5.1–5.36), 456 BTL test references total.
//! Covers: Event Notification (A/B), Acknowledge, Summaries,
//! Event Log, Life Safety, Notification Forwarder, Access Control, Elevator.

pub mod acknowledge;
pub mod alarm_summary;
pub mod domain_specific;
pub mod event_log;
pub mod notification_a;
pub mod notification_ext_b;
pub mod notification_int_b;
pub mod view_modify;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    notification_a::register(registry);
    notification_int_b::register(registry);
    notification_ext_b::register(registry);
    acknowledge::register(registry);
    alarm_summary::register(registry);
    event_log::register(registry);
    view_modify::register(registry);
    domain_specific::register(registry);
}
