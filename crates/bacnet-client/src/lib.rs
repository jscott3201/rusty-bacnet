#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]

pub mod client;
pub mod discovery;
mod endpoint_requester;
pub mod log_reader;
pub mod segmentation;
pub mod tags;
pub mod tsm;

#[doc(hidden)]
pub use endpoint_requester::{
    EndpointOperationAck, EndpointOperationOutcome, EndpointOperationRequest, EndpointReadAck,
    EndpointReadRequest, EndpointRequester, PacedEndpointOperation, PreparedEndpointOperation,
};
mod read_property;

mod endpoint_rpm;
