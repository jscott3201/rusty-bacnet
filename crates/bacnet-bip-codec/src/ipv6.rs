//! The B/IPv6 BVLL codec: BACnet/IPv6 messages (Annex U).
//!
//! It covers the wire format and nothing above it: the 4-octet BVLL header
//! with BVLC type 0x82, the twelve messages Annex U.2 defines, 3-octet
//! virtual MAC addresses ([`Bip6Vmac`], U.1.3 and H.7.2) and 18-octet B/IPv6
//! addresses ([`Bip6Address`], U.1.1.2). Sockets, multicast groups, the VMAC
//! table, BBMD forwarding and foreign-device registration belong to the
//! layer that uses it.
//!
//! It follows the B/IP codec at the crate root, and shares its
//! [`BvllSink`](crate::BvllSink), its [`PayloadShape`](crate::PayloadShape)
//! and its header constants ([`HEADER_LENGTH`](crate::HEADER_LENGTH),
//! [`MAX_BVLL_LENGTH`](crate::MAX_BVLL_LENGTH)). Nothing here allocates:
//! [`Bvll6`] borrows its NPDU from the datagram, and
//! [`Bvll6::encode_into`] writes into caller storage.
//!
//! # Validation
//!
//! Decoding is strict about shape and lenient about values.
//! [`Bvll6Frame::decode`] requires BVLC type 0x82 and a Length field equal
//! to the datagram's length, since it counts the whole message (U.2).
//! [`Bvll6Frame::message`] then requires a function U.2 defines and a
//! payload of the length its format clause gives (see
//! [`Bvlc6Function::payload_shape`]). It does not judge field values: any
//! VMAC, any address, an undefined result code, a zero time-to-live and an
//! empty NPDU all decode. [`decode`] runs both steps.
//!
//! Function 0x0B is unknown here: the clause that once defined it (U.2.12)
//! is deleted in this edition.
//!
//! Encoding is checked. Each [`Bvll6`] variant holds exactly its function's
//! fields, so most malformed shapes can't be built;
//! [`Bvll6::encoded_len`] also refuses a message longer than the Length
//! field can state and a BVLC-Result with a code U.2.1.1 doesn't define.
//!
//! # Addresses
//!
//! Every B/IPv6 message starts its payload with a VMAC
//! ([`Bvll6::source_vmac`]): the sender's, except in the two messages a
//! BBMD relays, Forwarded-NPDU and Forwarded-Address-Resolution, where it is
//! the originating node's. A [`Bip6Address`] holds the IPv6 address and
//! UDP port that travel, and nothing else: a socket address's flow label
//! and scope ID never cross the wire, so a receiver decides a link-local
//! address's scope itself.
//!
//! # Example
//!
//! ```
//! use bacnet_bip_codec::ipv6::{decode, Bip6Vmac, Bvlc6Function, Bvll6, Bvll6Error};
//!
//! let npdu = [0x01, 0x00, 0x10, 0x08];
//! let message = Bvll6::OriginalUnicastNpdu {
//!     source: Bip6Vmac::new([0x00, 0x04, 0xD2]),
//!     destination: Bip6Vmac::new([0x00, 0x16, 0x2E]),
//!     npdu: &npdu,
//! };
//! let mut wire = [0u8; 64];
//! let length = message.encode_into(&mut wire)?;
//! assert_eq!(wire[..10], [0x82, 0x01, 0x00, 0x0E, 0x00, 0x04, 0xD2, 0x00, 0x16, 0x2E]);
//!
//! let decoded = decode(&wire[..length])?;
//! assert_eq!(decoded, message);
//! assert_eq!(decoded.function(), Bvlc6Function::ORIGINAL_UNICAST_NPDU);
//! assert_eq!(decoded.destination_vmac(), Some(Bip6Vmac::new([0x00, 0x16, 0x2E])));
//! assert_eq!(decoded.npdu(), Some(&npdu[..]));
//! # Ok::<(), Bvll6Error>(())
//! ```

mod address;
mod error;
mod function;
mod message;

pub use address::{Bip6Address, Bip6Vmac};
pub use error::Bvll6Error;
pub use function::{Bvlc6Function, Bvlc6ResultCode};
pub use message::{Bvll6, Bvll6Frame, decode};

/// The BVLC Type octet of a B/IPv6 message (Annex U.2).
pub const BVLC_TYPE_BIP6: u8 = 0x82;
/// Octets in a B/IPv6 virtual MAC address (Annex U.1.3, H.7.2).
pub const VMAC_LENGTH: usize = 3;
/// Octets in a B/IPv6 address: IPv6 address and UDP port (Annex U.1.1.2).
pub const BIP6_ADDRESS_LENGTH: usize = 18;
