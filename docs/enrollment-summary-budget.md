# GetEnrollmentSummary local response policy

The deprecated GetEnrollmentSummary interoperability service now defaults to
4096 **total database objects** and 16384 **encoded service-ACK logical bytes**.
These are owner-selected local policy values, not BACnet normative thresholds.

The request is decoded first, preserving existing malformed-request Error/Reject
behavior. Under the same database read view, a total object count above the cap
causes a whole-service server Abort `OUT_OF_RESOURCES`, before any capability,
identifier, metadata, property or Notification Class callback. Noncandidates and
Notification Class objects count too.

Matching entries are encoded incrementally into scratch storage using the service
codec. If the next entry would exceed the byte limit, the service returns server
Abort `BUFFER_OVERFLOW`, discards scratch and stops later callbacks. Caller output
is unchanged on failure; there is no partial successful list, cursor or continuation.
The cap excludes APDU/NPDU overhead and is independent of peer maximum APDU size
and segmentation. A complete within-budget ACK still uses normal segmentation.

## Configuration and migration

Rust `server::GetEnrollmentSummaryBudget` exposes positive `max_objects` and
`max_service_ack_bytes` fields. Set it through
`ServerConfig::get_enrollment_summary_budget` or the generic, B/IP and SC builders'
`get_enrollment_summary_budget(...)` method. Validation rejects zero before
transport startup or SC dialing. Positive values through `usize::MAX` are accepted.
Exhaustive external `ServerConfig` literals must add the new field, normally
`get_enrollment_summary_budget: Default::default()`, or use a default struct update.
The public unconfigured `handle_get_enrollment_summary` helper retains its legacy
unlimited behavior and error contract.

Python `BACnetServer` adds keyword-only `enrollment_summary_max_objects=4096` and
`enrollment_summary_max_service_ack_bytes=16384`. Zero raises `ValueError`;
negative or platform-`usize` overflow raises `OverflowError`. The installed stub
and native constructor expose the same defaults. Earlier positional arguments
remain unchanged.

## Scope and evidence

Within-budget projection, all conjunctive filters, disabled detection omission,
latest-transition priority selection, recipient membership, strict Notification
Class failures, iteration order and response routing retain their existing semantics.
The shared Notification Class resolver is unchanged.

This is **not** full work preemption or a conformance guarantee. Exclusions include
request decoding, individual callbacks/reads, recipient-list size, decoding and
scanning (which may repeat for class lookups), allocator capacity or actual OOM,
RSS, CPU, deadlines and rollback of reads. No other service policy changes.
Audit #125, EventLog #181, GATE0007 and existing SC security hard stops remain open.

Source rationale (original paraphrases of the licensed local base ASHRAE135-2020):

| Source | Relevant distinction |
| --- | --- |
| §13.11, printed696–698 / PDF698–700 | Historical/deprecated service; server execution is discouraged. Its filters combine conjunctively and positive execution returns the complete matching list, including an empty list. |
| §21, printed861 / PDF863 | ACK is a bare sequence of object ID, event type, state, priority and optional class entries; no continuation mechanism. |
| §18.10, printed800–801 / PDF802–803 | Cannot-start resource refusal is distinct from response buffer overflow. |
| §18.11, printed801 / PDF803 | Actual dynamic allocation failure is distinct from these configured limits. |
| §5.4.5.3, printed48–49 / PDF50–51 | Local SendAbort and normal responding transaction/segmentation behavior remain applicable. |

Focused Rust tests cover decoder precedence, zero callbacks at preflight, complete
wire vectors and variable widths, exact/over limits, stopping later callbacks,
legacy strict/filter parity, configuration and localhost segmented responses.
Installed native Python tests cover constructor/stub parity and direct/routed
complete ACK or Abort behavior. No external device/platform qualification implied.
