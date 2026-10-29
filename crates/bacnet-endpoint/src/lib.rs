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

pub use identity::{DeviceIdentity, NetworkPortEntry, build_database_with_extra};
pub use roles::{ClientRoleHandle, EndpointApduDestination, ServerRoleHandle};
pub use session::{
    EndpointSession, PolicyCountersSnapshot, RESPONDER_OPT_IN_SERVICES, RESPONDER_SERVICES,
    ResponderOptIn, SessionConfig, SessionExit, SessionRole,
};
