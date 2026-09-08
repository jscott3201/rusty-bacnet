# Server request admission

Each running server has independent, global limits for **top-level inbound
handlers**: 64 confirmed and 32 unconfirmed by default. These finite defaults
are provisional owner policy, not benchmark results or normative BACnet limits.

Rust exposes `server::RequestAdmissionPolicy` through
`ServerConfig::request_admission_policy` and the generic, BIP, and SC builders'
`request_admission_policy(policy)` method. Both fields,
`max_confirmed_in_flight` and `max_unconfirmed_in_flight`, must be positive and
no greater than `tokio::sync::Semaphore::MAX_PERMITS`. Invalid values return an
error before server transport startup (and before SC TLS dialing). Validation
does not undo work already performed by a caller constructing its own transport.
Existing route, APDU, and SC reconnect validation precedence is retained.

Python appends keyword-only `max_confirmed_in_flight=64` and
`max_unconfirmed_in_flight=32` to `BACnetServer(...)`. Zero or a representable
value above the semaphore bound raises `ValueError`; negative or integer values
outside the native unsigned range raise `OverflowError`. Validation occurs in
the constructor, before any transport startup, including synchronous MS/TP
serial opening. There is no zero-as-disable or unlimited mode.

## Admission and overload

- Capacity is acquired synchronously before a handler is spawned. No queue of
  rejected work or capacity-waiting tasks is created. A slot lasts through the
  actual handler future, including awaited post-response work; completion,
  panic, and cancellation release it, even if its completed task is not reaped.
- Detectable exact pending/completed confirmed duplicates are discarded before
  capacity admission. They do not count as admitted or overloaded. The existing
  bounded duplicate retention, source canonicalization, and oversized untracked
  fallback remain. A denied new request is not marked completed and may retry.
- Confirmed overload schedules a server Abort with the original Invoke ID and
  `OUT_OF_RESOURCES`, retaining direct, routed, and MS/TP reply-channel paths.
  A separate private pool permits at most **eight owned Abort send workers**.
  It does not consume either handler class's slots and never waits for capacity.
  If that pool is also full, the confirmed request is silently dropped and the
  fallback counter increments. Admission does not mean the send succeeded.
- Unconfirmed overload is silently dropped and counted. DCC and discovery
  prechecks continue to precede handler admission; disabled traffic does not
  acquire a slot or provoke an inappropriate overload response.
- Inline SimpleAck, Error, Reject, Abort, and SegmentAck handling does not acquire
  these slots. Incoming reassembly retains its existing session/peer/byte bounds
  and acquires one handler slot only on completed dispatch. Segmented ComplexAck
  descendants remain owned and governed by their separate existing sender cap;
  they are not charged another confirmed slot.
- Explicit stop seals registration and cancels/joins owned handlers and overload
  workers using the existing request-task owner. Shutdown rejection is distinct
  from capacity exhaustion. This does not promise async Drop joining, arbitrary
  blocking-code preemption, or rollback of effects already performed.

The Abort reason follows the resource-exhaustion meaning in ASHRAE 135-2020
§18.10, with local server Abort behavior described in §5.4.5.3. The ultimate
silent-drop fallback is an owner-approved **known extreme-overload limitation**,
not a claim that §5.4.5.1 permits silently dropping valid confirmed requests or
that this policy establishes full protocol conformance.

## Counter contract

Rust `BACnetServer::request_admission_counters()` returns a
`RequestAdmissionCounters` value. Python
`await server.request_admission_counters()` returns a dictionary with the same
stable fields, described by the shipped `RequestAdmissionCounters` TypedDict:

| Fields | Meaning |
| --- | --- |
| `confirmed_active`, `unconfirmed_active` | Registered handler futures not yet finished/dropped |
| `confirmed_admitted_total`, `unconfirmed_admitted_total` | Cumulative handler registrations |
| `confirmed_overloaded_total`, `unconfirmed_overloaded_total` | Capacity-rejected requests; unconfirmed requests are dropped |
| `confirmed_shutdown_rejected_total`, `unconfirmed_shutdown_rejected_total` | Registrations denied by the sealed task owner |
| `abort_active` | Live overload Abort workers, at most eight |
| `abort_admitted_total` | Cumulative Abort registrations, **not send completions** |
| `confirmed_fallback_dropped_total` | Confirmed overloads dropped because all eight Abort workers were busy |
| `abort_shutdown_rejected_total` | Abort registrations denied by the sealed owner |

Fields are sampled independently from bounded atomic counters; the aggregate is
not a transactionally consistent snapshot. Totals cover one server lifetime,
without request history storage. Rust counters remain readable after successful
stop, with active counts zero. Python follows its existing `comm_state()` and
`local_address()` accessors: before start and after stop it raises
`RuntimeError("server not started")`. Immediately after a traffic-free start,
all counters are zero. No new restart semantics are promised.

## Remaining limits

There is no per-peer fairness or reserved critical-service capacity. A busy
confirmed class can reject DeviceCommunicationControl or ReinitializeDevice;
neither has a reserved slot. This bounds top-level handler concurrency, not
every allocation, incoming byte, service effect, notification, transport task,
or response budget. Work/response budgets remain deferred. Issue #521 remains
partial; #522 is unchanged. No throughput, fairness, hardware timing, or full
conformance claim follows from these limits.
