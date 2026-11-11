#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]

pub mod audit_notification;
mod command_lists;
mod committed_cov;
pub mod cov;
pub mod device_communication_control;
mod device_view;
pub mod event_enrollment;
pub mod fault_detection;
pub mod handlers;
pub mod life_safety;
mod local_device;
mod membership;
pub mod mutation;
#[doc(hidden)]
pub mod network_number;
#[doc(hidden)]
pub mod network_port_registration;
pub mod pics;
pub mod policy;
pub use policy::{FnPolicy, ServerPolicy};
mod runtime_clock;
pub mod schedule;
pub mod server;
#[cfg(test)]
mod test_event_state;
#[cfg(test)]
mod test_rows;
pub mod trend_log;

#[cfg(test)]
mod command_source;
// The tests name the source a local write takes as `crate::LocalCommandSource`;
// it is `bacnet_device_core::write::local::LocalCommandSource` (#2446).
#[cfg(test)]
use bacnet_device_core::write::local::LocalCommandSource;
