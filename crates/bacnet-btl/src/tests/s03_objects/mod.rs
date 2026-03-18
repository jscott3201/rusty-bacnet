//! BTL Test Plan Section 3 — Objects.
//!
//! One submodule per BTL subsection (one per object type or small group).
//! Total: 701 BTL test references across 66 object types.

pub mod access_control; // 3.44-3.49: 38 BTL refs
pub mod access_door; // 3.42: 16 BTL refs
pub mod accumulator; // 3.37+3.41: 15 BTL refs
pub mod analog_input; // 3.1:   2 BTL refs
pub mod analog_output; // 3.2:   8 BTL refs
pub mod analog_value; // 3.3:  10 BTL refs
pub mod audit; // 3.63+3.64: 2 BTL refs
pub mod averaging; // 3.4:   2 BTL refs
pub mod binary_input; // 3.5:   7 BTL refs
pub mod binary_output; // 3.6:  29 BTL refs
pub mod binary_value; // 3.7:  30 BTL refs
pub mod calendar; // 3.8:   7 BTL refs
pub mod channel; // 3.53: 51 BTL refs
pub mod color; // 3.65+3.66: 36 BTL refs (Color + ColorTemperature)
pub mod command; // 3.9:  13 BTL refs
pub mod device; // 3.10: 13 BTL refs
pub mod elevator; // 3.58-3.60: 13 BTL refs
pub mod event_enrollment; // 3.11+3.52: 3 BTL refs
pub mod event_log; // 3.22:  8 BTL refs
pub mod file; // 3.61: 10 BTL refs
pub mod global_group; // 3.36: 28 BTL refs
pub mod group; // 3.12:  1 BTL ref
pub mod life_safety; // 3.39+3.40: 20 BTL refs
pub mod lighting; // 3.54+3.55: 59 BTL refs
pub mod load_control; // 3.43:  9 BTL refs
pub mod loop_obj; // 3.13:  5 BTL refs
pub mod multistate_input; // 3.14:  6 BTL refs
pub mod multistate_output; // 3.15: 11 BTL refs
pub mod multistate_value; // 3.16: 14 BTL refs
pub mod network_port; // 3.56:  2 BTL refs
pub mod notification_class; // 3.17: 11 BTL refs
pub mod notification_forwarder; // 3.51: 16 BTL refs
pub mod program; // 3.38:  2 BTL refs
pub mod schedule; // 3.19:  4 BTL refs
pub mod staging; // 3.62: 24 BTL refs
pub mod structured_view; // 3.21:  2 BTL refs
pub mod timer; // 3.57: 33 BTL refs
pub mod trend_log; // 3.20+3.23: 2 BTL refs
pub mod value_types; // 3.24-3.35: ~114 BTL refs

use crate::engine::registry::TestRegistry;

pub fn register(registry: &mut TestRegistry) {
    analog_input::register(registry);
    analog_output::register(registry);
    analog_value::register(registry);
    averaging::register(registry);
    binary_input::register(registry);
    binary_output::register(registry);
    binary_value::register(registry);
    calendar::register(registry);
    command::register(registry);
    device::register(registry);
    event_enrollment::register(registry);
    group::register(registry);
    loop_obj::register(registry);
    multistate_input::register(registry);
    multistate_output::register(registry);
    multistate_value::register(registry);
    notification_class::register(registry);
    schedule::register(registry);
    trend_log::register(registry);
    structured_view::register(registry);
    event_log::register(registry);
    value_types::register(registry);
    global_group::register(registry);
    accumulator::register(registry);
    program::register(registry);
    life_safety::register(registry);
    access_door::register(registry);
    load_control::register(registry);
    access_control::register(registry);
    notification_forwarder::register(registry);
    channel::register(registry);
    lighting::register(registry);
    network_port::register(registry);
    timer::register(registry);
    elevator::register(registry);
    file::register(registry);
    staging::register(registry);
    audit::register(registry);
    color::register(registry);
}
