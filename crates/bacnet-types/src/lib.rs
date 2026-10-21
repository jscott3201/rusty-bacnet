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

#[cfg(feature = "std")]
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

/// BACnet MAC address — stack-allocated for typical sizes (≤6 bytes).
///
/// The data links this stack serves use 1-octet (MS/TP), 3-octet (the
/// B/IPv6 VMAC) and 6-octet (B/IP, Ethernet, the B/SC VMAC) MACs, so SmallVec
/// keeps every one of them inline. Only with `alloc`.
#[cfg(feature = "alloc")]
pub type MacAddr = smallvec::SmallVec<[u8; 6]>;
