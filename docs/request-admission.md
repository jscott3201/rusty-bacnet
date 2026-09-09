# Server request admission

Each running server has independent, global limits for **top-level inbound
handlers**: 64 confirmed and 32 unconfirmed by default, plus independent
per-logical-peer limits of **16 ordinary confirmed and 8 unconfirmed**. Inside the confirmed
64, a strict **4-slot DCC ENABLE recovery reserve** leaves **60 ordinary slots**.
The independent protected per-peer cap is **1**: one peer can hold **16 ordinary
plus 1 recovery = 17 confirmed handlers**, regardless of arrival order.
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
The effective unconfirmed peer limit is `min(configured peer limit, global limit)`.
`confirmed_recovery_reserve` (default 4) must satisfy `0 <= R < G`, where
`G = max_confirmed_in_flight`. Zero disables protection: eligible ENABLE requests
use ordinary capacity as before. A custom global limit of 4 or less must now
explicitly configure reserve 0 or a smaller valid reserve. In particular, global
1 requires reserve 0; default peer limits remain valid. There is no
peer-less-than-or-equal-to-global validation requirement. With protection enabled,
the effective ordinary peer cap is `min(max_confirmed_in_flight_per_peer, G-R)`;
the effective protected peer cap is `min(max_recovery_in_flight_per_peer, R)`.
With `R=0`, both request classes use the ordinary peer cap
`min(max_confirmed_in_flight_per_peer, G)`; the recovery peer setting must still be positive.

**Semantic migration:** the former inclusive confirmed peer policy is replaced
by independent ordinary and recovery quotas. Default numbers, constructors, and
counter fields have not changed. Recovery is no longer clamped by the ordinary
peer setting: for example ordinary peer=1, recovery peer=3 and R=3 now allow the
same peer to hold 1 ordinary plus 3 recovery handlers if global room exists.
Deployments relying on the former combined peer ceiling must account for the sum
of the two effective caps; this is not an opt-in exemption or a fairness guarantee.

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
Total confirmed active handlers never exceed G. Each partition checks only its
own peer count: ordinary saturation at a peer does not deny eligible ENABLE when
that peer's recovery quota and the global recovery partition have room, and held
recovery handlers do not reduce its ordinary quota. Either exhausted recovery
limit can still deny ENABLE; availability is not guaranteed.

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
   The sealed-owner check precedes partition acquisition, then that partition's
   peer check. If partition and peer
  capacities are both exhausted, the rejection is classified global/partition.
  Peer rejection releases the temporary global permit without counting admission.
   Independent ordinary confirmed, protected, and
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
| `confirmed_peer_overloaded_total`, `unconfirmed_peer_overloaded_total` | Relevant partition's peer capacity rejections while partition/global capacity was available |
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

## Bounded acceptance and evidence

The owner accepts the delivered scope of [#521](https://github.com/jscott3201/rusty-bacnet/issues/521)
as **bounded and complete**, with the qualifications below. This supersedes the
earlier partial-scope and blanket work/response-budget deferral descriptions;
it records acceptance, not a claim that the issue has already been closed.
The literal requests and acceptance criteria are mapped separately so that
bounded acceptance is not mistaken for an unqualified availability guarantee.

Evidence is reused from merged PRs through [PR578](https://github.com/jscott3201/rusty-bacnet/pull/578),
baseline `e1f6ab665332af5042346e6b0302fbdbd9bd7661` (the same production tree as
reviewed head `e431027ad8c992298b15c7c967e239e31c5ca293`), not new runtime execution
for this documentation update. Source links below identify the delivered
implementation and tests; they do not imply live-network or performance testing.

| #521 literal request | Accepted delivery and evidence | Qualification |
| --- | --- | --- |
| Configurable global and per-peer request admission limits | [PR568](https://github.com/jscott3201/rusty-bacnet/pull/568), [PR569](https://github.com/jscott3201/rusty-bacnet/pull/569), and PR578; [admission source][admission-source], [global tests][admission-tests], [peer tests][peer-tests] | Top-level handler futures; logical identities, not authenticated principals. |
| Separate confirmed, unconfirmed, and operationally critical budgets | [PR570](https://github.com/jscott3201/rusty-bacnet/pull/570) and PR578; [recovery tests][recovery-tests] | DCC ENABLE is the sole designated critical service. PR578 replaces PR570's inclusive peer ceiling with independent ordinary/recovery quotas. |
| Deterministic overload: bounded queue, drop, or protocol-appropriate Reject/Abort | PR568/569/570; [admission tests][admission-tests] and [recovery tests][recovery-tests] | No waiting queue. Confirmed overload normally schedules server OUT_OF_RESOURCES via eight owned Abort workers; a full Abort pool causes a counted silent drop, an accepted extreme-overload limitation, **not protocol-conformant reply permission**. Unconfirmed overload is a counted drop. |
| Track child tasks under cancellation and join during shutdown | [PR563](https://github.com/jscott3201/rusty-bacnet/pull/563)–[PR567](https://github.com/jscott3201/rusty-bacnet/pull/567); [request owner][request-owner], [stop source][stop-source], lifecycle evidence below | Explicit stop covers owned requests, Abort workers, known descendants and producers, not every transport task or arbitrary blocking work. |
| Bounded overload/active-handler counters | PR568/569/570/578; [counter source][admission-source] and [admission][admission-tests]/[recovery tests][recovery-tests] | Existing confirmed counters include recovery; recovery counters are a subset. Independently sampled, not an atomic snapshot or send-success count. |
| Per-service work and response-byte budgets for large RPM, File, ReadRange and summaries | [Seven delivered service budgets](#delivered-service-budgets), PR571–577 | Payload/cardinality/retained-response policies with service-specific exclusions, not total-work or whole-memory limits. |

| #521 literal acceptance evidence | Evidence and bounded interpretation |
| --- | --- |
| Hold handlers behind a barrier; inject beyond the limit; active handlers never exceed it | [Global tests][admission-tests] (`admission_default_confirmed_limit_rejects_before_handler`, `admission_default_unconfirmed_limit_is_32_without_waiters`) and [recovery tests][recovery-tests] (`recovery_default_ordinary_partition_is_sixty_not_sixty_four`, `recovery_peer_quotas_are_independent_in_both_arrival_orders`). |
| One noisy peer cannot starve another peer or designated critical services | Accepted as quota isolation and ENABLE recovery, **not fair scheduling or universal availability**. At defaults a single logical peer's 16 ordinary slots leave 44 of 60 for others, and its one recovery slot leaves three of four, when not occupied by others. [Peer tests][peer-tests] exercise distinct origins; [recovery tests][recovery-tests] include `recovery_wire_same_peer_enable_with_sixteen_ordinary_held`, both arrival orders, release/refill and quota tables. Configured peer caps at or above global caps are valid, so this is not an all-configurations guarantee. |
| Saturation has deterministic, tested wire behavior | [Admission tests][admission-tests] cover independent handlers, eight Abort workers, routed replies and fallback; [recovery tests][recovery-tests] cover protected exhaustion, duplicate retry, password failures and fallback. A scheduled Abort is not a guaranteed successful send. |
| Shutdown cancels and joins every admitted child task | [Request-task tests][request-tests] (PR563), [segmented-worker tests][segmented-tests] ([PR564](https://github.com/jscott3201/rusty-bacnet/pull/564)), [DCC timer tests][timer-tests] ([PR565](https://github.com/jscott3201/rusty-bacnet/pull/565)), [notification-worker tests][notification-tests] ([PR566](https://github.com/jscott3201/rusty-bacnet/pull/566)), and [producer shutdown tests][producer-tests] (PR567) cover the explicit-stop owned-task boundary and retained joins after interrupted stop. No async Drop joining or opaque blocking-code preemption promise. |
| No live-network stress target required | Reused in-process Rust barrier, lifecycle and wire tests plus installed-native Python evidence at the stated baseline; Python sequential tests are not simultaneous-quota proof. No live-network stress target was used for this acceptance. |

## Delivered service budgets

These defaults apply to configured server dispatch. Each linked service document
contains configuration, compatibility migrations, failure/paging behavior and
precise exclusions; legacy unconfigured helpers do not acquire these limits.
Here **16 KiB means 16,384 bytes**; service-ACK caps exclude APDU/NPDU envelopes.
Earlier per-slice issue-status statements describe their delivery stage, not a
remaining requirement beyond this owner-accepted scope.

| Service and policy details | Default bounded quantity | Merged implementation and tests |
| --- | --- | --- |
| [ReadPropertyMultiple](rpm-budget.md) | 256 expanded results; 16 KiB encoded service ACK | [PR571](https://github.com/jscott3201/rusty-bacnet/pull/571), [handler tests](../crates/bacnet-server/src/handlers/tests/rpm_budget.rs) |
| [GetAlarmSummary](alarm-summary-budget.md) | 4096 total database objects; 16 KiB service ACK, complete response or refusal | [PR572](https://github.com/jscott3201/rusty-bacnet/pull/572), [handler tests](../crates/bacnet-server/src/handlers/tests/alarm_summary_budget.rs) |
| [GetEnrollmentSummary](enrollment-summary-budget.md) | 4096 total database objects; 16 KiB service ACK, complete response or refusal | [PR573](https://github.com/jscott3201/rusty-bacnet/pull/573), [handler tests](../crates/bacnet-server/src/handlers/tests/enrollment_summary_budget.rs) |
| [AtomicReadFile](atomic-read-file-budget.md) | 16,384 raw requested stream octets or 256 raw requested records; 16 KiB complete service ACK | [PR574](https://github.com/jscott3201/rusty-bacnet/pull/574), [handler tests](../crates/bacnet-server/src/handlers/tests/atomic_read_file_budget.rs) |
| [ReadRange](read-range-budget.md) | 256 returned items; 16 KiB service ACK; directional pages, not total read-work bounds | [PR575](https://github.com/jscott3201/rusty-bacnet/pull/575), [page tests](../crates/bacnet-server/src/handlers/tests/read_range_pages.rs) |
| [GetEventInformation](event-information-budget.md) | 4096 total database objects; 256 returned summaries; 16 KiB service ACK; full strict remaining-object validation even after a page fills | [PR576](https://github.com/jscott3201/rusty-bacnet/pull/576), [handler tests](../crates/bacnet-server/src/handlers/tests/event_information_budget_tests.rs) |
| [AtomicWriteFile](atomic-write-file-budget.md) | 16,384 stream payload octets; 256 records; 16,384 summed record payload bytes, after decode and before storage write; no ACK-size cap | [PR577](https://github.com/jscott3201/rusty-bacnet/pull/577), [handler tests](../crates/bacnet-server/src/handlers/tests/atomic_write_file_budget.rs) |

## Accepted limits and separate scope

Peer quotas do not provide fair queues, Sybil resistance, authenticated-principal
isolation or availability when the relevant pool is full. ENABLE classification
is not authorization: wrong passwords can occupy protected slots, and exhausted
protected capacity still denies recovery. There is no lending; `R=0` retains
ordinary fallback. Only DCC ENABLE is designated critical in this accepted scope;
ReinitializeDevice and other DCC modes have no reserve.

Admission and service budgets do not bound every allocation, incoming byte,
opaque callback, service effect, notification or transport task. The linked
per-service exclusions remain authoritative: there is no full-callback, RSS,
CPU/deadline, rollback, throughput, hardware-timing or full-conformance guarantee.
Explicit stop joins the owned task families above, not async Drop or all server
work regardless of transport and opaque blocking behavior.

[#522](https://github.com/jscott3201/rusty-bacnet/issues/522) remains open and separate:
authentication/default-deny policy and deprecated DCC DISABLE handling are not
implemented by this work. Deferred production EventLog integration, Audit/#125,
additional #181 EventEnrollment fault families, and GATE0007/SC restrictions are
unchanged. Accepted exclusions do not automatically create follow-up tasks or
reopen the bounded #521 scope.

[admission-source]: ../crates/bacnet-server/src/server/request_admission.rs
[admission-tests]: ../crates/bacnet-server/src/server/request_admission_tests.rs
[peer-tests]: ../crates/bacnet-server/src/server/peer_admission_tests.rs
[recovery-tests]: ../crates/bacnet-server/src/server/recovery_admission_tests.rs
[request-owner]: ../crates/bacnet-server/src/server/request_tasks.rs
[stop-source]: ../crates/bacnet-server/src/server/shutdown.rs
[request-tests]: ../crates/bacnet-server/src/server/request_tasks_tests.rs
[segmented-tests]: ../crates/bacnet-server/src/server/segmented_worker_tests.rs
[timer-tests]: ../crates/bacnet-server/src/server/dcc_timer_tests.rs
[notification-tests]: ../crates/bacnet-server/src/server/notification_worker_tests.rs
[producer-tests]: ../crates/bacnet-server/src/server/producer_shutdown_tests.rs
