#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Redundant with the workspace lint, but explicit: only `sys` may allow it
// (#1923).
#![deny(unsafe_code)]

/// Items for the targets whose libc has `getifaddrs`: the one walk over it
/// (`sys::ifaddrs`, #1884) and its callers.
macro_rules! with_getifaddrs {
    ($($item:item)*) => {
        $(
            #[cfg(any(
                target_os = "linux",
                target_os = "l4re",
                target_os = "android",
                target_os = "emscripten",
                target_vendor = "apple",
                target_os = "freebsd",
                target_os = "dragonfly",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "solaris",
                target_os = "illumos",
                target_os = "haiku",
                target_os = "nto",
                target_os = "hurd",
                target_os = "fuchsia",
            ))]
            $item
        )*
    };
}

/// Items for the targets with no interface list: neither `getifaddrs` nor
/// Windows.
macro_rules! without_interface_list {
    ($($item:item)*) => {
        $(
            #[cfg(not(any(
                windows,
                target_os = "linux",
                target_os = "l4re",
                target_os = "android",
                target_os = "emscripten",
                target_vendor = "apple",
                target_os = "freebsd",
                target_os = "dragonfly",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "solaris",
                target_os = "illumos",
                target_os = "haiku",
                target_os = "nto",
                target_os = "hurd",
                target_os = "fuchsia",
            )))]
            $item
        )*
    };
}

pub mod any;
pub mod bbmd;
pub mod bip;
#[cfg(feature = "ipv6")]
#[cfg_attr(docsrs, doc(cfg(feature = "ipv6")))]
pub mod bip6;
pub mod bvll;
mod direct_response;
#[cfg(feature = "ethernet")]
#[cfg_attr(docsrs, doc(cfg(feature = "ethernet")))]
pub mod ethernet;
mod local_addresses;
pub mod loopback;
pub mod mstp;
#[cfg(feature = "serial")]
#[cfg_attr(docsrs, doc(cfg(feature = "serial")))]
pub mod mstp_serial;
mod npdu_channel;
pub mod port;
mod port_ownership;
mod principal;
mod runtime_clock;
pub mod sc;
pub mod sc_frame;
#[cfg(feature = "sc-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "sc-tls")))]
pub mod sc_hub;
mod sc_limits;
#[cfg(feature = "sc-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "sc-tls")))]
pub mod sc_tls;
mod sys;
pub mod tap;
#[cfg(feature = "sc-tls")]
mod tcp_connect;
#[cfg(feature = "sc-tls")]
mod tls_reject;
mod udp_metadata;
pub mod warn_throttle;

#[cfg(test)]
mod log_capture;
#[cfg(test)]
#[path = "rb07_provenance_tests.rs"]
mod rb07_provenance_tests;
