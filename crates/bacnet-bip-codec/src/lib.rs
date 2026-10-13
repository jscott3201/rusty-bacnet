//! An allocation-free codec for BACnet/IP BVLL messages, on IPv4 (Annex J)
//! at the crate root and on IPv6 (Annex U) in [`ipv6`].
//!
//! The crate covers the wire format and nothing above it. For B/IP that is
//! the 4-octet BVLL header, the twelve messages Annex J.2 defines, six-octet
//! B/IP addresses (J.1.2), and the entries of BDTs and FDTs; [`ipv6`] does
//! the same for the twelve messages of Annex U.2, with B/IPv6 addresses and
//! virtual MAC addresses. Sockets, BBMD forwarding, the foreign-device
//! registration lifecycle and which device answers what belong to the layer
//! that uses it. Both message families share the [`BvllSink`] trait and the
//! header constants.
//!
//! The default build never allocates. [`Bvll`] borrows its NPDU or table
//! from the datagram, tables are read through iterators
//! ([`Entries::iter`]), and [`Bvll::encode_into`] writes into caller
//! storage. Only the optional `alloc` and `bytes` sinks grow on the heap.
//!
//! # Validation
//!
//! Decoding is strict about shape and lenient about values.
//! [`BvllFrame::decode`] requires BVLC type 0x81 and a Length field equal to
//! the datagram's length: the field counts the whole message (J.2), so a
//! truncated datagram and one with trailing octets are both refused.
//! [`BvllFrame::message`] then requires a function J.2 defines and a payload
//! of the length its format clause gives (see [`BvlcFunction::payload_shape`]).
//! It does not judge field values: an undefined result code, a zero
//! time-to-live, any table entry and an empty NPDU all decode, and the
//! receiver decides what to do with them. [`decode`] runs both steps.
//!
//! Function 0x0C is unknown here: the clause that once defined it (J.2.13)
//! is deleted in this edition.
//!
//! Encoding is checked. A [`Bvll`] variant holds exactly its function's
//! fields, so most malformed shapes can't be built; [`Bvll::encoded_len`]
//! also refuses a message longer than the Length field can state and a
//! BVLC-Result with a code J.2.1.1 doesn't define.
//!
//! # Addresses
//!
//! A B/IP address is a plain [`SocketAddrV4`](core::net::SocketAddrV4), and
//! [`address_to_octets`], [`address_from_octets`] and
//! [`address_from_slice`] convert it to and from its six wire octets. Every
//! field of a `SocketAddrV4` travels, so a type of its own would add
//! nothing. B/IPv6 does get one, [`ipv6::Bip6Address`]: a `SocketAddrV6`
//! also holds a flow label and a scope ID, which never go on the wire, and
//! the type keeps only the 18 octets that do.
//!
//! # Features
//!
//! No feature needs `std`. Each adds a [`BvllSink`] implementation, which
//! both encoders use:
//!
//! - `alloc`: for `alloc::vec::Vec<u8>`.
//! - `bytes`: for `bytes::BytesMut`; implies `alloc`.
//! - `heapless`: for `heapless::Vec<u8, N>`.
//!
//! # Example
//!
//! ```
//! use bacnet_bip_codec::{decode, Bvll, BvllError, BvlcFunction};
//!
//! let npdu = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF];
//! let message = Bvll::OriginalBroadcastNpdu(&npdu);
//! let mut wire = [0u8; 64];
//! let length = message.encode_into(&mut wire)?;
//! assert_eq!(wire[..4], [0x81, 0x0B, 0x00, 0x0A]);
//!
//! let decoded = decode(&wire[..length])?;
//! assert_eq!(decoded, message);
//! assert_eq!(decoded.function(), BvlcFunction::ORIGINAL_BROADCAST_NPDU);
//! assert_eq!(decoded.npdu(), Some(&npdu[..]));
//! # Ok::<(), BvllError>(())
//! ```

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, clippy::must_use_candidate)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod error;
mod function;
pub mod ipv6;
mod message;
mod sink;
mod table;

pub use error::BvllError;
pub use function::{BvlcFunction, BvlcResultCode, PayloadShape};
pub use message::{decode, Bvll, BvllFrame};
pub use sink::{BvllSink, SinkFull};
pub use table::{
    address_from_octets, address_from_slice, address_to_octets, BdtEntries, BdtEntry, Entries,
    EntryIter, FdtEntries, FdtEntry, TableEntry,
};

/// The BVLC Type octet of a B/IP message (Annex J.2).
pub const BVLC_TYPE_BIP: u8 = 0x81;
/// The BVLL header: type, function and the two Length octets. B/IP and
/// B/IPv6 share it (Annex J.2, U.2).
pub const HEADER_LENGTH: usize = 4;
/// The largest message the 16-bit Length field can state, header included,
/// in either annex.
pub const MAX_BVLL_LENGTH: usize = u16::MAX as usize;
/// Octets in a B/IP address: IPv4 address and UDP port (Annex J.1.2).
pub const BIP_ADDRESS_LENGTH: usize = 6;
/// Octets in a broadcast distribution table entry: B/IP address and mask
/// (Annex J.2.2.1).
pub const BDT_ENTRY_LENGTH: usize = 10;
/// Octets in a foreign device table entry: B/IP address, time-to-live and
/// seconds remaining (Annex J.2.8.1).
pub const FDT_ENTRY_LENGTH: usize = 10;
/// The default UDP port, 47808 (0xBAC0), for B/IP and B/IPv6 alike (Annex
/// J.1.2, U.1.1.2).
pub const DEFAULT_PORT: u16 = 0xBAC0;

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;
