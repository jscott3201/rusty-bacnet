//! The `bacnet` tool's own layer over [`bacnet_ops`], shared by its one-shot
//! commands, its terminal UI and its MCP server (#2560): the parts an
//! application on `bacnet-ops` doesn't need.
//!
//! - [`safety`]: the write-safety core the TUI and the MCP server share:
//!   levels, tiers, previews, the ledger, the audit log, the workspace lock
//!   and the client half of a write (#2068);
//! - [`class`]: what each command does to a device (read, write or device
//!   management), the table the front ends' safety levels share;
//! - [`decode`]: detailed field trees with source octet ranges; shared
//!   header summaries are in [`bacnet_ops::decode`];
//! - [`bbmd`]: B/IP broadcast management tables as plain data, their reads
//!   through the client, the survey that follows them and the consistency
//!   check over them.
#![forbid(unsafe_code)]

pub mod bbmd;
pub mod class;
pub mod decode;
pub mod safety;
