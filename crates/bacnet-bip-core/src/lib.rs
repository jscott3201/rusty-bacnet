//! Portable BACnet/IP behaviour (Annex J), above the wire format of
//! `bacnet-bip-codec`.
//!
//! The crate is `no_std` and needs no allocator: tables live in arrays whose
//! size the caller picks with a const generic, results are written into
//! storage the caller passes, and time is an [`Instant`] the caller supplies
//! with each call. It reads no clock and opens no socket, so the same code
//! runs on firmware and under the Tokio B/IP transport of
//! `bacnet-transport`, which drives it.
//!
//! [`Instant`] and [`Duration`] are `bacnet-time`'s, in nanoseconds,
//! re-exported here: the types `bacnet-mstp-core` and `bacnet-router-core`
//! take too, so firmware that runs them all feeds them one clock. The codec
//! is re-exported as [`codec`], since the API speaks its types
//! ([`Bvll`](codec::Bvll), [`BdtEntries`](codec::BdtEntries) and the rest).
//!
//! - [`bbmd`]: a BBMD's two tables (the BDT and the FDT), foreign-device
//!   registration, and where each broadcast is forwarded (Annex J.4, J.5).
//! - [`node`]: a sans-IO B/IP node in normal, foreign-device and BBMD
//!   mode: its receive rules and NAKs, what it sends, its own management
//!   exchanges, and a foreign device's registration lifecycle (Annex J.2,
//!   J.4, J.5).
//! - [`port`]: the port driver that joins a node to a socket and carries
//!   out what it asks: [`NodePort`](port::NodePort) for a node in normal or
//!   foreign-device mode, and [`BbmdPort`](port::BbmdPort) for a BBMD,
//!   which forwards broadcasts and answers management requests from its
//!   tables.
//! - [`Arrival`]: whether a datagram was addressed to the node or to its
//!   subnet's broadcast address, which the rules above and a node's own
//!   both depend on, and [`LocalSubnet`], the subnet it reaches that way.
//!
//! The types that may grow are `#[non_exhaustive]`. Build a
//! [`Datagram`](node::Datagram), a [`Context`](bbmd::Context), a
//! [`LocalSubnet`] or a [`Registration`](bbmd::Registration) with its
//! `new`; start a [`Link`](node::Link) or a [`Foreign`](node::Foreign) from
//! its `new`, or the [`RegistrationLimits`](bbmd::RegistrationLimits) from
//! `DEFAULT`, and set the fields to change; and give a `match` on an
//! error, a refusal, a [`Discard`](node::Discard) or another of these
//! enums a wildcard arm. The enums a host acts on case by case, such as
//! [`Action`](node::Action) and [`Received`](node::Received), stay
//! exhaustive.
//!
//! # Example
//!
//! A normal node's link takes a unicast NPDU from a peer, and says what to
//! send for a broadcast:
//!
//! ```
//! use core::net::{Ipv4Addr, SocketAddrV4};
//! use bacnet_bip_core::codec::Bvll;
//! use bacnet_bip_core::node::{Addressed, Datagram, Link, Mode, Permissive, Received};
//! use bacnet_bip_core::{Instant, LocalSubnet};
//!
//! let own = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 47808);
//! let subnet = LocalSubnet::new(SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 47808));
//! let link = Link::new(own, subnet, Mode::Normal)?;
//!
//! // A peer's Original-Unicast-NPDU, sent to this node's own address.
//! let npdu = [0x01, 0x00, 0x10, 0x08];
//! let mut wire = [0u8; 8];
//! let length = Bvll::OriginalUnicastNpdu(&npdu).encode_into(&mut wire)?;
//! let peer = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 20), 47808);
//! let to = Addressed { node: true, broadcast: false };
//! let datagram = Datagram::new(&wire[..length], peer, Some(to));
//! let Received::Deliver(delivery) = link.receive(&datagram, Instant::ZERO, &mut Permissive)
//! else {
//!     panic!("delivered");
//! };
//! assert_eq!((delivery.npdu, delivery.source, delivery.group), (&npdu[..], peer, false));
//!
//! // A broadcast goes to the subnet's broadcast address.
//! let (outgoing, forward_too) = link.broadcast(&npdu)?;
//! assert_eq!(outgoing.to, subnet.broadcast);
//! assert_eq!(outgoing.message, Bvll::OriginalBroadcastNpdu(&npdu));
//! assert!(!forward_too, "only a BBMD forwards its own broadcasts");
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! # Time
//!
//! A `now` earlier than the latest one passed in is taken as that latest
//! one: time never runs backwards, so a clock that steps back holds the
//! timers still instead of failing. Here that is the [`node::Node`]; the
//! BBMD tables keep no clock of their own, and count a `now` before a
//! registration's start as no time passed.
//!
//! # Features
//!
//! None. The crate depends only on `bacnet-bip-codec`, with its default
//! features, and `bacnet-time`, and builds on bare-metal targets without an
//! allocator.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_debug_implementations)]
#![warn(clippy::must_use_candidate)]

mod arrival;
pub mod bbmd;
pub mod node;
pub mod port;
mod subnet;

pub use arrival::Arrival;
pub use bacnet_bip_codec as codec;
pub use bacnet_time::{Duration, Instant};
pub use subnet::LocalSubnet;

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
#[derive(Debug)]
pub struct ReadmeDoctests;
