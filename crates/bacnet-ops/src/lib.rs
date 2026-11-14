//! The UI-free operations core of the `bacnet` tool, shared by its one-shot
//! commands, its terminal UI and its MCP server (#2064).
//!
//! Everything here works on a [`BACnetClient`](bacnet_client::client::BACnetClient)
//! and plain data, with no terminal, no ratatui and no MCP dependency, so
//! every front end reads targets, values and devices the same way and
//! obeys the same request limits:
//!
//! - [`target`]: the target grammar (`192.168.1.10`, `1234`, `2:1234`,
//!   `bip6:0x0004D2`, `mstp:5`, ...);
//! - [`parse`]: the object, property and value grammar (`ai:1`,
//!   `present-value[3]`, `72.5@8`);
//! - [`format`](mod@format) and [`label`]: values as display text, and how objects,
//!   properties and priorities are named;
//! - [`limits`]: the global request-rate cap, the one-request-per-device
//!   gate, rates per network and per-caller accounts;
//! - [`engine`]: the read operations (properties, the object list, a device
//!   summary, trend log pages and alarms) and Who-Is over those limits;
//! - [`export`]: a device's objects in another model's terms, so far an
//!   experimental Standard 223 point list in Turtle (#2436);
//! - [`watch`]: the watch hub, one managed COV subscription or poller per
//!   point however many listeners share it, holding the latest values.
//!
//! The parts only the `bacnet` tool uses (the write-safety core, command
//! classes, frame decoding and the BBMD survey) are the `bacnet-tool`
//! crate (#2560).

pub mod engine;
pub mod export;
pub mod format;
pub mod label;
pub mod limits;
pub mod octets;
pub mod parse;
pub mod target;
pub mod watch;
