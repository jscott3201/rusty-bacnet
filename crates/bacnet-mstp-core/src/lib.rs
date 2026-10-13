//! A synchronous, platform-neutral BACnet MS/TP master core.
//!
//! The core owns no clock, serial port, executor, lock, or system resource.
//! Callers supply timestamped events and execute the actions returned by
//! [`MasterCore::poll_action`].
//!
//! Time is an [`Instant`] in nanoseconds from an epoch the caller chooses,
//! with a [`Duration`] for lengths: `bacnet-time`'s types, re-exported here.
//! `bacnet-bip-core` and `bacnet-router-core` take the same ones, so firmware
//! that runs several of them feeds them one clock. A microsecond tick becomes
//! an instant with [`Instant::from_micros`]. The frame codec,
//! `bacnet-mstp-codec`, is re-exported as [`codec`], since the API speaks
//! its types ([`FrameType`](codec::FrameType), [`Frame`](codec::Frame),
//! [`FrameError`](codec::FrameError)).
//!
//! The types that may grow are `#[non_exhaustive]`. Start a
//! [`MstpCoreConfig`] from [`MstpCoreConfig::STANDARD`] or
//! [`MstpCoreConfig::for_baud_rate`] and set the fields to change; a
//! [`QueueStorage`] of your own builds its items with
//! [`DequeuedNpdu::new`] and [`QueueItemInfo::new`]; and give a `match` on
//! an error, a refusal, a [`ReceiveError`] or a [`TransmitFailure`] a
//! wildcard arm. The enums an adapter acts on case by case,
//! [`Action`], [`ReplyDecision`] and [`Direction`], stay exhaustive, as do
//! the closed sets [`MasterState`], [`NetworkPriority`] and [`QueueKind`].
//!
//! # Time
//!
//! A `now` earlier than the latest one passed in is taken as that latest
//! one: time never runs backwards, so a clock that steps back holds the
//! timers still instead of failing. [`MasterCore::enqueue_npdu`] takes its
//! `now` the same way, though a later one it is given doesn't move the
//! core's clock: only the line, deadline, transmit and request calls do.
//!
//! # Counters
//!
//! [`MasterCore::take_counters`] returns what the core counted since the
//! last take, its queues' counts with it, and starts the counts again from
//! zero. Each is a `u32` that saturates between takes; add each take to
//! totals of your own, as wide as you need. The core keeps no totals, so a
//! firmware image carries no wider integers than it uses; the Tokio MS/TP
//! transport in `bacnet-transport` keeps `u64`s. [`MasterCore::counters`]
//! shows the same counts without taking them.
//!
//! # The adapter's side
//!
//! [`MasterCore`] describes the calls in full. Three rules matter most:
//!
//! - **Poll after every call.** After each call into the core, poll
//!   [`MasterCore::poll_action`] until [`Action::Idle`], and always before
//!   reporting the next transmit event. Control actions can't run out of
//!   room, and a start is refused ([`CoreError::TransmitNotPolled`]) until
//!   its frame has been polled; received data waits in a fixed number of
//!   slots, and data that finds them full is dropped and counted.
//! - **A frame starts only on `Ok`.** Call
//!   [`MasterCore::on_transmit_started`] right before enabling the line
//!   driver, and drive the line only if it succeeds. It refuses a start that
//!   comes within T_turnaround of line activity
//!   ([`CoreError::TransmitDeferred`]), and the frame comes back later as a
//!   new [`Action::Transmit`] (Clause 9.5.5.1).
//! - **Echo begins at the confirmed start.** Until then, keep the driver
//!   off and the receiver on, and feed the core every octet and receive
//!   error, draining the receive FIFO or DMA buffer right before the start
//!   call: they are line activity. A `SetDirection(Receive)` that arrives
//!   before you confirmed a start means the core took the frame back; drop
//!   it. From the confirmed start to [`MasterCore::on_transmit_complete`],
//!   what the receiver hears is the station's own echo, which the core
//!   ignores; a frame it was part way through receiving is dropped then.
//!
//! An adapter on a host serial API, which hands over octets in chunks
//! without per-octet times, still feeds every octet to
//! [`MasterCore::on_octet`], stamped with the time of the read that returned
//! it: a stamp can only be late, which only defers the station's own sends,
//! and the core's own decoder then assembles every frame. That is what the
//! host MS/TP transport in `bacnet-transport` does, with a longer
//! T_frame_abort for chunk gaps. A backend that assembles frames in
//! hardware, and so has no octets to feed, reports them with
//! [`MasterCore::on_host_activity`], [`MasterCore::on_validated_frame`] and
//! [`MasterCore::on_invalid_frame`] instead; the same rules apply, and such
//! an assembler drops its own partial frame at a confirmed start.
//!
//! Line activity never makes the core abandon a frame it is waiting to send
//! on its own: the frame waits for the line to go quiet. Only the evidence
//! its state watches for, or a time limit taken from that state, drops it,
//! and the drop is counted.
//!
//! # Storage, and building the core on a microcontroller
//!
//! The core's large buffers belong to the caller: the receive storage, the
//! [`ActionBuffers`] (the frame being sent and the delivery slots) and,
//! optionally, the queues, which the core can borrow as `&mut`. The core
//! borrows them for its lifetime, so what it holds itself is a few hundred
//! octets, and [`MasterCore::new`] returns it cheaply even on a small stack.
//!
//! On firmware, keep the buffers together in one static whose initializer is
//! a `const fn` of zeros, so the linker puts it in `.bss` and no flash holds
//! an initial image; [`ActionBuffers::new`] and [`HeaplessQueues::new`]
//! are both all zero bits. Then build the core on the stack over that
//! storage. This needs no `unsafe` in the library and does not depend on the
//! compiler constructing anything in place:
//!
//! ```
//! # mod example {
//! use bacnet_mstp_core::codec::MAX_STANDARD_FRAME_LENGTH;
//! use bacnet_mstp_core::{ActionBuffers, HeaplessQueues, Instant, MasterCore, MstpCoreConfig};
//!
//! /// Everything large, zeroed. On firmware it lives in a static, borrowed
//! /// once at reset (for example through cortex-m-rt's `static mut`).
//! pub struct Storage {
//!     rx: [u8; MAX_STANDARD_FRAME_LENGTH],
//!     queues: HeaplessQueues<4, 2>,
//!     // One delivery slot: this adapter polls after every call.
//!     actions: ActionBuffers<1>,
//! }
//!
//! impl Storage {
//!     pub const fn new() -> Self {
//!         Self {
//!             rx: [0; MAX_STANDARD_FRAME_LENGTH],
//!             queues: HeaplessQueues::new(),
//!             actions: ActionBuffers::new(),
//!         }
//!     }
//! }
//!
//! pub fn run(storage: &mut Storage, config: MstpCoreConfig, now: Instant) {
//!     let Storage { rx, queues, actions } = storage;
//!     let Ok(mut core) = MasterCore::new(config, queues, rx, actions, now) else {
//!         return;
//!     };
//!     // Feed the core the line's events and carry out its actions.
//!     # let _ = core.poll_action();
//! }
//! # }
//! ```
//!
//! # Extended frames
//!
//! [`MstpCoreConfig::extended_frames`] turns on the extended frame types,
//! 32 and 33, which carry NPDUs of 502 to 1497 octets COBS-encoded. Clause
//! 9.3 requires them of a routing node and leaves them optional for any
//! other, so the flag is off by default, and a core without it behaves as
//! one built before it existed. With it on, every buffer must hold an
//! extended frame:
//! receive storage of
//! [`MAX_EXTENDED_FRAME_LENGTH`](codec::MAX_EXTENDED_FRAME_LENGTH),
//! [`ExtendedActionBuffers`], queues whose slots hold
//! [`MAX_EXTENDED_MPDU_DATA`](codec::MAX_EXTENDED_MPDU_DATA)
//! octets (such as `HeaplessQueues<U, B, MAX_EXTENDED_MPDU_DATA>`), and so a
//! `MasterCore<'_, Q, MAX_EXTENDED_MPDU_DATA>`. [`MasterCore::new`] refuses
//! smaller ones, naming the buffer. The sizes are const generics with
//! standard defaults, so a firmware image that leaves the flag off costs no
//! more RAM or flash than before: its buffers keep their standard sizes,
//! and the flag's branches are small.
//!
//! # A local policy
//!
//! When a NO_TOKEN deadline is serviced after this station's token
//! generation slot has closed, the core opens the window again a cycle of
//! slots later rather than wait for ever, which Clause 9.5.6.7 does not
//! provide for. Each token generated that way is counted in
//! [`MstpCounters::missed_generation_slots`]; see [`MasterCore`].
//!
//! # Queue storage, and the one feature
//!
//! [`HeaplessQueues`] keeps the transmit queues in arrays sized at compile
//! time, with no allocator and no feature. The `alloc` feature adds
//! `AllocQueues`, which reserves its capacity when built; no feature needs
//! `std`. A caller may also implement [`QueueStorage`] itself.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_debug_implementations, clippy::must_use_candidate)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(test)]
extern crate std;

mod actions;
mod config;
mod counters;
mod queue;
mod state;

pub use actions::{
    Action, ActionBuffers, Direction, ExtendedActionBuffers, RequestId, TransmitFailure,
    TransmitId, DEFAULT_DELIVERY_SLOTS,
};
pub use bacnet_mstp_codec as codec;
pub use bacnet_time::{Duration, Instant};
pub use config::{
    ConfigError, CoreBuffer, LocalDurationField, LocalLimitField, MstpCoreConfig, MAX_FRAME_ABORT,
    MAX_REPLY_TIMEOUT, MAX_USAGE_TIMEOUT, MIN_REPLY_TIMEOUT, MIN_USAGE_TIMEOUT, N_MIN_OCTETS,
    N_POLL, N_RETRY_TOKEN, T_NO_TOKEN, T_SLOT,
};
pub use counters::{MstpCounters, QueueCounters};
#[cfg(feature = "alloc")]
pub use queue::AllocQueues;
pub use queue::{
    DequeuedNpdu, HeaplessQueues, NetworkPriority, PriorityError, QueueItemInfo, QueueKind,
    QueueRefusal, QueueStorage,
};
pub use state::{
    CoreError, MasterCore, MasterSnapshot, MasterState, ReceiveError, ReplyDecision, LATE_DEADLINE,
    MIN_RX_STORAGE,
};

/// The README's example, compiled and run as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
#[derive(Debug)]
pub struct ReadmeDoctests;
