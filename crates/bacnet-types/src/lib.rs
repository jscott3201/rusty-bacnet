#![doc = include_str!("../README.md")]
// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
#![forbid(unsafe_code)]

// `format!` and `vec!` without std's prelude. Without `alloc` the crate
// links no allocator at all (#1614).
#[cfg(feature = "alloc")]
#[macro_use]
extern crate alloc;

pub mod bip6_vmac;
// Its shared views are `Arc`s, which need atomic compare-and-swap (#1615).
#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
pub mod bip_port;
pub mod bitstring;
// Matches the constructed calendar types, which need alloc.
#[cfg(feature = "alloc")]
pub mod calendar;
#[cfg(feature = "alloc")]
pub mod constructed;
pub mod data_link;
pub mod enums;
pub mod error;
pub mod map_hash;
pub mod network_number;
pub mod primitives;
pub mod text;

/// BACnet MAC address — stack-allocated for typical sizes (≤6 bytes).
///
/// The data links this stack serves use 1-octet (MS/TP), 3-octet (the
/// B/IPv6 VMAC) and 6-octet (B/IP, Ethernet, the B/SC VMAC) MACs, so SmallVec
/// keeps every one of them inline. Only with `alloc`.
#[cfg(feature = "alloc")]
pub type MacAddr = smallvec::SmallVec<[u8; 6]>;

/// The BACnet protocol version this stack implements: the Device object's
/// Protocol_Version (Clause 12.11.12).
///
/// The Device object and the PICS generator both take it from here, so they
/// can't disagree (#2125).
pub const PROTOCOL_VERSION: u16 = 1;

/// The BACnet protocol revision this stack implements: the Device object's
/// Protocol_Revision (Clause 12.11.13).
///
/// The stack implements ASHRAE 135-2020, which the standard's History of
/// Revisions lists as version 1, revision 22. Later addenda raise the
/// revision; they are not implemented as a whole, so the stack doesn't claim
/// them.
pub const PROTOCOL_REVISION: u16 = 22;
