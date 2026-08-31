//! BACnet object model: traits, database, and standard object types.

pub mod access_control;
pub mod accumulator;
pub mod analog;
pub mod audit;
pub mod averaging;
pub mod binary;
pub mod clock;
pub mod color;
pub mod command;
pub(crate) mod common;
pub mod database;
pub mod device;
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
pub mod loop_obj;
pub mod multistate;
pub mod network_port;
pub mod notification_class;
pub mod program;
pub mod property_metadata;
pub(crate) mod reference;
pub(crate) mod reliability_inhibit;
pub(crate) mod rollback;
pub mod schedule;
pub mod staging;
pub mod timer;
pub mod traits;
pub mod trend;
pub mod value_types;

#[cfg(test)]
mod log_status_tests;

#[cfg(test)]
mod property_metadata_tests;

#[cfg(test)]
mod reliability_writability_tests;

#[cfg(test)]
mod reliability_inhibit_tests;
