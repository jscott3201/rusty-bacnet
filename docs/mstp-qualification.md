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
- Current transport scope remains standard frames, at most 501 NPDU octets and
  480 advertised APDU octets. A routed ReadProperty experiment is diagnostic
  evidence, **not a conformant MS/TP router claim**. Extended/COBS routing (RB-25),
  endpoint composition (RB-17+) and Linux RT/affinity controls (RB-24/#501) are not
  implemented or changed here. No timers, queue sizes or direction policies change.
- Neither dedicated-thread execution nor PREEMPT_RT promises on-wire timing.
  A USB result says nothing about native UART, another adapter with the same chip,
  or another driver/kernel/USB topology. No issue-closure claim before evidence.
- Keep raw captures, licensed standards and site inventories outside the repository.
  Publish only redacted counts, settings and synthetic role labels. An independent
  analyzer must observe the bus without becoming another active master or opening
  either application's serial device a second time.

## Host counters: obtain before moving the transport

`bacnet_transport::mstp::{MstpDiagnostics, MstpDiagnosticsSnapshot}` is Rust-only.
Keep the handle in each downstream router/mini process before its existing owner
consumes `MstpTransport`. No change to `TransportPort` or Python status is required.
Builders that construct the transport internally do not automatically expose it;
this slice does not add a generic endpoint status API.

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
or runtime. Counters saturate at `u64::MAX`; there is no reset. Snapshot fields use
individually atomic Relaxed loads, **not a globally coherent transaction**. Poll at
coarse intervals outside the MAC loop; do not add per-frame logging. For controlled
run deltas, sample before traffic and after quiescence/awaited stop. Drop/abort may
leave in-flight updates until cancellation completes. Reject cross-restart deltas
and mark saturated fields indeterminate rather than claiming exact counts.

| Fields | Host meaning and limits |
|---|---|
| `der_tx`, `der_rx` | Valid DataExpectingReply written / decoded. RX includes traffic addressed elsewhere, before MAC/application acceptance. |
| `dner_tx_direct` | DNER written through the one-use application reply path. |
| `dner_tx_queued` | DNER written from the token queue, including broadcasts. **Not necessarily a deferred reply.** |
| `dner_rx` | All valid DNER decoded, not a count of matching application responses. |
| `reply_postponed_tx`, `reply_postponed_rx` | Successfully written / decoded ReplyPostponed; RX need not match our outstanding request. |
| `wait_for_reply_timeouts` | Existing timer expiration handled in WaitForReply; not an APDU timeout or a wire-time measurement. |
| `invalid_frame_discards` | Host invalid-decode or resynchronization discard operations (including noise); not bad-wire-frame or byte counts. USB chunking can change event counts. |
| `stale_partial_resets` | Existing stale-host-assembly policy cleared a partial buffer; **not Tframe_abort**. |
| `outbound_queue_full`, `outbound_oversize` | Rejected queue admissions / oversize queued NPDU or direct reply. Not-started and invalid-MAC errors are excluded. |
| `ingress_full`, `ingress_closed` | Failed NPDU delivery because the bounded receive channel was full / closed. |
| `serial_read_errors`, `serial_write_errors` | Completed serial calls returning errors. Backend direction/drain errors count if surfaced by `write`. |

TX means successful encode and `SerialPort::write` completion, not UART drain,
actual wire transmission, peer receipt or ACK. Failed writes may emit partial
bytes; cancelled calls have no completed outcome to count. Direct `node_state`
manipulation bypasses transport admission/delivery accounting. There are no frame
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
| Transmitted request end to subsequent token activity when no reply begins | Observe Treply_timeout: 255 ms in this implementation; permitted enlargement only through 300 ms, not a tunable feature here |
| Token pass/PFM end to successor activity, and no-activity retry interval | Record both; peer response uses the 15 ms usage limit. Sender Tusage_timeout is 20 ms here, permitted enlargement only through 35 ms, not a tunable feature here |

For timeout measurements, absence of DataAvailable/ReceiveError events is part of
the condition; host snapshots cannot establish it. Wire idle and host scheduling
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
| DER on wire but no mini `der_rx` delta | Check addressing-independent decode/serial/assembly evidence and observation interval. |
| Mini `der_rx`, then `dner_tx_direct`, and matching immediate DNER on wire | Prompt path observed; follow router `dner_rx`, ingress drops and B/IP result. |
| ReplyPostponed on wire / mini `reply_postponed_tx`, then token and queued DNER | Deferred path only when the independent capture/application result correlates it. `dner_tx_queued` alone cannot. |
| Router `wait_for_reply_timeouts` increases | MAC wait expired; distinguish missing reply, late reply, host decode loss and application APDU timeout with capture. |
| `invalid_frame_discards` or `stale_partial_resets` increases | Host decode/resynchronization or stale assembly respectively, not automatically a wire CRC/timing violation. Compare homogeneous controls and independent capture. |
| `outbound_queue_full`/`outbound_oversize`, `ingress_full`/`ingress_closed`, or serial errors | Local rejection/delivery/I/O evidence; record exact deltas and unchanged returned errors, rather than interpreting no response as only a timer problem. |

If a specific path cannot meet measured limits, evaluate a different serial path
or dedicated controller and repeat the matrix. Do not tune fixed constants to hide
failures, infer native-UART behavior from USB, or promise a dedicated thread fixes
USB buffering. Leave #502/#707 open pending their remaining evidence/owner review.

## Result format and repository checks

The JSON template is an **unrun form**, not sample success data or a second
conformance ledger. Copy `cell_template` into `runs` per cell/repetition. Replace
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
cargo test -p bacnet-transport mstp --locked
cargo test -p bacnet-transport mstp --features serial --locked
cargo test -p bacnet-transport --doc --locked
```

The serial tests use simulated direction and native pseudo-terminal paths where
available, not physical RS-485. Bench results and cross-platform/native-direction
qualification remain separate evidence gates.
