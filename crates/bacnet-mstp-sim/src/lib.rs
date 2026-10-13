//! A deterministic, virtual-time MS/TP bus for testing
//! [`bacnet_mstp_core`] without hardware.
//!
//! A [`Sim`] puts several real [`MasterCore`](bacnet_mstp_core::MasterCore)s
//! on one simulated RS-485 line and drives each through its public API, the
//! way a platform adapter would. The token ring's Clause 9 behaviour can then
//! be tested end to end, in CI, on any host: ring formation, rotation,
//! maintenance polls, replies, joins and leaves, token loss, noise. Invariants
//! are checked after every event, and a failure stops the run with a
//! readable trace of what led up to it.
//!
//! ```
//! use bacnet_mstp_codec::FrameType;
//! use bacnet_mstp_core::Instant;
//! use bacnet_mstp_sim::{MasterSpec, Sim, SimConfig};
//!
//! let mut sim = Sim::new(SimConfig::new(38_400));
//! for station in [0, 3] {
//!     sim.add_master(MasterSpec::new(station, 3, 38_400));
//! }
//! sim.run_until(Instant::from_micros(1_000_000));
//! // Station 0's generation slot comes first, 500 ms into the silence.
//! let first = &sim.frames()[0];
//! assert!(first.is(FrameType::POLL_FOR_MASTER, 0, 1));
//! assert_eq!(first.start, Instant::from_micros(500_000));
//! sim.assert_no_violations();
//! ```
//!
//! # The model
//!
//! **Time and events.** Virtual time is the core's own
//! [`Instant`](bacnet_mstp_core::Instant), which counts nanoseconds; the
//! simulator's own times (octet boundaries, adapter jitter) are whole
//! microseconds of it. One queue orders every event by time, then by a fixed
//! class (characters arriving, transmissions ending, scheduled events, core
//! deadlines, adapters starting frames, later octets of a frame), then by
//! station address, then by scheduling order. Nothing reads a wall clock or
//! runs on another thread, so a run replays exactly from its seed on any
//! host.
//!
//! **The line.** One half-duplex medium at a configured baud rate. An octet
//! occupies ten bit times (Clause 9.2.3); a driver sends its octets back to
//! back, with boundaries measured from the start of its transmission and
//! rounded up to a whole microsecond. Receivers frame characters the way a
//! UART does: the first octet to begin opens a character that ends with that
//! octet. If no other driver was on the line meanwhile, receivers get the
//! octet; otherwise they get a framing error, and while out-of-step octets
//! remain they get further framing errors, one per octet time. Propagation is
//! ideal: every receiver sees the same line at the same instant.
//!
//! **Masters and their adapters.** Each master runs a core over
//! [`AllocQueues`](bacnet_mstp_core::AllocQueues). A master built with
//! [`MasterSpec::extended_frames`] takes and sends extended frames (Types
//! 32 and 33), as a routing node must; one without counts them as invalid
//! frames, as a node without COBS support does, so a ring can mix the two.
//! Every master's queues and action buffers are sized for extended frames
//! (see [`SimCore`]), so the simulator doesn't run the core's standard-size
//! instantiation that firmware uses; with the flag off the behaviour is the
//! same.
//! After every call into the core its adapter polls actions until the core
//! is idle:
//!
//! - It records [`Action::SetDirection`](bacnet_mstp_core::Action).
//! - For a transmit, after [`AdapterSpec::start_delay`] it calls
//!   `on_transmit_started` and, if the core agrees, drives the frame. It
//!   reports `on_transmit_complete` when the final stop bit has gone out and
//!   re-enables its receiver then. Until the start, its receiver stays on;
//!   if the core switches back to receive first, it has taken the frame
//!   back, and the adapter drops it.
//! - It hands NPDUs to the node's application, which answers requests per
//!   its [`ReplyPolicy`].
//!
//! The receiver feeds `on_octet` for each clean character and
//! `on_receive_error` for each garbled one, timestamped when the character
//! ends. It is off while the node drives the line, unless
//! [`AdapterSpec::echo`] is set. Deadlines are serviced at
//! `next_deadline`, plus any configured lateness and per-deadline jitter.
//!
//! **Scripted stations** decode the line with the codec's stream decoder
//! and react to frames through a [`Script`]: a minimal master that answers
//! polls and sends test frames, a slave, a misbehaving device.
//!
//! **Faults** ([`Fault`]): noise bursts, one damaged octet in a station's
//! next frame, power loss (at once, or part way through the next frame) and
//! power-up with a fresh core, and adapter timing changes.
//!
//! # Invariants
//!
//! Checked continuously; see [`ViolationKind`]:
//!
//! 1. **Frames decode.** Every frame a master sends decodes whole unless a
//!    fault touched it. While the run is not [perturbed](Sim::perturbed) (no
//!    fault or collision since the start or since it was
//!    [settled](Sim::settle)), every master that heard a clean frame whole
//!    counted it as received.
//! 2. **Turnaround.** A master's core hands a frame over, and its adapter
//!    starts it, no sooner than its Tturnaround after the last character on
//!    the line, its own included.
//! 3. **Collisions.** Every overlap of two drivers is explained or fails.
//!    Noise and a generation race are tolerated. A slow adapter is tolerated
//!    only under [`Tolerance::slow_adapter_collisions`], and only when its
//!    core handed the frame over before the other frame began. Scripted
//!    stations are tolerated only under [`Tolerance::scripted_collisions`].
//! 4. **One token holder.** At most one master is in USE_TOKEN,
//!    DONE_WITH_TOKEN, WAIT_FOR_REPLY or POLL_FOR_MASTER. A master in
//!    PASS_TOKEN has handed the token on and only watches for its successor
//!    to use it, so it does not count; two can be in PASS_TOKEN at once when
//!    a successor hands straight on. Without a fault there may never be two
//!    holders. After a fault, two may coexist for up to
//!    [`Tolerance::duplicate_token`] (zero by default). Masters that each
//!    generated a token themselves, having measured the silence from
//!    different instants, may race for up to [`Tolerance::generation_race`]
//!    (two seconds by default).
//! 5. **Bounded events.** Pending events stay within a small multiple of the
//!    node count, and the trace keeps only its most recent entries.
//! 6. **Core contract.** No unexpected [`CoreError`](bacnet_mstp_core::CoreError);
//!    adapter errors, delivery drops and frames from a station's own address
//!    (its echo leaking past the adapter) stay zero; a punctual adapter's core
//!    counts no deadline misses; no new deadline lies in the past, and none
//!    stays due after being serviced over and over at one instant.
//! 7. **Usage delay.** A punctual master answers a Token or Poll For Master
//!    addressed to it within Tusage_delay, unless it is still answering a
//!    request (Clause 9.5.6.9 finishes that first).
//!
//! # What it models and what it does not
//!
//! It models bit-time framing at any baud rate, collisions and the framing
//! errors they cause, the core's Clause 9 timers against real line
//! activity, adapter latency and timer jitter, and power loss.
//!
//! It does not model:
//!
//! - **Electrical behaviour.** There is no biasing, reflection or driver
//!   edge. A driver stops exactly at its final stop bit, so Tpostdrive is
//!   zero, and sends its octets with no idle gap, so Tframe_gap is never
//!   tested.
//! - **Propagation and drift.** Propagation is instantaneous. Every node
//!   runs at exactly the line's baud rate, with no ±1% tolerance. Seeded
//!   [`AdapterSpec::deadline_jitter`] stands in for clock and timer
//!   variation; without any, two masters that start in step stay in step,
//!   which a real bus would not allow for long.
//! - **Noise at bit level.** Noise on an idle line arrives as clean random
//!   octets; noise over a frame garbles whole characters. There are no
//!   overruns, parity errors or breaks.
//! - **Contention.** Every overlap reaches every receiver as framing errors.
//!   Two real drivers fighting over the line can also yield valid but wrong
//!   characters, and different ones at different receivers.
//! - **When a character is reported.** A received character is timestamped
//!   at the end of its stop bit; a UART usually signals it in the middle of
//!   the stop bit, about half a bit time sooner.
//! - **Host-style input.** Received octets go through `on_octet` one at a
//!   time. The chunked `on_validated_frame` and `on_host_activity` paths of
//!   a host adapter are not exercised.
//!
//! Each powered-on core borrows its receive buffer and action buffers for
//! its whole life, so the simulator leaks them on every power-up, about
//! 14 KB with the action buffers sized for extended frames; scenarios power
//! nodes up a bounded number of times.
//!
//! # Adding a scenario
//!
//! A scenario is an integration test in `tests/`:
//!
//! 1. Build a [`Sim`] from a [`SimConfig`], add masters with
//!    [`MasterSpec::new`] and any scripted stations.
//! 2. Schedule traffic with [`Sim::enqueue`] and faults with [`Sim::inject`].
//!    To act at a moment the run reaches, run up to it (see
//!    `run_until_frame` in `tests/common`), then inject at [`Sim::now`].
//! 3. Run with [`Sim::run_until`] and assert on the wire log
//!    ([`Sim::frames`]), on what applications received
//!    ([`Sim::deliveries`]), on cores' snapshots and counters
//!    ([`Sim::core`]) and on the [`Report`]. [`analysis`] reads token holds,
//!    passes and maintenance polls off the wire.
//! 4. Give the test a short doc comment naming the clauses it covers, in
//!    your own words.
//!
//! Runs are strict by default: the first violation panics with the trace.
//! Loosen only what the scenario is about, through [`Tolerance`].

#![forbid(unsafe_code)]

pub mod analysis;
mod event;
mod line;
mod report;
mod rng;
mod sim;
mod spec;
mod trace;
mod wire;

pub use line::Driver;
pub use report::{
    Collision, CollisionClass, Delivery, Episode, Report, Tolerance, Violation, ViolationKind,
    WireFrame,
};
pub use rng::SimRng;
pub use sim::{Sim, SimConfig, SimCore};
pub use spec::{
    AdapterSpec, Corruption, Fault, MasterSpec, ReplyPolicy, Script, ScriptSend, SeenFrame,
};
pub use trace::{Millis, Trace, TraceEntry, TraceKind, Who};
pub use wire::{WireTiming, BITS_PER_OCTET};
