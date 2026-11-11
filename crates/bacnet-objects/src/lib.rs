#![doc = include_str!("../README.md")]
// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
#![forbid(unsafe_code)]

// `format!` and `vec!` without std's prelude (#1615).
#[macro_use]
extern crate alloc;

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!(
    "bacnet-objects needs the `std` or the `alloc` feature: the database boxes every object"
);
#[cfg(not(target_has_atomic = "ptr"))]
compile_error!(
    "bacnet-objects needs atomic compare-and-swap: its objects share state through alloc::sync::Arc"
);

#[macro_use]
mod no_std_support;
pub(crate) use no_std_support::{catch_panic, collections, float, prelude, sync};

pub mod access_control;
pub mod accumulator;
pub mod analog;
pub mod audit;
pub mod averaging;
pub mod binary;
pub mod catalogue;
pub mod channel;
pub mod clock;
pub mod color;
pub mod command;
pub mod command_source;
pub(crate) mod common;
pub mod cov_change;
pub mod creation;
pub mod database;
pub mod device;
pub mod device_identity;
pub use device_identity::DeviceIdentity;
pub(crate) mod device_reference;
pub mod durable;
pub mod elevator;
pub mod event;
pub mod event_enrollment;
pub mod event_log;
pub mod file;
pub mod group;
pub mod life_safety;
pub mod lighting;
pub mod load_control;
pub mod log_buffer;
pub(crate) mod log_lifecycle;
pub mod log_persistence;
pub mod log_reporting;
pub(crate) mod log_window;
pub mod loop_obj;
pub mod multistate;
pub mod network_port;
pub mod notification_class;
pub mod notification_forwarder;
pub mod object_profile;
pub mod points;
pub mod present_value_access;
pub mod program;
pub mod property_metadata;
pub(crate) mod reference;
pub(crate) mod reliability_inhibit;
pub mod schedule;
pub mod snapshot;
pub mod staging;
pub mod subscribed_recipients;
pub mod timer;
pub mod traits;
pub(crate) mod transition;
pub mod trend;
pub mod value_types;
pub mod write_config;

#[cfg(test)]
mod description_kept_tests;

#[cfg(test)]
mod log_status_tests;

#[cfg(test)]
mod log_window_tests;

#[cfg(test)]
mod property_metadata_audit;

#[cfg(test)]
mod present_value_access_tests;

#[cfg(test)]
mod local_input_tests;

#[cfg(all(test, feature = "event-reporting"))]
mod event_reporting_tests;

#[cfg(test)]
mod capability_features_tests;

#[cfg(test)]
mod detection_disabled_commit_tests;

#[cfg(test)]
mod cov_reporting_tests;

#[cfg(test)]
mod evaluation_capability_tests;

#[cfg(test)]
mod log_file_capability_tests;

#[cfg(test)]
mod audit_capability_tests;

#[cfg(test)]
mod schedule_capability_tests;

#[cfg(test)]
mod property_metadata_tests;
#[cfg(test)]
mod test_rows;

#[cfg(test)]
mod reliability_writability_tests;

#[cfg(test)]
mod reliability_inhibit_tests;

#[cfg(test)]
mod read_consistency_tests;
#[cfg(test)]
mod read_write_round_trip_seeds;
#[cfg(test)]
mod read_write_round_trip_tests;
#[cfg(test)]
mod typed_enum_storage_tests;
#[cfg(test)]
mod write_capability_tests;
#[cfg(test)]
mod write_consistency_tests;

#[cfg(test)]
mod typed_units_tests;

#[cfg(test)]
mod computed_status_flags_tests;

#[cfg(test)]
mod cov_criteria_tests;

#[cfg(test)]
mod scalar_relinquishment_tests;

#[cfg(all(test, feature = "event-reporting"))]
mod enrollment_summary_capability_tests;

#[cfg(all(test, feature = "event-reporting"))]
mod acknowledge_alarm_object_family_tests;

#[cfg(all(test, feature = "event-reporting"))]
mod acked_transitions_tests;

#[cfg(test)]
mod undefined_property_rows_tests;
