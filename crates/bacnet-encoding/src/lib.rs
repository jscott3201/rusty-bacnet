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

/// Items only the `alloc` profile has: each gets `#[cfg(feature = "alloc")]`.
macro_rules! alloc_only {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "alloc")]
            $item
        )*
    };
}

pub mod apdu;
pub mod constructed;
#[cfg(feature = "alloc")]
pub mod npdu;
pub mod primitives;
#[cfg(feature = "alloc")]
pub mod segmentation;
pub mod sink;
pub mod tags;

#[cfg(all(test, feature = "alloc"))]
mod tags_canonical_tests;
