//! BTL Test Plan Section 2 — Basic BACnet Functionality.
//!
//! 2.1 Base Requirements (22 tests)
//! 2.2 Segmentation Support (3 tests)
//! 2.3 Private Transfer Services (2 tests)

pub mod base;

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    base::register(registry);
}
