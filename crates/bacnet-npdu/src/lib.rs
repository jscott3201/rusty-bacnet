//! The BACnet network-layer vocabulary (Clause 6), in one place, for every
//! layer of rusty-bacnet and for firmware.
//!
//! The crate is `no_std`, needs no allocator and has no dependencies. It
//! holds what the portable cores, the hosted transports and network layer,
//! and the owned codec of `bacnet-encoding` all speak, so each concept has
//! one definition:
//!
//! - [`Npdu`]: a borrowed view of an NPDU's control information and its
//!   payload. [`Npdu::decode`] reads one in place, telling an address too
//!   long for any data link ([`DecodeError::AddressTooLong`]) apart from
//!   every other fault ([`Malformed`]); [`Npdu::encoded_len`] and
//!   [`Npdu::encode_into`] write one into the caller's buffer, refusing
//!   anything the decoder would refuse before writing an octet.
//! - [`NetworkMessageType`] and [`RejectMessageReason`]: the open sets of
//!   wire codes for network-layer messages and their reject reasons.
//! - [`RejectMessageToNetwork`]: a reject's data, its reason and network,
//!   read and written in place, refused with [`RejectLengthError`] when it
//!   isn't three octets.
//! - [`NetworkPriority`]: the closed, two-bit network priority.
//! - [`NpduLimits`]: the longest NPDU a link carries each way, and
//!   [`MAX_NPDU`], the longest any data link here carries.
//! - [`Mac`]: a link-layer address of up to [`MAX_MAC_LEN`] octets, stored
//!   inline.
//!
//! The other network-layer messages' payloads, the routing table and the
//! router itself are `bacnet-router-core`'s.
//!
//! # Example
//!
//! ```
//! use bacnet_npdu::{Address, Npdu, NetworkPriority};
//!
//! // A confirmed request for station 5 on network 2, at urgent priority.
//! let mut request = Npdu::apdu(&[0x00, 0x05, 0x01, 0x0C]);
//! request.destination = Some(Address { network: 2, mac: &[5] });
//! request.expecting_reply = true;
//! request.priority = NetworkPriority::Urgent;
//!
//! let mut frame = [0u8; 32];
//! let len = request.encode_into(&mut frame)?;
//! assert_eq!(len, request.encoded_len());
//! assert_eq!(&frame[..len], &[0x01, 0x25, 0x00, 0x02, 0x01, 0x05, 0xFF, 0x00, 0x05, 0x01, 0x0C]);
//!
//! let decoded = Npdu::decode(&frame[..len]).expect("an NPDU");
//! assert_eq!(decoded.destination, request.destination);
//! assert_eq!(decoded.priority, NetworkPriority::Urgent);
//! assert_eq!(decoded.payload, request.payload);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_debug_implementations)]
#![warn(clippy::must_use_candidate)]

mod codes;
mod encode;
mod limits;
mod mac;
mod priority;
mod reject;
mod view;

pub use codes::{NetworkMessageType, RejectMessageReason};
pub use encode::EncodeError;
pub use limits::NpduLimits;
pub use mac::{Mac, MacTooLong};
pub use priority::{NetworkPriority, PriorityError};
pub use reject::{RejectLengthError, RejectMessageToNetwork};
pub use view::{Address, AddressField, DecodeError, Malformed, Npdu};

/// The protocol version every NPDU starts with.
pub const VERSION: u8 = 1;

/// The DNET of a global broadcast.
pub const GLOBAL_NETWORK: u16 = 0xFFFF;

/// The longest NPDU, in octets, that B/IP, B/IPv6, Ethernet and MS/TP with
/// extended frames carry (Table 6-1; Clause 9.3 for MS/TP).
pub const MAX_NPDU: usize = 1497;

const _: () = assert!(MAX_NPDU <= u16::MAX as usize, "NpduLimits holds MAX_NPDU");

/// The longest DADR, SADR or link MAC the stack accepts, in octets (#1141).
///
/// A length octet could announce up to 255. No standard data link in
/// Table 6-2 needs more than 7 (a LonTalk Neuron_ID destination), and the
/// links this stack serves address a node in at most 6: 1 for MS/TP, 3 for
/// B/IPv6 (its VMAC, Annex U.1.3) and 6 for B/IP, Ethernet and BACnet/SC.
/// The bound is 18, the length of the IPv6 address and port the stack once
/// used as the B/IPv6 MAC: a defensive ceiling, which lets a router carry a
/// non-standard MAC through unchanged, and which stays where it is by
/// decision (#1673).
///
/// [`Npdu::decode`] refuses a longer DLEN or SLEN, [`Npdu::encode_into`]
/// a longer DADR or SADR, and [`Mac`] holds no more.
pub const MAX_MAC_LEN: usize = 18;

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
