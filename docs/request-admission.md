# Server request admission

Each running server has independent, global limits for **top-level inbound
handlers**: 64 confirmed and 32 unconfirmed by default, plus independent
per-logical-peer limits of **16 confirmed and 8 unconfirmed**. These finite defaults
are provisional owner policy, not benchmark results or normative BACnet limits.

Rust exposes `server::RequestAdmissionPolicy` through
`ServerConfig::request_admission_policy` and the generic, BIP, and SC builders'
`request_admission_policy(policy)` method. All four fields,
`max_confirmed_in_flight`, `max_unconfirmed_in_flight`,
`max_confirmed_in_flight_per_peer`, and `max_unconfirmed_in_flight_per_peer`, must be positive and
no greater than `tokio::sync::Semaphore::MAX_PERMITS`. Invalid values return an
error before server transport startup (and before SC TLS dialing). Validation
does not undo work already performed by a caller constructing its own transport.
Existing route, APDU, and SC reconnect validation precedence is retained.
The effective peer limit is `min(configured peer limit, global limit)` for each
class. A global limit of 1 with the default peer limits is valid; there is no
peer-less-than-or-equal-to-global validation requirement.

Adding the two Rust policy fields and four counter fields is a **source-breaking
struct expansion** for exhaustive downstream literals/patterns. Policy literals
can use `..RequestAdmissionPolicy::default()` to inherit peer defaults, or set
explicit positive peer limits. Global defaults and the private Abort cap are unchanged.

Python appends keyword-only `max_confirmed_in_flight=64` and
`max_unconfirmed_in_flight=32`, followed by
`max_confirmed_in_flight_per_peer=16` and `max_unconfirmed_in_flight_per_peer=8`,
to `BACnetServer(...)`, preserving old positional arguments. Zero or a representable
value above the semaphore bound raises `ValueError`; negative or integer values
outside the native unsigned range raise `OverflowError`. Validation occurs in
the constructor, before any transport startup, including synchronous MS/TP
serial opening. There is no zero-as-disable or unlimited mode.

## Admission and overload

The shared private inbound duplicate/admission identity uses a routed source's
network and MAC when the network is 1 through 65534 and the MAC is nonempty.
Otherwise it uses the immediate source MAC. Thus one routed origin through two
routers shares a quota, while distinct origins have separate quotas. On SC this
is the supplied VMAC/logical source, **not an authenticated principal**. Spoofed
or multiplied identities can bypass a single identity's quota and exhaust the
global pool. COV, reassembly, and endpoint-core identity contracts are not changed.

- Capacity is acquired synchronously before a handler is spawned. No queue of
  rejected work or capacity-waiting tasks is created. A slot lasts through the
  actual handler future, including awaited post-response work; completion,
  panic, and cancellation release it, even if its completed task is not reaped.
  The sealed-owner check precedes global acquisition, which precedes peer
  registration. If both capacities are exhausted, the rejection is global.
  Peer rejection releases the temporary global permit without counting admission.
  Separate class maps contain only active peer counts, bounded by their global
  quotas. Last-guard drop removes the peer entry before releasing its global
  permit, including never-polled cancellation. There are no historical peer
  entries, timestamps, eviction rules, or exposed identity maps.
- Detectable exact pending/completed confirmed duplicates are discarded before
  capacity admission. They do not count as admitted or overloaded. The existing
  bounded duplicate retention, source canonicalization, and oversized untracked
  fallback remain. A denied new request is not marked completed and may retry.
- Confirmed overload schedules a server Abort with the original Invoke ID and
  `OUT_OF_RESOURCES`, retaining direct, routed, and MS/TP reply-channel paths.
  A separate private pool permits at most **eight owned Abort send workers**.
  It does not consume either handler class's global or peer slots and never waits
  for capacity. Global and peer rejection use this same eight-worker pool.
  If that pool is also full, the confirmed request is silently dropped and the
  fallback counter increments. Admission does not mean the send succeeded.
- Unconfirmed overload is silently dropped and counted. DCC and discovery
  prechecks continue to precede handler admission; disabled traffic does not
  acquire a slot or provoke an inappropriate overload response.
- Inline SimpleAck, Error, Reject, Abort, and SegmentAck handling does not acquire
  these slots. Incoming reassembly retains its existing session/peer/byte bounds
  and acquires one global and logical-peer handler slot only on completed dispatch. Segmented ComplexAck
  descendants remain owned and governed by their separate existing sender cap;
  they are not charged another confirmed global or peer slot.
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
| `confirmed_global_overloaded_total`, `unconfirmed_global_overloaded_total` | Global capacity rejections, tested first |
| `confirmed_peer_overloaded_total`, `unconfirmed_peer_overloaded_total` | Peer capacity rejections while global capacity was available |
| `confirmed_shutdown_rejected_total`, `unconfirmed_shutdown_rejected_total` | Registrations denied by the sealed task owner |
| `abort_active` | Live overload Abort workers, at most eight |
| `abort_admitted_total` | Cumulative Abort registrations, **not send completions** |
| `confirmed_fallback_dropped_total` | Confirmed overloads dropped because all eight Abort workers were busy |
| `abort_shutdown_rejected_total` | Abort registrations denied by the sealed owner |

Fields are sampled independently from bounded atomic counters; the aggregate is
not a transactionally consistent snapshot. Totals cover one server lifetime,
and at quiescence each class's overload total equals its global plus peer reason
totals (not necessarily while concurrently sampling them).
No request history is stored by these counters. Rust counters remain readable after successful
stop, with active counts zero. Python follows its existing `comm_state()` and
`local_address()` accessors: before start and after stop it raises
`RuntimeError("server not started")`. Immediately after a traffic-free start,
all counters are zero. No new restart semantics are promised.

## Remaining limits

Peer quotas partition identities, but do not provide scheduling fairness,
critical-service reservations, or guaranteed availability once the global pool
is full. A busy
confirmed class can reject DeviceCommunicationControl or ReinitializeDevice;
neither has a reserved slot. This bounds top-level handler concurrency, not
every allocation, incoming byte, service effect, notification, transport task,
or response budget. Work/response budgets remain deferred. Issue #521 remains
partial; #522 is unchanged. No throughput, fairness, hardware timing, or full
conformance claim follows from these limits.
