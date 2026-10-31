#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
#![forbid(unsafe_code)]

// `format!` and `vec!` without std's prelude, as in bacnet-objects.
#[macro_use]
extern crate alloc;

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!(
    "bacnet-device-core needs the `std` or the `alloc` feature: the database it serves boxes every object"
);
#[cfg(not(target_has_atomic = "ptr"))]
compile_error!(
    "bacnet-device-core needs atomic compare-and-swap, as bacnet-objects does: its objects share state through alloc::sync::Arc"
);

pub mod discovery;
pub mod read;
pub mod reply;
