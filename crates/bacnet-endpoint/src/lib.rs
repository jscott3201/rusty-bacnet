#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]

pub mod bip;
mod held_database;
pub mod identity;
pub mod mstp;
pub mod roles;
pub mod sc;
pub mod session;
mod source_audit;
// The server's rows for test objects. The session's tests state no value
// types, so `typed` goes unused here.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../bacnet-server/src/test_rows.rs"]
mod test_rows;
#[cfg(test)]
mod wrapped_session_tests;

pub use identity::{DeviceIdentity, NetworkPortEntry, build_database_with_extra};
pub use roles::{ClientRoleHandle, EndpointApduDestination};
pub use session::{
    EndpointSession, PolicyCountersSnapshot, SessionConfig, SessionExit, SessionRole,
};
