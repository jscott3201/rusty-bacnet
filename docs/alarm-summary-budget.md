# GetAlarmSummary service budgets

Configured Rust and Python servers default to **4096 total database objects**
and **16384 logical encoded service-ACK bytes** per GetAlarmSummary request.
These positive, unbenchmarked thresholds are owner-approved local policy, not
Standard-mandated values or CPU/memory guarantees.

## Configuration and migration

Rust exposes `bacnet_server::server::GetAlarmSummaryBudget` with `max_objects`
and `max_service_ack_bytes`. Set `ServerConfig::get_alarm_summary_budget` or
call `get_alarm_summary_budget(...)` on generic, BACnet/IP or SC builders.
`validate()` rejects zero; all positive `usize` values are accepted without
preallocating their capacity. Validation precedes startup and SC dialing;
existing earlier validation priorities remain intact.

The new public `ServerConfig` field expands Rust source requirements:
exhaustive literals must add `get_alarm_summary_budget: Default::default()`
or an explicit budget. Literals with `..Default::default()` need no change.
Large deployments may need to raise one or both limits explicitly.

Python constructor settings are keyword-only on all transports:

```python
server = BACnetServer(
    123,
    alarm_summary_max_objects=4096,
    alarm_summary_max_service_ack_bytes=16384,
)
```

Zero raises `ValueError`; negative/native-integer-overflow values raise
`OverflowError`, and non-integer values raise `TypeError`. Constructor
validation happens before transport startup, including SC dial or serial open.
The legacy public low-level `handlers::handle_get_alarm_summary(db, buf)` stays
unconfigured and compatible; manual callers do not acquire server limits.

## Complete response or whole-service refusal

Under the same database read view used for projection, `db.len()` is checked
**before any object metadata, identifier or property callbacks**. Every object
counts, including Device and non-alarming objects. A database over the work cap
receives a server Abort **OUT_OF_RESOURCES**, without projection callbacks.

Within the work cap, existing database iteration order, selection, detection
enable filtering and strict projection errors remain unchanged. Entries are
encoded incrementally using a bounded logical scratch buffer and one temporary
entry, not accumulated as a complete vector. A triple that cannot fit triggers
a server Abort **BUFFER_OVERFLOW**, discards scratch and prevents later object
reads. No successful truncated ACK is sent; the internal helper leaves existing
caller output unchanged on failure. Earlier read side effects are not rolled back.
An empty selected set succeeds even with a one-byte budget (zero service bytes).

The byte budget counts encoded service parameters only, excluding APDU/NPDU.
It is independent of the peer's APDU size and segmentation capability. Within
budget, ordinary response sizing, segmentation and transport routing still apply.
Genuine projection failures retain their existing service Error mapping.

## Exclusions and evidence boundary

This policy does not bound one metadata callback/property read, its internal
allocations, allocator capacity, actual OOM, RSS, CPU time or arbitrary callback
preemption. Scratch capacity may exceed logical length. It does not add
authorization, fairness, pagination, rollback or other-service budgets. #521
remains partial; #522 and the #125/#181/GATE0007 SC hard stops are unchanged.
No README/PICS/BIBB or full-conformance support claim is added.

## Protocol rationale and local-policy evidence

Original paraphrase of ASHRAE 135-2020: §13.10 (printed 694–695 / PDF 696–697)
describes the deprecated, no-parameter service's complete qualifying active
alarm set, detection-disabled exclusions and empty result. Server execution is
discouraged. §21 (printed 861 / PDF 863) defines a flat sequence of identifier,
state and acknowledgment bits without continuation/truncation signaling.
§5.4.5.3 (printed 48–49 / PDF 50–51) permits local SendAbort, while ordinary
segmentation remains distinct. §18.10 (printed 800–801 / PDF 802–803) supports
the cannot-start resource refusal versus encoded-capacity overflow distinction.
Configured limits are not actual dynamic allocation failures (§18.11, printed
801 / PDF 803). ComplexACK remains §20.1.5 (printed 833–834 / PDF 835–836).

Focused evidence lives in handler `tests/alarm_summary_budget.rs`, server
`alarm_summary_budget.rs`, `alarm_summary_tests.rs`, SC pre-dial tests and
installed-native Python `test_alarm_summary_budget.py`. Independent triple/wire
vectors supplement codec comparisons; these tests establish the local policy,
not normative full conformance or hardware interoperability.
