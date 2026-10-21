//! Firmware stand-in for the `no_std` lint on ARMv6-M (#1613); not published.
//!
//! `bytes` needs atomic compare-and-swap, which `thumbv6m-none-eabi` lacks.
//! There it takes its atomics from `portable-atomic`, whose backend only the
//! final firmware may choose. `scripts/ci/check-no-std.sh` lints each build
//! that uses `bytes` on that target with this crate in the graph: on that
//! target alone, its manifest turns on `bytes`' `extra-platforms` feature and
//! `portable-atomic`'s `critical-section` backend, and cargo unifies both into
//! the linted crate's build. The crate has no code of its own.
#![no_std]
#![forbid(unsafe_code)]
