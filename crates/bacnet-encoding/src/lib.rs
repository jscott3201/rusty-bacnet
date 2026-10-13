//! BACnet ASN.1/BER encoding and APDU/NPDU codecs per ASHRAE 135-2020.
//!
//! This crate provides the wire-format encoding and decoding for BACnet:
//! - ASN.1 tag encode/decode (Clause 20.2.1)
//! - Application-tagged primitive codecs (Clause 20.2)
//! - APDU encode/decode (Clause 20.1)
//! - NPDU encode/decode (Clause 6)
//!
//! # Features
//!
//! The codecs need an allocator, and one of these features to say so:
//!
//! - `std` (default): hosted builds. Also logs, through `tracing`, an NPDU
//!   whose reserved control bits are set.
//! - `alloc`: `no_std` with a global allocator, for bare-metal targets:
//!   `default-features = false, features = ["alloc"]`. `bytes` must build for
//!   the target; on one without atomic compare-and-swap, such as ARMv6-M,
//!   that means its `extra-platforms` feature and a `portable-atomic` backend
//!   the firmware picks.
//!
//! A build with neither fails; that combination is reserved for a future
//! allocation-free subset.

// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!("bacnet-encoding needs the `std` or the `alloc` feature");

// `format!` and `vec!` without std's prelude.
#[macro_use]
extern crate alloc;

pub mod apdu;
pub mod constructed;
pub mod npdu;
pub mod primitives;
pub mod segmentation;
pub mod tags;

#[cfg(test)]
mod tags_canonical_tests;
