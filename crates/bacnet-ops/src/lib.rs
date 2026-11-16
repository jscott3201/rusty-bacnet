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
//! - [`format`](mod@format) and [`label`]: values as display text, and how objects
//!   and properties are named;
//! - [`limits`]: the global request-rate cap, the one-request-per-device
//!   gate, rates per network and per-caller accounts;
//! - [`client`](mod@client): [`Client`], the high-level client: its connect
//!   builders ([`Client::bip`] and the others), and discovery over those
//!   limits: devices ([`Discover`]) and the devices that hold an object
//!   ([`Find`]), each a local broadcast unless it names another [`Scope`];
//! - [`device`]: [`DeviceAddress`], where a device answers, and the
//!   [`Device`] handle, pinned to an address or following an instance, whose
//!   methods are the read operations (properties, the object list, a device
//!   summary, trend log pages and alarms);
//! - [`reading`](mod@reading): what a read names ([`PropertyRef`], from
//!   text such as `"ai:1 pv"`) and returns: a [`Value`] (typed data, display
//!   text and the octets sent), a [`Reading`] a property, the
//!   [`Readings`] of a bulk read, which reports a refused property in its
//!   row and keeps what was read when the device stops answering, and an
//!   object's [`ObjectRead`];
//! - [`write`](mod@write): the write calls' parts: [`Priority`], the
//!   [`WritePolicy`] that refuses priorities 1, 2, 5 and 6 unless allowed,
//!   what [`Device::write`] and [`Device::relinquish`] send and return
//!   ([`Written`], [`PriorityView`]), and what
//!   [`Device::preview_write`] reads ([`WritePreview`]);
//! - [`inventory`]: a device's objects as [`PointRecord`]s
//!   ([`Device::inventory`]): each one's name, units, state texts and
//!   whether it takes commands, with its value when asked, read in batches
//!   and written as JSON or CSV;
//! - [`error`](mod@error): [`OpsError`], why a call failed as a whole;
//! - [`raw`]: the reading layer under them, for front ends that run their
//!   own request jobs, failing with the low-level client's errors;
//! - [`export`]: an inventory in another model's terms, so far an
//!   experimental Standard 223 point list in Turtle (#2436);
//! - [`arrival`]: every I-Am this node receives, one [`IAmArrival`] per
//!   frame from the transport's frame tap ([`IAmArrivals`]), with its
//!   arrival time and who put it on the link: the device, a forwarding
//!   BBMD or a router;
//! - [`watch`]: watching points: [`Client::watch`] gives a [`Watch`], a
//!   stream of each point's [`Update`]s (its latest value, how it came, and
//!   how its watch goes, [`CovSupport`] included), over the watch hub, one
//!   managed COV subscription or poller per point however many listeners
//!   share it.
//!
//! The parts only the `bacnet` tool uses (the write-safety core, command
//! classes, detailed frame trees and the BBMD survey) are the `bacnet-tool`
//! crate (#2560).

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod arrival;
pub mod client;
pub mod decode;
pub mod device;
pub mod error;
pub mod export;
pub mod format;
pub mod inventory;
pub mod label;
pub mod limits;
pub mod octets;
pub mod parse;
pub mod raw;
pub mod reading;
pub mod target;
pub mod watch;
pub mod write;

pub use arrival::{IAmArrival, IAmArrivals};
pub use client::{Client, Discover, Find, ResolveScope, Scope};
pub use device::{Device, DeviceAddress};
pub use error::OpsError;
pub use format::{PropertyError, Value};
pub use inventory::{Inventory, InventoryOptions, PointRecord};
pub use reading::{IntoObject, IntoProperty, ObjectRead, PropertyRef, Reading, Readings};
pub use watch::{CovSupport, Update, Watch};
pub use write::{
    IntoWriteValue, Priority, PriorityView, WriteDenied, WritePolicy, WritePreview, Written,
};

/// The BACnet/SC TLS configuration [`Client::sc`] takes, and the error its
/// constructors return with the types its variants name, so an application
/// needs no direct `bacnet-transport` dependency to load the site's PEM files.
#[cfg(feature = "sc-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "sc-tls")))]
pub use bacnet_transport::sc_tls::{
    ScCertificateForm, ScKeyForm, ScNodeTlsConfig, ScPemFault, ScTlsConfigError, ScTlsInput,
};
