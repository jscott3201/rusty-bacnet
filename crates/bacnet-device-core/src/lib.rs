#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
#![forbid(unsafe_code)]

// `alloc`'s collections without std. The library formats nothing, so a
// firmware image links no `core::fmt` on its account (#2187).
extern crate alloc;

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!(
    "bacnet-device-core needs the `std` or the `alloc` feature: the database it serves boxes every object"
);
#[cfg(not(target_has_atomic = "ptr"))]
compile_error!(
    "bacnet-device-core needs atomic compare-and-swap, as bacnet-objects does: its objects share state through alloc::sync::Arc"
);

pub mod clock;
pub mod cov;
pub mod discovery;
#[cfg(feature = "event-reporting")]
pub mod event;
pub mod read;
pub mod reply;
pub mod report;
pub mod responder;
pub mod write;

#[cfg(test)]
mod read_only_rows_tests;
#[cfg(test)]
mod served_datatypes_tests;
// The server's rows for test objects, as bacnet-endpoint shares them. The
// core's tests state no value types, so `typed` goes unused here.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../bacnet-server/src/test_rows.rs"]
mod test_rows;

pub use responder::SERVICES;
