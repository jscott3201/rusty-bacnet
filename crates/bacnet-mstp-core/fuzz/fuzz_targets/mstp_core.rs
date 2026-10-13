//! One MS/TP master core driven from arbitrary steps with injected time.
//!
//! # Input
//!
//! A station, configured from the input within the bounds validation
//! accepts: its address and Max_Master (small rings are likely),
//! Max_Info_Frames, baud rate, the configurable timers, the local scheduling
//! settings, extended frames on or off, and its storage: standard or
//! extended buffers, allocating or heapless queues of a few slots, one to
//! three delivery slots, and sometimes receive storage shorter than a
//! standard frame.
//!
//! Then up to 2,048 steps:
//!
//! - octets and receive errors on the line, with silences between them, and
//!   frames cut off part way, so the next step finds one half received;
//! - whole valid frames of every type from other stations, for this
//!   station, another or broadcast, standard and extended, octet by octet
//!   (sometimes with a gap inside) or from a host assembler
//!   (`on_validated_frame`, after `on_host_activity` or not); frames damaged
//!   past the header; `on_invalid_frame`; `on_host_activity`;
//! - the core's deadline, serviced on time or late, sometimes with a start
//!   reported before the frame it queued was polled;
//! - the adapter's start (accepted, deferred or expired), completion (with
//!   the frame's own echo, a host assembler's copy of it, or other octets
//!   heard meanwhile) and abort, and transmit events for ids it doesn't hold;
//! - NPDUs queued, oversize ones included, and the application's decisions
//!   on requests, late or for ids it never got;
//! - calls stamped before the core's last event.
//!
//! The adapter polls every action after each call, or, as the setup says,
//! polls what a line input queued only before its next other call, so
//! deliveries wait in their slots. A punctual-adapter step carries a token
//! use through quickly. With `MSTP_CORE_FUZZ_TRACE` set, a replay prints
//! each step, each frame handed over, the core's snapshot and the checker's
//! beliefs to stderr.
//!
//! # Oracle
//!
//! After every step, and on every action, independently of the core's state
//! (see `src/checker.rs` for the reasoning, with the clauses). The driver
//! reads the core's `snapshot()`, `counters()` and `next_deadline()` only to
//! confirm that a refused call or the station's echo changed nothing, to
//! schedule deadlines, and for traces; the checker shares only the codec
//! with the core, to build and decode frames.
//!
//! - **No panic**, and no debug assertion in the core.
//! - **Token custody.** A data frame, Token or Poll For Master goes out only
//!   while the station holds the token, as the frames on the line show it,
//!   or as a token retry, a search for a new successor or token generation
//!   at the moments Clause 9.5.6 allows them. Without the token only answers
//!   go out: a Reply To Poll For Master, a Test_Response with the request's
//!   data, Reply Postponed or the application's own reply data, each to a
//!   station that asked, once, and in time for the asker. A Token goes to
//!   NS, and only when there is one.
//! - **Owed answers go out.** One the station was free to give, once it
//!   exists, goes out on a quiet line before the asker may stop waiting,
//!   unless the application abandoned it.
//! - **Frames per token use** stay within Max_Info_Frames (and the core's
//!   local share of it), and a reply timeout ends the use.
//! - **Poll sweeps** stay within Max_Master: polls go to addresses up to it
//!   other than TS, one address after the next; a token use's first poll
//!   goes to PS + 1 or TS + 1, never to NS.
//! - **Turnaround.** Every frame starts T_turnaround after the last line
//!   activity, and a start refused as too soon was too soon.
//! - **Bounded storage.** Neither queue holds more than its slots; an NPDU
//!   longer than the station sends is refused and never queued; each data
//!   frame carries the NPDU the queue just handed out, typed by its length
//!   and its data-expecting-reply bit.
//! - **The adapter contract**, on every action: at most one frame pending
//!   or active; a switch to transmit directly before each frame; starts
//!   accepted only for the frame polled, once; completion only after a
//!   confirmed start; ids the adapter doesn't hold refused as stale; echo,
//!   between a confirmed start and its completion, changes nothing; at most
//!   as many deliveries between two calls as there are slots, each lying in
//!   the borrowed action buffers and fitting a slot.
//! - **Deliveries** are frames heard for this station or broadcast, in
//!   order; only a data request for this station carries a request id.
//! - **Monotonic time.** A call stamped before the last one does exactly
//!   what the same call stamped at the last one would: the core holds its
//!   clock there rather than run it back.
//! - **Progress.** A serviced deadline doesn't stay due.

#![no_main]

use libfuzzer_sys::fuzz_target;
use mstp_core_fuzz::Input;

fuzz_target!(|input: Input| mstp_core_fuzz::run(&input));
