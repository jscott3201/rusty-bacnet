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

pub use identity::{build_database_with_extra, DeviceIdentity, NetworkPortEntry};
pub use roles::{ClientRoleHandle, EndpointApduDestination, ServerRoleHandle};
pub use session::{
    EndpointSession, PolicyCountersSnapshot, SessionConfig, SessionExit, SessionRole,
};
