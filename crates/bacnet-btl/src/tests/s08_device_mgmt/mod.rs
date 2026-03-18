//! BTL Test Plan Section 8 — Device Management BIBBs.
//!
//! 30 subsections (8.1–8.30), 591 BTL test references total.
//! Covers: Device/Object Binding, Time Sync, DCC, Reinitialize,
//! Backup/Restore, Restart, CreateObject/DeleteObject, List Manipulation,
//! Text Message, Virtual Terminal, Subordinate Proxy.

pub mod binding;
pub mod create_delete_a;
pub mod create_delete_b;
pub mod dcc;
pub mod list_manipulation;
pub mod misc;
pub mod time_sync;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    binding::register(registry);
    time_sync::register(registry);
    dcc::register(registry);
    misc::register(registry);
    create_delete_a::register(registry);
    create_delete_b::register(registry);
    list_manipulation::register(registry);
}
