#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]

mod acks;
pub mod client;
pub mod discovery;
pub mod log_reader;
mod poll_gate;
pub mod segmentation;
mod sync_mutex;
pub mod tags;
pub mod tsm;
