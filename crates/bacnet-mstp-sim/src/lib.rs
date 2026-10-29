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
//! octet boundaries and adapter jitter use whole microseconds; configured
//! gaps and driver holds retain nanosecond precision. One queue orders every event by
//! time, then by a fixed class (characters arriving, data ending, physical
//! driver releases, scheduled events, local echo availability, host
//! reads/completion, core deadlines, adapters starting frames, later octets), then by
//! station address, then by scheduling order. Nothing reads a wall clock or
//! runs on another thread, so a run replays exactly from its seed on any
//! host.
//!
//! **The line.** One half-duplex medium at a configured baud rate. An octet
//! occupies ten bit times (Clause 9.2.3). Each transmission captures its
//! [`AdapterSpec::inter_octet_gap`]: extra idle time after each non-final stop
//! bit. Octet boundaries use the original contiguous character grid, rounded
//! up to a whole microsecond, plus the number of preceding gaps. Rounding
//! never accumulates from character to character. There is no leading or
//! trailing gap. Physical ownership continues through gaps, but gaps create
//! no characters or line activity. Receivers frame characters the way a
//! UART does: the first octet to begin opens a character that ends with that
//! octet. If no other driver was on the line meanwhile, receivers get the
//! octet; by default an overlap produces a framing error, and while out-of-step octets
//! remain they get further framing errors, one per octet time. Propagation is
//! ideal: every receiver sees the same line at the same instant.
//! A configured [`AdapterSpec::post_drive`] holds idle mark after the final
//! data stop bit. It remains an overlapping driver but generates no characters
//! and does not reset line activity. [`Report::driver_releases`] records its
//! physical release separately from [`WireFrame::end`]; an overlap during the
//! hold carries [`PostDriveOverlap`] evidence without changing a completed
//! frame's receipt. Power loss releases the driver immediately.
//!
//! **Receiver-specific contention.** [`Sim::set_contention`] opts individual
//! receivers into [`ContentionPolicy`] while retaining the common physical
//! character grid. A receiver can capture a named contributor, XOR one
//! character with a clean UART indication, or use seeded choices. Different
//! receivers may see different data/errors for the same real overlap.
//! Every active contributor must span the complete common window; misaligned
//! windows, truncation, explicit framing errors and mid-character receiver
//! enable still produce errors. Explicit XOR faults retain their damaged byte.
//! Uncontended input, line activity, frame clocks, driver release and collision
//! records are unchanged. [`WireFrame`] remains transmission intent, and an
//! intact capture does not change its global `collided` or `is_clean` result.
//! This is synthetic digital fault injection, not electrical arbitration,
//! separate UART sampling/resynchronization, baud tolerance or clock drift.
//!
//! Seeded contention has one stream per station, separate from scenario,
//! timer-jitter, host-chunk and other receiver draws. It persists across
//! reboot; only eligible aligned contention draws from it. The existing
//! bounded trace records selected contributors and actual data/errors in
//! [`TraceKind::Contention`], and [`Sim::contention_stats`] exposes compact
//! counters. A transmitting host's local echo queue still belongs to its own
//! operation, but carries its actual resolved observation; the contention
//! trace identifies the selected contributor at the same physical time.
//!
//! Each opt-in master tracks one sequential contributor and up to 513
//! completed receipt candidates. A separate bounded codec instance follows
//! actual forwarded bytes and read-time gaps, respecting transmit, framing,
//! echo-filter and reboot dispositions. An intact physical stream creates a
//! candidate; only a matching reference decode at its terminal character
//! qualifies a core receipt check, with unsupported extended frames expecting
//! no receipt. Comparison uses that exact core input, so an unrelated later
//! frame cannot satisfy it. Identical simultaneous intents owe one perceived
//! receipt. Explicit filter suppression and power loss cancel candidates;
//! unexplained loss of a buffered terminal character remains diagnostic.
//! An older parser candidate can invalidate standalone-frame expectations,
//! recorded as unqualified. Enabling the audit after previously untracked
//! input waits for a known parser reset before qualifying receipts.
//! Mixed-origin and corrupted streams use bounded
//! reference decoding and exact scenario assertions, not a universal MAC
//! oracle. Receipt overflow is counted and reported; storage stays bounded.
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
//!   re-enables its receiver after any post-drive hold. Thus the core's timer
//!   origin is the final data stop, even when physical release comes later.
//!   A new start waits for its own previous driver's release. Until the start,
//!   its receiver stays on;
//!   if the core switches back to receive first, it has taken the frame
//!   back, and the adapter drops it.
//! - It hands each NPDU to the node's [`Application`]: a fixed
//!   [`ReplyPolicy`] or a callback. The application can answer it, in the
//!   reply window for a request that expects one, and can queue NPDUs of
//!   its own, such as a broadcast, through an [`Outbox`] for the node's
//!   next token.
//!
//! The receiver feeds `on_octet` for each clean character and
//! `on_receive_error` for each garbled one, timestamped when the character
//! ends. It is off while the node drives the line, unless
//! [`AdapterSpec::echo`] is set. Deadlines are serviced at
//! `next_deadline`, rounded up to the adapter's next timer tick when it has
//! a [`AdapterSpec::timer_tick`], plus any configured lateness and
//! per-deadline jitter.
//! Octet adapters service the transmit watchdog while a frame is going out;
//! expiry truncates it, releases its driver and enables reception. A final
//! stop exactly at the deadline completes first; a later octet starting at
//! the deadline is suppressed by the abort.
//!
//! **Buffered host adapters.** [`MasterSpec::host`] opts a node into
//! [`HostSpec`], independently of other nodes. The line still delivers each
//! peer character into its serial buffer. A read becomes ready 1–16 ms
//! after the first buffered character and coalesces characters that arrived
//! meanwhile. Each read takes at most the configured chunk size (optionally
//! seeded and randomized), and all its bytes reach `on_octet` with the read
//! timestamp, as in the production Tokio transport. Ready reads run before
//! core deadlines and transmit admission. Admission also drains up to eight
//! available reads without waiting, then checks the core's start contract.
//!
//! **Local echo return.** [`HostSpec::echo_return_delay`] defaults to zero
//! and is captured at transmit start. With [`AdapterSpec::echo`] enabled,
//! each eligible own character or framing error is retained with its physical
//! stop time and becomes locally available after that delay. Ordinary host
//! receive latency then applies. This adds no physical transmission, line
//! activity or peer input; octet-mode echo remains immediate. The queue owns
//! its observations across data completion, physical release, software
//! notification and later writes. A new write does not erase old unread
//! returns or wait for them; all bytes still face the content-based filter.
//!
//! Local returns use a FIFO, preserving physical observation order even if
//! a later transmission selects a shorter delay. This is a simulator policy,
//! not a Standard requirement. [`TraceKind::LocalEcho`] retains the original
//! transmission/incarnation, physical time, requested availability and actual
//! byte/error; its trace timestamp shows any FIFO deferral. Each host has at
//! most 4096 pending returns and one queue-head scheduler candidate. Due
//! returns precede host reads and new starts. Power-off scenario events win
//! a same-time availability tie, cancel the queue and fence a reboot.
//! [`Report::local_echo`] preserves compact capture, release, cancellation,
//! overflow and high-water counters across reboots. A full queue refuses
//! further characters and reports one invariant failure per overflow episode.
//!
//! Host completion is stamped with the later of serial drain return and
//! final data stop, independently of physical driver release. With
//! `echo_grace` configured, a content-matching filter
//! suppresses a complete copy of the outgoing frame and preserves differing
//! peer bytes, including a shared preamble. Its Grace, Residual and Probe
//! phases model the production policy; paired fixtures through the public
//! `MstpTransport` check payloads and counters. This simulator-owned filter is
//! not the production filter. Physical echo generation remains separately
//! controlled by [`AdapterSpec::echo`]. A missing echo can postpone completion
//! notification and expose an overdue core deadline; its completion stamp
//! still denotes the later of drain and final data stop. [`Sim::host_stats`]
//! reports this adapter work. As in the production host, core deadlines are
//! not polled during write/drain/echo. [`Report::host_transmits`] records
//! start, watchdog deadline, physical end, completion stamp and notification
//! independently. [`HostBudgetOverrun`] distinguishes late completion from
//! a timely completion whose notification was late; neither observation
//! claims a core timeout or physical abort occurred.
//! Local delay may change notification time without changing the physical
//! end or completion stamp. A ready read gets its entire chunk before Grace
//! expires, while Probe expires before bytes read exactly at its deadline.
//! Residual prefixes survive until strictly excessive read silence or a new
//! confirmed start. Arbitrarily late or interleaved copies can escape this
//! bounded heuristic; their own-address/parser/receipt diagnostics remain.
//!
//! Host input is capped at 4096 characters, pending clean-frame receipts at
//! 513, and filter storage at one transmitted frame, independently of the
//! bounded local-return queue. Overflow is a reported
//! invariant failure. Host state is discarded on power loss; delayed starts
//! and application decisions carry an incarnation fence across reboot.
//!
//! **Baud and clock error** ([`MasterSpec::skew`], #1772). Clause 9.2.3
//! lets a node's bit rate sit up to one percent off nominal; the simulator
//! holds each master's UART and protocol clock to that band (see [`Skew`]).
//! A master sends its frames on its own bit grid. Its core reads a clock
//! that matches the line's at each boot and then runs at its own rate:
//! every time the adapter hands the core is on that clock, and each
//! deadline is serviced once that clock reaches it. The turnaround check
//! measures on the same clock, since it checks a timer the core keeps; the
//! usage-delay and reply-delay checks, and every record, stay on the line's
//! clock, since they are what the node's peers rely on. So do application
//! delays and host-adapter timers.
//!
//! One percent is a stress band, far wider than a crystal's 100 ppm. At its
//! edges, adjacent masters' token-generation slots meet: at 38,400 baud
//! and above they collide once and form a ring. At 9,600 and 19,200 baud
//! exact timers keep them in lockstep; timers that tick at SilenceTimer's
//! nominal resolution ([`AdapterSpec::timer_tick`], Clause 9.5.2) let them
//! fall out of step within a few collisions, as real nodes do (#1804).
//!
//! **Scripted stations** decode the line with the codec's stream decoder
//! and react to frames through a [`Script`]: a minimal master that answers
//! polls and sends test frames, a slave, a misbehaving device. Each
//! [`ScriptSend`] can also set an inter-octet gap and a post-drive hold.
//!
//! **Gap limits and local budgets.** Gaps default to zero. The 20-bit idle
//! limit in 135-2020 9.2.3 includes the stop bit, leaving at most 19 bit times
//! after its end. [`WireTiming::max_inter_octet_gap`] rounds down to a
//! nanosecond, and larger configurations panic. Receive-abort timing uses
//! spacing between character indications, which also includes the next
//! character's duration; it is a separate threshold. Core watchdog defaults
//! budget contiguous frames. Scenarios with long gaps must explicitly set
//! a sufficient local watchdog using [`WireTiming::duration_with_gap`] and
//! their completion allowance. Long queued workloads also need sufficient
//! queue age. The simulator does not enlarge production defaults or protocol
//! timers to make a scenario pass.
//!
//! **Post-drive limits.** Holds default to zero. The 135-2020 text uses
//! different edges: 9.2.3 limits release to 15 bit times from the final stop
//! bit's beginning, while 9.5.3 measures from its end. This model explicitly
//! takes their conservative intersection: at most 14 bit times after the
//! end. [`WireTiming::max_post_drive`] rounds that upper bound down to a
//! nanosecond; larger configurations panic instead of being clamped. This is
//! a bounded interpretation of the conflicting reference edges, not hardware
//! conformance evidence. Release is ordered before new starts at the same
//! instant, and each release is fenced to its original transmission.
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
//!    counted it as received. Host accounting waits for bounded read delivery
//!    and counts multiple frames coalesced into one read.
//! 2. **Turnaround.** A master's core hands a frame over, and its adapter
//!    starts it, no sooner than its Tturnaround after the last character on
//!    the line, its own included, as the master's own clock measures it.
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
//! 6. **Core contract.** No unexpected refusal of a transmit start
//!    ([`TransmitError`](bacnet_mstp_core::TransmitError)) or a decision
//!    ([`RequestError`](bacnet_mstp_core::RequestError)); adapter and
//!    internal errors, delivery drops and frames from a station's own address
//!    (its echo leaking past the adapter) stay zero; a punctual adapter's core
//!    counts no late deadlines; no new deadline lies in the past, and none
//!    stays due after being serviced over and over at one instant.
//! 7. **Usage delay.** A punctual master answers a Token or Poll For Master
//!    addressed to it within Tusage_delay, unless it is still answering a
//!    request (Clause 9.5.6.9 finishes that first).
//!    Host direct replies also retain the physical 250 ms first-octet limit.
//!
//! Host defaults allow 100 ms frame abort, 35 ms usage timeout and 300 ms
//! reply timeout; those receive allowances do not extend the peer's 15 ms
//! usage or 250 ms reply obligations. A 16 ms receive delay can fail the wire
//! usage check. Even a shorter delay can hide a newly injected noise burst
//! at admission: the report retains the physical turnaround failure and any
//! resulting collision, with typed [`HostTimingFailure`] timestamps. Random
//! fault scenarios distinguish these negative cases from clean operation;
//! they never remove the violations or qualify a UART or USB adapter.
//!
//! # What it models and what it does not
//!
//! It models bit-time framing at any baud rate, each node's baud and clock
//! error within the tolerance, collisions and the framing errors they
//! cause, the core's Clause 9 timers against real line activity, adapter
//! latency and timer jitter, inter-octet gaps, post-drive ownership, local
//! watchdog expiry in octet mode, and power loss.
//!
//! It does not model:
//!
//! - **Electrical behaviour.** There is no biasing, reflection, analog driver
//!   edge or hardware watchdog qualification. Inter-octet gaps and post-drive
//!   hold model ownership and receiver eligibility, not voltage or current.
//! - **Propagation, and skew past the tolerance.** Propagation is
//!   instantaneous. A node's [`Skew`] stays within one percent; a node
//!   further off, and the bit errors its receivers would see, are not
//!   modeled. A skewed character is received whole, as a UART takes one
//!   whose rate is a couple of percent off. Clocks run at a constant rate:
//!   there is no wander. A phased [`AdapterSpec::timer_tick`] and seeded
//!   [`AdapterSpec::deadline_jitter`] stand in for timer variation.
//! - **Noise at bit level.** Noise on an idle line arrives as clean random
//!   octets; noise over a frame follows the selected contention policy. There are no
//!   overruns, parity errors or breaks.
//! - **Electrical contention and convergence.** Digital receiver choices
//!   do not predict voltage arbitration or guarantee token recovery.
//!   Duplicate-token collisions can persist under 135-2020 9.4.
//! - **When a character is reported.** A received character is timestamped
//!   at the end of its stop bit; a UART usually signals it in the middle of
//!   the stop bit, about half a bit time sooner.
//! - **Alternative frame assemblers.** Both octet mode and the current host
//!   transport's chunked reads feed `on_octet`. A platform adapter that uses
//!   `on_validated_frame` and `on_host_activity` is not exercised here.
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
mod app;
mod contention;
mod event;
mod line;
mod report;
mod rng;
mod sim;
mod skew;
mod spec;
mod trace;
mod wire;

pub use app::{Answer, Application, BROADCAST, Outbox, Received, ReplyPolicy};
pub use contention::{ContentionPolicy, ContentionStats};
pub use line::Driver;
pub use report::{
    Collision, CollisionClass, Delivery, DriverRelease, DriverReleaseCause, Episode,
    HostBudgetOverrun, HostStats, HostTimingFailure, HostTransmitTiming, LocalEchoStats,
    PostDriveOverlap, Report, Tolerance, Violation, ViolationKind, WireFrame,
};
pub use rng::SimRng;
pub use sim::{Sim, SimConfig, SimCore};
pub use skew::{MAX_SKEW_PPM, Skew};
pub use spec::{
    AdapterSpec, Corruption, Fault, HostSpec, MasterSpec, Script, ScriptSend, SeenFrame,
};
pub use trace::{Millis, Trace, TraceEntry, TraceKind, Who};
pub use wire::{BITS_PER_OCTET, WireTiming};
