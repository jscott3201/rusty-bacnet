//! BACnet protocol types per ASHRAE 135-2020.
//!
//! This crate provides the foundational types for the BACnet protocol:
//! enumerations, primitive data types, addresses, and error types.
//!
//! It has zero runtime dependencies beyond `bitflags` and `thiserror`,
//! and supports `no_std` environments via the `std` feature flag.
//!
//! Every enum newtype implements `FromStr`, accepting any common case style
//! (`analog-input`, `AnalogInput`, `ANALOG_INPUT`, ...) for its named
//! constants. The optional `serde` feature adds `Deserialize` for
//! [`enums::ObjectType`] and [`enums::PropertyIdentifier`] on top of it.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub mod bitstring;
pub mod constructed;
pub mod enums;
pub mod error;
pub mod primitives;

/// BACnet MAC address — stack-allocated for typical sizes (≤6 bytes).
///
/// BACnet MAC addresses are 1 byte (MS/TP), 6 bytes (Ethernet/IP),
/// 3 bytes (SC VMAC), or 18 bytes (BIP6 IP:port). SmallVec avoids
/// heap allocation for the common cases.
pub type MacAddr = smallvec::SmallVec<[u8; 6]>;
