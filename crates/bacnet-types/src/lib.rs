//! BACnet protocol types per ASHRAE 135-2020.
//!
//! This crate provides the foundational types for the BACnet protocol:
//! enumerations, primitive data types, addresses, and error types.
//!
//! Its runtime dependencies are `bitflags`, `smallvec` and `thiserror`.
//!
//! Every enum newtype implements `FromStr`, accepting any common case style
//! (`analog-input`, `AnalogInput`, `ANALOG_INPUT`, ...) for its named
//! constants. The optional `serde` feature adds `Deserialize` for
//! [`enums::ObjectType`] and [`enums::PropertyIdentifier`] on top of it.
//!
//! # Features
//!
//! The crate needs an allocator, and one of these features to say so:
//!
//! - `std` (default): hosted builds. Adds the transport I/O error
//!   (`Error::Transport`) and the B/IP port contract (`bip_port`).
//! - `alloc`: `no_std` with a global allocator, for bare-metal targets:
//!   `default-features = false, features = ["alloc"]`.
//!
//! A build with neither fails; that combination is reserved for a future
//! allocation-free subset.

// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

// bacnet-encoding and bacnet-services pass their feature on to this crate, so
// with neither on they stop here, before their own check can run.
#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!(
    "bacnet-types needs the `std` or the `alloc` feature \
     (bacnet-encoding and bacnet-services pass theirs on)"
);

// `format!` and `vec!` without std's prelude.
#[macro_use]
extern crate alloc;

#[cfg(feature = "std")]
pub mod bip_port;
pub mod bitstring;
pub mod calendar;
pub mod constructed;
pub mod data_link;
pub mod enums;
pub mod error;
pub mod network_number;
pub mod primitives;

/// BACnet MAC address — stack-allocated for typical sizes (≤6 bytes).
///
/// The data links this stack serves use 1-octet (MS/TP), 3-octet (the
/// B/IPv6 VMAC) and 6-octet (B/IP, Ethernet, the B/SC VMAC) MACs, so SmallVec
/// keeps every one of them inline.
pub type MacAddr = smallvec::SmallVec<[u8; 6]>;
