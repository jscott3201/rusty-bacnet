# MS/TP host diagnostics and on-wire qualification method

This is an operator-run **isolated-bench method**, not a hardware result, a fix for
[#707](https://github.com/jscott3201/rusty-bacnet/issues/707), BTL testing/listing,
or completion of [#502](https://github.com/jscott3201/rusty-bacnet/issues/502).
It advances the RB-26 method only. No platform is qualified by this document.
Use the [result template](mstp-qualification-result.json) for each campaign.

## Scope and safety

- Use authorized bench peers and read-only sample properties. No production
  writes, FEC writes, reboot/DCC, configuration/table writes, or production load
  generation. **Do not use DNET 2000 in the reporter's lab.** Obtain an explicit
  unused, isolated test network assignment; this document supplies no default.
- A device's port uses standard frames by default, at most 501 NPDU octets and
  480 advertised APDU octets. `MstpConfig::extended_frames` (Python
  `mstp_extended_frames`) raises those to 1497 and 1476, and `BACnetRouter` turns
  it on for every MS/TP port it routes, so record the setting for each cell. A
  routed ReadProperty experiment is diagnostic evidence, **not a conformant MS/TP
  router claim**. On Linux the dedicated thread can run under `SCHED_FIFO` and be
  pinned to CPUs (`MstpThreadConfig::with_realtime_priority` and
  `with_cpu_affinity`, CLI `--mstp-rt-priority` and `--mstp-cpus`, #501), so
  record the requested settings and `execution_status()` for each cell. The
  host's timer defaults are under **Host timeouts and USB latency** below; this
  method changes no timers, queue sizes or direction policies.
- Neither dedicated-thread execution nor PREEMPT_RT promises on-wire timing.
  A USB result says nothing about native UART, another adapter with the same chip,
  or another driver/kernel/USB topology. No issue-closure claim before evidence.
- Keep raw captures, licensed standards and site inventories outside the repository.
  Publish only redacted counts, settings and synthetic role labels. An independent
  analyzer must observe the bus without becoming another active master or opening
  either application's serial device a second time.
  `bacnet_transport::mstp::monitor::MstpMonitor` over a
  `mstp_serial::ReceiveOnlyPort` on a second adapter is such an analyzer: it
  only reads, and its statistics carry the timestamp resolution to quote with
  any interval it reports.

## Host counters: obtain before moving the transport

`bacnet_transport::mstp::{MstpDiagnostics, MstpDiagnosticsSnapshot}` is Rust-only.
Keep the handle in each downstream router/mini process before its existing owner
consumes `MstpTransport`. No change to `TransportPort` or Python status is required.
A session that `MstpEndpointBuilder::build_session` builds keeps the handle for you:
`EndpointSession::mstp_diagnostics` returns it, before start, while running and
after stop. The client and server builders take a transport you built, so take
`MstpTransport::diagnostics` before handing it over.

```rust
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};

let (serial, _peer) = LoopbackSerial::pair(); // simulator, not hardware
let transport = MstpTransport::new(serial, MstpConfig::default());
let diagnostics = transport.diagnostics();
let observer = diagnostics.clone();
let before = observer.snapshot();
// Pass `transport` to the existing router/endpoint owner, which starts it once.
let owned_transport = transport;
let during = observer.snapshot();
drop(owned_transport);
let after = observer.snapshot(); // still readable; drop is not awaited stop
assert_eq!(before, during);      // no traffic in this example
assert_eq!(before, after);
```

Each transport starts at zero. Clones retain counters only, not the serial owner
or runtime. Every counter, `mac`'s totals of the core's counts included,
saturates at `u64::MAX`; there is no reset. Snapshot fields use individually atomic
Relaxed loads plus the core's counters as last published by the MAC task,
**not a globally coherent transaction**. Poll at
coarse intervals outside the MAC loop; do not add per-frame logging. For controlled
run deltas, sample before traffic and after quiescence/awaited stop. Drop/abort may
leave in-flight updates until cancellation completes. Reject cross-restart deltas
and mark saturated fields indeterminate rather than claiming exact counts.

| Fields | Host meaning and limits |
|---|---|
| `der_tx` | DataExpectingReply frames the core saw complete. |
| `dner_tx_direct` | DNER answers to a received request, handed back through the one-use application reply path in time. |
| `dner_tx_queued` | DNER sent from the token queue, including broadcasts and answers that missed their reply window. **Not necessarily a deferred reply.** |
| `reply_postponed_tx` | ReplyPostponed frames sent. |
| `wait_for_reply_timeouts` | Waits for a reply ended by T_reply_timeout; not an APDU timeout or a wire-time measurement. |
| `outbound_queue_full`, `outbound_oversize` | Rejected queue admissions / oversize queued NPDU or direct reply. Not-started and invalid-MAC errors are excluded. |
| `ingress_full`, `ingress_closed` | Failed NPDU delivery because the bounded receive channel was full / closed. |
| `serial_read_errors`, `serial_write_errors` | Completed serial calls returning errors; write and drain errors abort the frame. |
| `echo_mismatches`, `echo_missing`, `echo_fragments_dropped` | On a port declared with `Echo::Full`: frames during whose echo hunt other octets were passed on to the core (stray or pad octets, noise, a short or corrupted echo, an early peer answer); frames with nothing at all read within the grace; and echo fragments dropped incomplete, after waiting longer than T_frame_abort or at this station's next start. |
| `late_answers_queued`, `late_answers_dropped`, `late_answers_expired` | Application answers that came after Reply Postponed (or after the core gave up) and were queued for the token; reply channels given up because more than eight were waiting, or at the queue's maximum age. |
| `mac.frames_received`, `mac.npdus_delivered`, `mac.frames_delivered` | Valid frames the core's decoder assembled (any destination), and data it passed up. |
| `mac.parser_errors`, `mac.frame_aborts` | Frames invalid for this station per the receive rules (bad header, or bad data for this station or broadcast), and partial frames abandoned by T_frame_abort (100 ms on a host) or at a transmit start. Chunk gaps under 100 ms are not aborts. |
| `mac.late_deadlines`, `mac.deadline_lateness` | Deadlines the MAC task serviced 5 ms or more late (`HOST_LATE_DEADLINE`), and every serviced deadline by lateness: under 1 ms, 1-2, 2-5, 5-10, 10-25 and 25 ms or more, with the largest. Tokio's timer alone puts most wakes in the first two bands. |
| `mac.token_rotations`, `mac.token_retries`, `mac.token_regenerations`, `mac.deferred_starts`, `mac.late_replies`, `mac.expired_requests`, `mac.cancelled_sends`, `mac.discarded_while_busy`, `mac.own_address_frames` | The core's protocol counters, which `bacnet_mstp_core::MstpCounters` describes; the transport adds up each take of them (`bacnet_transport::mstp::MstpCounters`). |
| `mac.transmit_io_errors`, `mac.transmit_timeouts` | Frames aborted because a serial write or drain failed, and frames the core's transmit watchdog aborted. The receive-error counts (`mac.framing_errors` and the rest) stay zero on a host: its serial API reports no per-octet errors. So do `mac.transmit_aborts` and `mac.transmit_rejections`: the transport reports only `Io` aborts, and `Timeout` comes from the core's watchdog alone. |
| `mac.unicast`, `mac.broadcast` | The two transmit queues' depth when last published, highest depth, and drops. |

TX means the frame completed on the wire as far as the host can tell: the port's
drain returned and its time at the baud rate had passed. That is not peer receipt
or ACK. Failed writes may emit partial bytes; cancelled calls have no completed
outcome to count. The counters see only
traffic that goes through the transport API. There are no frame
bytes, invoke IDs, MACs, identities, paths or timestamps in these snapshots.
Host-read times must never be presented as wire timestamps. Counters alone cannot
correlate a queued DNER with an earlier DER or establish the cause of a lost reply.

## Matrix and setup

The reported three-master mixed FTDI/CH343 USB-RS485 trunk supplies **prior
consumer observations**, not results from this PR: passive and routed RP passed
at 38400/76800; 19200 passive was healthy but routed confirmed RP returned no
response; 9600 passive decoding was already degraded. Record exact revisions for
every rerun. The reporter's pin `acbf7baefe69d05f2368763dcc659d68e4bc114c`
already included the drain/turnaround and dedicated-thread work; do not presume
those changes are a new remedy.

1. Plan cells for **9600, 19200, 38400, 76800**, each with passive-only then
   read-only routed traffic, idle and representative load, and both `Tokio` and
   `DedicatedThread` where available. Record unsupported/unrun cells explicitly.
   Under 135-2020 Clause 9.2.3, 9600 and 38400 are required protocol rates; 19200
   and 76800 are optional. This requirement is not a measured hardware support claim.
2. Inventory every active participant and passive observer: role alias, application
   and library revision/build/features, board/CPU, OS/kernel, adapter make/model/
   revision/chipset, firmware, serial driver/version, baud, execution mode,
   Max_Master, Max_Info_Frames (MIF), direction mechanism, requested and effective
   RS-485/latency settings (or unavailable readback). Record USB root hub/port,
   hubs, other USB traffic, power, cable length, termination, bias, shielding,
   reference/ground and isolation, and bench topology. Keep private serial IDs out.
3. Independently qualify native UART with kernel/hardware direction, software-GPIO
   direction, and USB automatic-direction paths when available. Record DE polarity
   and delay settings/readback where supported. A driver's requested settings are
   not proof of effective settings. An unavailable path is `not_run`, not pass.
4. At 9600 especially, repeat with all-FTDI and all-CH343 active-adapter controls,
   holding topology, peer software, baud and load constant. Swap adapter roles and
   USB ports one variable at a time. Check the passive observer against an independent
   analyzer before concluding that a chipset or transmitter caused decode loss.

## Synchronized run procedure

Use a common run label and record clock alignment/uncertainty outside the counters.
Do not subtract timestamps from different clocks without a measured mapping.
Capture simultaneously:

- **B/IP oracle:** repeated read-only RP against a known B/IP sample object, with
  attempts, successes, BACnet Error/Reject/Abort outcomes, timeouts, retry settings,
  and response latency. It separates general application/host failure from the
  routed path; its success alone does not locate the MS/TP failure.
- **Router and mini:** before/after snapshots of every field above, optional
  coarse during-run snapshots, and process start/stop/restart boundaries. Keep
  peer activity fixed enough for count deltas to be interpretable.
- **Independent passive/analyzer evidence:** frame class/CRC/error counts and
  request/reply ordering on the actual trunk. For timing, use a calibrated logic
  analyzer with bus RX/TX and DE where accessible; record sample rate, resolution,
  decoding configuration, probe points, capture loss and clock uncertainty.
  A USB passive decoder with host timestamps cannot qualify sub-millisecond timing.

For each cell, first observe token/PFM/passive health without test requests. Then
run one outstanding routed confirmed RP at a time against a known read-only mini
property, with a B/IP oracle running concurrently. Record actual request rate,
APDU timeout/retries and all outcomes; count retransmissions separately. Preserve
the third master's usual bench activity. A recommended **method minimum**, not a
Standard requirement, is three repetitions of at least ten minutes each per
passive/active and idle/load cell, with at least 1000 RP attempts per active cell
(extend duration rather than increasing load beyond authorization).

Repeat with documented representative concurrent CPU, disk/database, logging,
network and USB load. Record generators/versions, intensity and actual utilization;
do not invent a local stress command or silently substitute an idle run. Stop on
unexpected traffic or unsafe electrical conditions. Save a final quiescent
snapshot, durations, measurement sample counts, maxima and violation counts.

### Timing measurements and decision rules

Source: licensed ASHRAE 135-2020 **9.2.3** (printed 99–100 / PDF 101–102) and
**9.5.3** (printed 104–106 / PDF 106–108). Use the adopted edition and applicable
corrections; no licensed text/capture is bundled. One bit time is `1 / baud` seconds.

| Quantity / analyzer edges | Criterion |
|---|---|
| Token receipt: last received stop-bit end to first transmitted start bit | Tusage_delay maximum 15 ms |
| PFM receipt: last received stop-bit end to first reply start bit | Tusage_delay maximum 15 ms |
| DER receipt: last received stop-bit end to first reply or ReplyPostponed start bit | Treply_delay maximum 250 ms; fixed, not baud-scaled |
| Last received stop-bit end to local DE assertion (also record first TX start) | Tturnaround minimum 40 bit times |
| Last transmitted stop bit to DE release | Record offsets from **both beginning and end** of that stop bit. 9.2.3 uses beginning, while 9.5.3 defines Tpostdrive from end (15 bit times). Conservatively require release no later than 15 bit times from beginning and not before the stop bit is generated; retain both measurements, not a silently chosen origin. |
| Within-frame idle interval: previous octet stop-bit beginning to next start-bit beginning (including the stop bit) | Tframe_gap maximum 20 bit times |
| Transmitted request end to subsequent token activity when no reply begins | Observe Treply_timeout: 255 to 300 ms allowed; this implementation's host default is 300 ms (`MstpConfig::reply_timeout`, Python `mstp_reply_timeout_ms`), so record the configured value |
| Token pass/PFM end to successor activity, and no-activity retry interval | Record both; peer response uses the 15 ms usage limit. Sender Tusage_timeout is 20 to 35 ms; the host default is 35 ms (`MstpConfig::usage_timeout`, Python `mstp_usage_timeout_ms`), so record the configured value |

For timeout measurements, absence of DataAvailable/ReceiveError events is part of
the condition; host snapshots cannot establish it.

**Host timeouts and USB latency (#1655).** A USB adapter hands the host what it
receives late: FTDI parts hold a short read for their latency timer, 16 ms by
default, and the host collects it at the next 1 ms USB frame. A successor that
answers at the 15 ms usage limit therefore reaches the host about 32 ms after
the token or poll ended, and an answer begun at the 250 ms reply limit about
267 ms after the request. The shortest timeouts (20 and 255 ms) run out first,
so every host port defaults to the longest ones 9.5.3 allows, 35 and 300 ms.
That leaves 3 ms and 33 ms for an adapter whose latency is longer still, as an
FTDI latency timer raised above 16 ms would be; lower the timer, rather than
expect the timeouts to cover it. On Windows, where the port has no drain, the
host counts a frame's end from its wire time plus the transmit-latency pad
(4 ms by default), so both timers start up to the pad, less the adapter's real
transmit latency, after the last stop bit: measured on the wire, the waits can
run to about 39 ms and 304 ms. Token generation is tighter still: each master's
generation slot (T_slot) is 10 ms wide, shorter than an FTDI part's default
16 ms latency timer, so a host that hears the line that late can misjudge
whose slot the silence has reached. For a station that may generate the token,
set the adapter's latency timer to its minimum (1 ms on FTDI parts). In the inventory, record the configured
`usage_timeout`, `reply_timeout` and `transmit_latency_pad` with the adapter's
latency setting. Signs that the latency exceeds the margin: `mac.token_retries`
rising while the successor is present, `wait_for_reply_timeouts` rising with
replies visible on the analyzer within 250 ms, or polls moving past a master
that answered. The transport tests show the defaults absorb a 17 ms delivery
delay where the minimums do not, and the simulator's invariants hold with the
longer timeouts; that is host evidence, not a hardware result. Wire idle and host scheduling
are different observations. Wire Tframe_abort starts at 60 bit times (allowed
enlargement up to 100 ms); **do not apply it to host USB chunk gaps** or change the
transport's stale-host policy. Unlike the fixed millisecond reply/usage limits,
Tturnaround scales with baud: 40 bit times is about 4.167/2.083/1.042/0.521 ms at
9600/19200/38400/76800 respectively.

Every measurement row records sample count, **minimum, maximum, violation count**,
uncertain count, units, interval edges, criterion and uncertainty. Percentiles are
optional additions, never substitutes for maxima. Missing DE or insufficient clock
precision makes the relevant row `indeterminate`, not pass. A sample whose error
interval straddles a bound is uncertain. A cell passes only its stated measured
scope with zero violations/uncertain samples and complete planned observations;
missing observations, dropped captures and unrun cells cannot support qualification.
RP success is a functional outcome separate from timing verdicts.

### Classify #707 without overclaiming

| Simultaneous evidence | Classification / next comparison |
|---|---|
| Oracle works; DER absent on analyzer and router `der_tx` unchanged | Investigate upstream routing/admission/token opportunity; counters do not prove which. |
| Router `der_tx` increases, but DER absent on independent wire capture | Compare driver/direction/capture completeness; write success is not wire proof. |
| DER on wire but no mini `mac.npdus_delivered` delta | Check `mac.frames_received`, `mac.parser_errors`, `mac.frame_aborts`, serial evidence and observation interval. |
| Mini `mac.npdus_delivered`, then `dner_tx_direct`, and matching immediate DNER on wire | Prompt path observed; follow router `mac.npdus_delivered`, ingress drops and B/IP result. |
| ReplyPostponed on wire / mini `reply_postponed_tx`, then token and queued DNER | Deferred path only when the independent capture/application result correlates it. `dner_tx_queued` alone cannot. |
| Router `wait_for_reply_timeouts` increases | MAC wait expired; distinguish missing reply, late reply, host decode loss and application APDU timeout with capture. |
| `mac.parser_errors` or `mac.frame_aborts` increases | Frames invalid for this station, or partial frames abandoned (by 100 ms of silence or at a transmit start); not automatically a wire CRC/timing violation. Compare homogeneous controls and independent capture. |
| `mac.late_deadlines` increases | The MAC task woke 5 ms or more late for a protocol deadline, beyond the timer's own rounding; compare the lateness bands and maximum, and look at host load and execution mode. |
| `outbound_queue_full`/`outbound_oversize`, `ingress_full`/`ingress_closed`, or serial errors | Local rejection/delivery/I/O evidence; record exact deltas and unchanged returned errors, rather than interpreting no response as only a timer problem. |

If a specific path cannot meet measured limits, evaluate a different serial path
or dedicated controller and repeat the matrix. Do not tune fixed constants to hide
failures, infer native-UART behavior from USB, or promise a dedicated thread fixes
USB buffering. Leave #502/#707 open pending their remaining evidence/owner review.

## Result format and repository checks

The JSON template is an **unrun form**, not sample success data or a
conformance record. Copy `cell_template` into `runs` per cell/repetition. Replace
nulls with measured values; use `not_run`, `indeterminate`, `pass`, or `fail` and
explain unavailable evidence. A measurement with zero samples cannot pass.
Each router/mini `before`, `after`, and `delta` map must contain exactly the listed
`diagnostic_fields`, as unsigned decimal **strings** to preserve all 64 bits in
JSON consumers. A saturated delta is null with its field in `saturated_fields`.
Do not serialize the snapshot with `Debug` and call it JSON. Measurement summaries
use the named row template; duplicate it for every timing quantity above.

Downstream router/mini/oracle launch commands and analyzer formats are external
to this repository. Record their exact versions and commands privately and export
the counts, settings, RP outcomes and measurement schema above. This repository
does not provide a hardware runner or a capture parser; none is invented here.
These actual repository tests provide **simulator/host** evidence only:

```sh
cargo nextest run -p bacnet-transport mstp --locked
cargo nextest run -p bacnet-transport mstp --features serial --locked
cargo test -p bacnet-transport --doc --locked
```

The serial tests use simulated direction and native pseudo-terminal paths where
available, not physical RS-485. Bench results and cross-platform/native-direction
qualification remain separate evidence gates.
