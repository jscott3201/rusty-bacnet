#![doc = include_str!("../README.md")]
// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]
#![forbid(unsafe_code)]

// Without `alloc` the crate links no allocator at all (#1614). The library
// builds no `format!` text (#1802).
#[cfg(feature = "alloc")]
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

// Who-Is, I-Am and ReadProperty encode and decode without an allocator.
pub mod read_property;
pub mod who_is;

alloc_only! {
    pub mod alarm_event;
    pub mod alarm_summary;
    pub mod audit;
    pub mod common;
    pub mod cov;
    mod cov_decode;
    pub mod cov_multiple;
    pub mod device_mgmt;
    pub mod enrollment_summary;
    pub mod file;
    pub mod life_safety;
    pub mod list_manipulation;
    pub mod object_mgmt;
    pub mod private_transfer;
    pub mod read_range;
    pub mod rpm;
    pub mod structured_error;
    pub mod text_message;
    pub mod virtual_terminal;
    pub mod who_am_i;
    pub mod who_has;
    pub mod wpm;
    pub mod write_group;
    pub mod write_property;
}

#[cfg(all(test, feature = "alloc"))]
mod malformed_member_tests;
#[cfg(all(test, feature = "alloc"))]
mod reject_reason_tests;
#[cfg(all(test, feature = "alloc"))]
mod truncated_member_tests;
