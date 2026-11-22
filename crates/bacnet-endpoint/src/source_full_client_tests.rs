//! Source Audit of the session's client (#2617): the services it covers,
//! what a stop or a caller that goes away does to an audited request, the
//! 64 slots, and the node's DeviceCommunicationControl.
use super::*;

#[path = "source_full_client_services_tests.rs"]
mod services;

#[path = "source_full_client_lifecycle_tests.rs"]
mod lifecycle;

#[path = "source_full_client_slots_tests.rs"]
mod slots;

#[path = "source_full_client_dcc_tests.rs"]
mod dcc;
