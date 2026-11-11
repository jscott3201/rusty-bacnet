//! The scenarios that the firmware link test's images run (#1626, #1635,
//! #1664, #1667, #1679, #1771).
//!
//! Each image links one stack into a bare-metal binary, all but the last
//! four with no allocator:
//!
//! - `mstp` ([`mstp::run`]): `bacnet-mstp-codec` and `bacnet-mstp-core`.
//! - `mstp-extended` ([`mstp_extended::run`]): the same core with extended
//!   frames on, and the buffers they need.
//! - `bip` ([`bip::run`]): `bacnet-bip-codec`, B/IP and B/IPv6.
//! - `node` ([`node::run`]): `bacnet-bip-core`'s port driver over its node
//!   engine, as a foreign device.
//! - `bbmd` ([`bbmd::run`]): `bacnet-bip-core`'s port driver on a BBMD: its
//!   BBMD tables, behind its receive rules, answering management requests
//!   themselves.
//! - `router` ([`router::run`]): `bacnet-router-core` with its B/IP and
//!   MS/TP glue, routing between a BBMD's B/IP port and an MS/TP port with
//!   extended frames, over bip-core's port driver and BBMD tables, the
//!   MS/TP core and both codecs.
//! - `device` ([`device::run`]): an MS/TP device that answers Who-Is and
//!   ReadProperty, the MS/TP core under `bacnet-npdu`'s view and the
//!   allocation-free profile of `bacnet-encoding` and `bacnet-services`.
//! - `apdu` (`apdu::run`, with the `alloc` feature): `bacnet-encoding` and
//!   `bacnet-services`, `no_std` with `alloc`, and a global allocator over a
//!   fixed arena.
//! - `objects` (`objects::run`, with the `objects` feature):
//!   `bacnet-device-core`'s `Responder` answering from a `bacnet-objects`
//!   database, both `no_std` with `alloc`, over a fixed arena. Both need
//!   atomic compare-and-swap, so it builds for thumbv7m, thumbv7em and
//!   riscv32imac only.
//! - `reporting` (`reporting::run`, with the `reporting` feature): the
//!   objects image's database with `bacnet-device-core`'s COV `Reporter`
//!   and Device clock, on the objects image's targets.
//! - `points` (`points::run`, with the `points` feature): the objects
//!   image's requests answered from a database `bacnet-objects`' point-list
//!   builder makes, one point of each of the nine types it covers among
//!   them, on the objects image's targets. Its size less the objects
//!   image's is what the builder costs.
//!
//! One image per stack keeps each stack's flash and RAM separate: within one
//! LTO image, code can't be attributed to a crate reliably. So `bip` links
//! the codec alone, and `node` and `bbmd` each link the codec with their
//! part of `bacnet-bip-core`. `router` links all of those that a router
//! between the two links uses, so its size is the whole firmware's.
//!
//! A scenario is one `no_std` function that returns a summary of what it
//! saw, and each module holds the summary of a good run: [`mstp::EXPECTED`],
//! [`mstp_extended::EXPECTED`], [`bip::EXPECTED`], [`node::expected()`],
//! [`bbmd::EXPECTED`], [`router::expected()`], `device::expected()`,
//! `apdu::expected()`, `objects::expected()`, `reporting::expected()` and
//! `points::expected()`. The
//! host tests in `tests/` run each scenario natively and compare the whole
//! summary with it; that is where behaviour is checked. Each image
//! (`src/bin/`) calls its scenario from its reset entry, so building it
//! proves that the stack links without `std`, and for all but the last four
//! without an allocator. CI never runs the images. Their emulation build,
//! with the `emulate` feature and a board's, runs under QEMU by hand: each
//! image checks its summary against the same expected one and reports
//! through semihosting (docs/ci.md, "Emulation (manual)").
//!
//! The `alloc` feature, on by default, brings in the `apdu` module and its
//! dependencies, and the `objects` feature, also on by default, the
//! `objects` module and its own; `reporting` and `points`, on by default
//! too, each add their module to `objects`. The other images build without
//! any of them: with `alloc`, this crate links `alloc`, and an image
//! without a global allocator fails to link. The `objects`, `reporting` and
//! `points` images have no emulation build yet.
//!
//! The crate is `no_std` on every target, so the host build compiles the
//! same code as the images.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
pub mod apdu;
pub mod bbmd;
pub mod bip;
pub mod device;
pub mod mstp;
pub mod mstp_extended;
pub mod node;
#[cfg(feature = "objects")]
pub mod objects;
#[cfg(feature = "points")]
pub mod points;
#[cfg(feature = "reporting")]
pub mod reporting;
pub mod router;
mod tally;

pub use tally::Sent;

/// Stop where the panic happened. The handler never reads the panic's
/// message, so the formatting code behind it can be left out of an image.
/// The host tests get std's handler instead, and the emulation build the
/// images' own, which reports the panic through semihosting.
#[cfg(all(target_os = "none", not(feature = "emulate")))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
