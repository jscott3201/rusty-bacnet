#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]

mod acks;
pub mod client;
pub mod discovery;
mod endpoint_requester;
pub mod log_reader;
pub mod segmentation;
mod sync_mutex;
pub mod tags;
pub mod tsm;

#[doc(hidden)]
pub use endpoint_requester::{
    EndpointOperationAck, EndpointOperationOutcome, EndpointOperationRequest, EndpointReadAck,
    EndpointReadRequest, EndpointRequester, PacedEndpointOperation, PreparedEndpointOperation,
};

mod endpoint_rpm;
