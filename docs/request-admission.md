# Server request admission

Each running server has independent, global limits for **top-level inbound
handlers**: 64 confirmed and 32 unconfirmed by default, plus independent
per-logical-peer limits of **16 confirmed and 8 unconfirmed**. Inside the confirmed
64, a strict **4-slot DCC ENABLE recovery reserve** leaves **60 ordinary slots**.
The additional protected per-peer cap is **1**, not an addition to the total 16.
These finite defaults
are provisional owner policy, not benchmark results or normative BACnet limits.

Rust exposes `server::RequestAdmissionPolicy` through
`ServerConfig::request_admission_policy` and the generic, BIP, and SC builders'
`request_admission_policy(policy)` method. The fields
`max_confirmed_in_flight`, `max_unconfirmed_in_flight`,
`max_confirmed_in_flight_per_peer`, `max_unconfirmed_in_flight_per_peer`, and
`max_recovery_in_flight_per_peer` (default 1), must be positive and
no greater than `tokio::sync::Semaphore::MAX_PERMITS`. Invalid values return an
error before server transport startup (and before SC TLS dialing). Validation
does not undo work already performed by a caller constructing its own transport.
Existing route, APDU, and SC reconnect validation precedence is retained.
The effective peer limit is `min(configured peer limit, global limit)` for each
class. `confirmed_recovery_reserve` (default 4) must satisfy `0 <= R < G`, where
`G = max_confirmed_in_flight`. Zero disables protection: eligible ENABLE requests
use ordinary capacity as before. A custom global limit of 4 or less must now
explicitly configure reserve 0 or a smaller valid reserve. In particular, global
1 requires reserve 0; default peer limits remain valid. There is no
peer-less-than-or-equal-to-global validation requirement. The effective protected
peer cap is `min(max_recovery_in_flight_per_peer, R, max_confirmed_in_flight_per_peer)`.

Adding these two Rust policy fields and three recovery counter fields is a **source-breaking
struct expansion** for exhaustive downstream literals/patterns. Policy literals
can use `..RequestAdmissionPolicy::default()` to inherit defaults, subject to the
tiny-global migration above. Total global defaults and the private Abort cap are unchanged.

Python appends keyword-only `max_confirmed_in_flight=64` and
`max_unconfirmed_in_flight=32`, followed by
`max_confirmed_in_flight_per_peer=16` and `max_unconfirmed_in_flight_per_peer=8`,
then `confirmed_recovery_reserve=4` and `max_recovery_in_flight_per_peer=1`,
to `BACnetServer(...)`, preserving old positional arguments. Invalid reserve/global
relationships, zero positive-only limits, or a representable
value above the semaphore bound raises `ValueError`; negative or integer values
outside the native unsigned range raise `OverflowError`. Validation occurs in
the constructor, before any transport startup, including synchronous MS/TP
serial opening. Only the reserve accepts zero, disabling protection rather than
disabling admission or granting unlimited capacity.

## Recovery eligibility and strict partitioning

Only requests accepted by the existing DCC decoder with mode ENABLE are eligible.
Other DCC modes, ReinitializeDevice, and malformed requests remain ordinary.
Classification is **not authorization**: existing handler password checks remain
authoritative. Decode-valid ENABLE with a wrong or missing required password can
briefly occupy a protected slot and return PASSWORD_FAILURE. Capacity exhaustion
still produces the existing Abort before handler execution or password validation.
No authentication or default-deny behavior changes.

Neither partition lends capacity: ordinary requests cannot use free protected
slots; protected requests cannot fall back to free ordinary slots when `R > 0`.
Total confirmed active handlers never exceed G. Both partitions share the existing
total confirmed peer cap, so a peer with 16 ordinary handlers can still be denied
ENABLE despite free protected capacity. Recovery availability is not promised for
a peer already at its own total cap.

The server-private classifier first traverses borrowed tag contents and bounds
optional password content before invoking the existing decoder. A decoded password
has at most 20 UTF-8 bytes; accepted UTF-8/Latin-1 payloads require at most 20 wire
bytes and UCS-2 at most 40. The preflight therefore introduces no attacker-sized
password allocation and excludes no accepted password encoding. Duration handling,
alternate charset validation, and the decoder's existing trailing-data tolerance
are retained. There is no arbitrary total-request cutoff, password comparison,
password logging, or mutating handler call in classification.

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
  The sealed-owner check precedes partition acquisition, then the inclusive total
  peer check, then the additional protected peer check. If partition and peer
  capacities are both exhausted, the rejection is classified global/partition.
  Peer rejection releases the temporary global permit without counting admission.
  Shared confirmed (ordinary plus protected), additional protected, and separate
  unconfirmed maps contain only active peer counts, bounded by their quotas.
  Last-guard drop removes all its peer registrations before releasing its partition
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
| `confirmed_global_overloaded_total`, `unconfirmed_global_overloaded_total` | Confirmed partition capacity or unconfirmed global capacity rejections, tested first |
| `confirmed_peer_overloaded_total`, `unconfirmed_peer_overloaded_total` | Total/protected peer capacity rejections while partition/global capacity was available |
| `recovery_active`, `recovery_admitted_total`, `recovery_overloaded_total` | Protected subset of the corresponding confirmed aggregates; all zero when reserve is zero |
| `confirmed_shutdown_rejected_total`, `unconfirmed_shutdown_rejected_total` | Registrations denied by the sealed task owner |
| `abort_active` | Live overload Abort workers, at most eight |
| `abort_admitted_total` | Cumulative Abort registrations, **not send completions** |
| `confirmed_fallback_dropped_total` | Confirmed overloads dropped because all eight Abort workers were busy |
| `abort_shutdown_rejected_total` | Abort registrations denied by the sealed owner |

Fields are sampled independently from bounded atomic counters; the aggregate is
not a transactionally consistent snapshot. Totals cover one server lifetime,
and all existing confirmed aggregates include ordinary and protected handlers.
At quiescence each class's overload total equals its global plus peer reason
totals (not necessarily while concurrently sampling them).
No request history is stored by these counters. Rust counters remain readable after successful
stop, with active counts zero. Python follows its existing `comm_state()` and
`local_address()` accessors: before start and after stop it raises
`RuntimeError("server not started")`. Immediately after a traffic-free start,
all counters are zero. No new restart semantics are promised.

## Remaining limits

Peer quotas partition identities, but do not provide scheduling fairness or
guaranteed availability once the relevant partition or peer cap is full. Only
DCC ENABLE has a reserve; all other critical-service reservations, including
ReinitializeDevice and other DCC modes, remain deferred. This bounds top-level handler concurrency, not
every allocation, incoming byte, service effect, notification, transport task,
or response budget. Work/response budgets remain deferred. Issue #521 remains
partial; #522 is unchanged. No throughput, fairness, hardware timing, or full
conformance claim follows from these limits.
