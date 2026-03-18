//! BTL test definitions — organized by BTL Test Plan 26.1 sections.
//!
//! ALL 13 sections fully audited and migrated.
//! Counts below are registered tests (includes parameterized cross-cutting tests):
//!   s02_basic/          — Section 2: Basic BACnet Functionality (27 tests)
//!   s03_objects/         — Section 3: Objects (701 tests)
//!   s04_data_sharing/    — Section 4: Data Sharing (801 tests)
//!   s05_alarm/           — Section 5: Alarm & Event (472 tests)
//!   s06_scheduling/      — Section 6: Scheduling (227 tests)
//!   s07_trending/        — Section 7: Trending (219 tests)
//!   s08_device_mgmt/     — Section 8: Device Management (592 tests)
//!   s09_data_link/       — Section 9: Data Link Layer (494 tests)
//!   s10_network_mgmt/    — Section 10: Network Management (96 tests)
//!   s11_gateway/         — Section 11: Gateway (5 tests)
//!   s12_security/        — Section 12: Network Security (9 tests)
//!   s13_audit/           — Section 13: Audit Reporting (80 tests)
//!   s14_web_services/    — Section 14: Web Services (2 tests)

pub mod helpers;
pub mod smoke;

pub mod s02_basic;
pub mod s03_objects;
pub mod s04_data_sharing;
pub mod s05_alarm;
pub mod s06_scheduling;
pub mod s07_trending;
pub mod s08_device_mgmt;
pub mod s09_data_link;
pub mod s10_network_mgmt;
pub mod s11_gateway;
pub mod s12_security;
pub mod s13_audit;
pub mod s14_web_services;

pub mod parameterized;

use crate::engine::registry::TestRegistry;

/// Register all implemented BTL tests.
pub fn register_all(registry: &mut TestRegistry) {
    smoke::register(registry);
    s02_basic::register(registry);
    s03_objects::register(registry);
    s04_data_sharing::register(registry);
    s05_alarm::register(registry);
    s06_scheduling::register(registry);
    s07_trending::register(registry);
    s08_device_mgmt::register(registry);
    s09_data_link::register(registry);
    s10_network_mgmt::register(registry);
    s11_gateway::register(registry);
    s12_security::register(registry);
    s13_audit::register(registry);
    s14_web_services::register(registry);
    parameterized::register(registry);
}
