//! BACnet object model: traits, database, and standard object types.

pub mod access_control;
pub mod accumulator;
pub mod analog;
pub mod audit;
pub mod averaging;
pub mod binary;
pub mod channel;
pub mod clock;
pub mod color;
pub mod command;
pub mod command_source;
pub(crate) mod common;
pub mod database;
pub mod device;
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
pub mod log_reporting;
pub(crate) mod log_window;
pub mod loop_obj;
pub mod multistate;
pub mod network_port;
pub mod notification_class;
pub mod notification_forwarder;
pub mod object_profile;
pub mod present_value_access;
pub mod program;
pub mod property_metadata;
pub(crate) mod reference;
pub(crate) mod reliability_inhibit;
pub mod schedule;
pub mod staging;
pub mod subscribed_recipients;
pub mod timer;
pub mod traits;
pub(crate) mod transition;
pub mod trend;
pub mod value_types;

#[cfg(test)]
mod log_status_tests;

#[cfg(test)]
mod log_window_tests;

#[cfg(test)]
mod property_metadata_audit;

#[cfg(test)]
mod present_value_access_tests;

#[cfg(test)]
mod property_metadata_tests;

#[cfg(test)]
mod reliability_writability_tests;

#[cfg(test)]
mod reliability_inhibit_tests;

#[cfg(test)]
mod typed_enum_storage_tests;

#[cfg(test)]
mod computed_status_flags_tests;

#[cfg(test)]
mod cov_criteria_tests;

#[cfg(test)]
mod scalar_relinquishment_tests;

#[cfg(test)]
mod enrollment_summary_capability_tests;

#[cfg(test)]
mod acknowledge_alarm_object_family_tests;

#[cfg(test)]
mod acked_transitions_tests;

#[cfg(test)]
mod undefined_property_rows_tests;
