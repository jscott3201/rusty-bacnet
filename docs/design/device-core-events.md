# Design: COV and event reporting in the device core (#2303)

Status: **approved** by the maintainer on 2026-11-02 as the plan of record
(draft PR #2318); §11 records the decisions. Phase 3 of #2178, milestone
0.16.0. Line references are to dev at `c308eda25`; paths without a crate are
in `crates/bacnet-server/src/`. BACnet references are clause and table
numbers in ANSI/ASHRAE 135-2020, paraphrased.

## 1. Summary

Phases 1 and 2 ([device-core.md](device-core.md)) gave `bacnet-device-core`
the request side of a device: it answers RP, RPM, WP, WPM, Who-Is and
Who-Has from a borrowed `ObjectDatabase`, and `BACnetServer` runs those
services on it. Everything a device *initiates* still lives in
`bacnet-server`: the COV subscription table and its notifications, the
one-second intrinsic-reporting tick, Event Enrollment, Notification Class
distribution and alarm acknowledgement. A firmware image built on the core
can be read and written, but it can't tell anyone that a value changed or
that an alarm fired.

This design moves the decisions behind those features into the core as
sans-IO code on `bacnet-time`, in the idiom of the MS/TP, B/IP and router
cores: the host hands in requests, changed objects and the time; the core
hands back notifications to send and the next instant it wants to be
called. The core owns the subscription table, flight state included, as
plain data. The host owns the lock around it, the transport, address
resolution and confirmed delivery. The server runs on the moved code, as it
does for phases 1 and 2, and parity tests hold it to the same octets.

## 2. Goals and non-goals

**Goals (v1 of phase 3).** One implementation, run by firmware and by
`BACnetServer`, of: SubscribeCOV and SubscribeCOVProperty (Clauses 13.14,
13.15) with their lifetimes, caps and initial notification; COV evaluation
(Tables 13-1, 13-1a, 13-1a-2) after a write, a local change or a timed
commit, confirmed or not (Clauses 13.6, 13.7), and Active_COV_Subscriptions;
intrinsic reporting for every type the catalogue implements it for, its
delays measured from the instant a condition began, and the `ReliabilityEval`
fault pass; Notification Class distribution (Clauses 12.21, 13.2.5);
AcknowledgeAlarm and local acknowledgement (Clauses 13.2.3, 13.5) and
GetEventInformation, which a device with event-initiating objects has to
execute (Clause 13.2.4). Event Enrollment (Clauses 12.12, 13.3, 13.4) is
designed here (§6.6) and delivered under its own issue, #2319.

Also: no I/O, locks, async or clock in the core; the server's octets
unchanged but for the deliberate changes in §9; a `reporting` firmware
image, while the `objects` image keeps its rows. Like the rest of the core,
the reporting code needs an allocator (the `alloc` profile, not #1771's
allocation-free one).

**Non-goals (v1)**, all staying hosted (§7.1): the SubscribeCOVPropertyMultiple
service, its notifications and timestamped changes (§5.6; follow-up #2320),
though the table admits its contexts from PR 1; transport, routing,
segmentation, DCC, the server's confirmed-request transactions and its
Who-Is wait for an unbound recipient; receiving notifications; GetAlarmSummary
and GetEnrollmentSummary, which Clause 13.2.4 advises new devices against;
persisting subscriptions (Clause 13.1 lets a COV server lose them on
restart) or event state (§6.8); COV_Period, which the server lacks; policy,
authorization and audit, as in phases 1 and 2.

## 3. Placement and dependencies

### 3.1 In `bacnet-device-core`, behind features

```
Layer 5:   bacnet-objects       objects; CovReporting, EventReporting, EnrollmentEval, ReliabilityEval
Layer 5.5: bacnet-device-core   requests (phases 1-2) and reporting (this design):
             report             Outbox, Notification, Token, Delivery; the Reporter facade   (§4)
             cov                CovTable, subscribe, evaluate, flights, active lists          (§5)
             event              after_write, sweep, distribute, acknowledge, GetEventInfo     (§6)
             event::enrollment  the Event Enrollment pass                                     (§6.6)
Layer 6:   bacnet-server        locks, transport, routing, retries; tasks that call the above
```

The modules join the existing crate (§11), sharing `reply`,
`Inbound`, the `Responder`, `ReadOverlay` (§5.7) and `WriteCommitObserver`.
The new dependency is `bacnet-time` (the core's `std` turns on its `std`,
for `HostClock`); `bacnet-objects` takes it too, for the delay instants of
§6.1. `bacnet-services` already has the codecs.

### 3.2 Features

| Feature | Turns on | Gates |
|---|---|---|
| `cov` | `dep:bacnet-time` | `cov`, `report`; SubscribeCOV and SubscribeCOVProperty in `SERVICES` and the facade |
| `event-reporting` | `dep:bacnet-time`, `bacnet-objects/event-reporting` | `event`, `report`; AcknowledgeAlarm, GetEventInformation |
| `event-enrollment` | `event-reporting` | `event::enrollment`, which links `decode_event_parameter` (§6.6) |
| `cov-multiple` (#2320) | nothing more | the Multiple form (§5.6): `cov::subscribe_cov_property_multiple`, `cov::timed_capture`, `cov::timed`, `cov::multiple_report`, `cov::multiple_chunks`, and the private `cov::multiple_reads` and `cov::multiple_items`; SubscribeCOVPropertyMultiple in `SERVICES` and the facade, `NotifyService::CovMultiple` handed out |
| `std` (default) | the four above, beside `structured-writes` | hosted builds unchanged |

The linker drops what nothing calls, but the facade dispatches by service,
so an arm for SubscribeCOV links the COV path whether or not anyone
subscribes. #2215's rule applies: a feature stays only if its PR measures a
real saving (its spike found COV, audit and clock features worth under 400
octets each in `bacnet-objects`; these are worth kilobytes, §8). Every
feature needs `alloc`: the table, outbox and encoders use `Vec` and
`BytesMut`, and `ObjectDatabase` boxes objects and shares `Arc`s.

### 3.3 Time

Every instant the core stores or compares is a `bacnet_time::Instant`, and
every call that can expire or fire something takes `now`. Nothing divides a
64-bit value: a lifetime is `Duration::from_secs` (a multiply);
Time_Remaining, which the server rounds up (`cov/lifetime.rs:1-61`), is
`as_secs` of the remaining time plus 999,999,999 ns, `bacnet-time`'s long
division; Event Enrollment's pass count stays a `u32` ceiling division
(`event_enrollment/support.rs:58-61`).

The server maps its clock once, a `HostClock` reading Tokio's clock as
`runtime_clock::now` does (`runtime_clock.rs:34`), so paused-time tests move
the core's deadlines too. Wall-clock time (timestamps, recipient windows)
is the Device clock the database already reads (`ObjectDatabase::clock_frame`,
`bacnet-objects` `database.rs:526`); §6.2 and §6.3 cover a device without one.

### 3.4 CI scripts

`bacnet-device-core` is already in `check-no-std.sh`'s `cas_only` list
(atomic compare-and-swap, so no ARMv6-M); the script gains a second entry
for it with every feature on `alloc`. `check-firmware-link.sh` builds the
`reporting` image (§8.3) on the `objects` image's targets.
`check-default-features.sh` checks each feature alone with `alloc`, so a
doc link into a gated module fails there.

## 4. API

### 4.1 Two layers, as in phases 1 and 2

The server awaits between steps (to take a lock, to send, to wait for an
ACK); the core can't. So the core has **staged functions** the server calls
under its guards (`CovTable` methods, `cov::evaluate`, `event::sweep`,
`event::distribute`, ...), and a **`Reporter`** that strings them together
for a host with nothing to await: firmware and the parity tests. Every
staged function borrows what it touches and locks nothing.

### 4.2 Outbox, notifications and delivery

Evaluation runs under the host's database guard and encodes what it
decides to send, so nothing reads the database after the guard drops.

```rust
pub enum Destination {
    Subscriber(SubscriberEndpoint), // the route a subscription arrived on (cov/identity.rs:8-23, moved)
    Recipient(BACnetRecipient),     // a Notification Class destination
}
pub enum NotifyService { Cov, CovMultiple, Event }  // CovMultiple since #2320 PR 2b
pub struct Notification {
    pub token: Token,                         // its end comes back through `delivered`
    pub to: Destination,
    pub service: NotifyService,
    pub confirmed: bool,
    pub priority: NetworkPriority,            // Table 13-6 for events; normal for COV
}
pub enum Delivery { Sent, Acknowledged, Failed, NotSent }  // NotSent: DCC, no route; no attempt made
// `reply` gains `unconfirmed_request(service, params, out)` and
// `confirmed_request(invoke_id, max_apdu, service, params, out)`.
```

The outbox holds service parameters, not APDUs, since a confirmed request's
invoke ID is the host's. Unconfirmed sends report back too: the server
completes a COV observation only once its send succeeded
(`cov_notifications.rs:559-563`). Order within one input is the write
path's (`server/requests/mod.rs:665-719`): acknowledgement, events, COV,
then initial notifications.

The outbox is bounded by entry count and by octets, inside one buffer
allocated when it is built, so a storm can't grow the heap (§9). When it is
full, a COV notification isn't queued and its subscription is marked owed
(one later notification with the current values); an event notification
evicts the newest COV entry, whose subscription becomes owed the same way,
and is dropped and counted only when no COV entry is left to evict.

`next_deadline` is the earliest of: the next delay due (§6.1), the next
sweep, pass or fault pass, the earliest expiry, hold-off end and flight
deadline (§5.5), merged with `bacnet_time::earliest`.

### 4.3 Inputs

| Input | Staged function | Server caller today |
|---|---|---|
| A subscribe request | `CovTable::subscribe`, `subscribe_property` | `server/requests/mutations.rs:395-467` → `handlers/cov.rs:62-247` |
| Objects a commit changed | `cov::evaluate`, `event::after_write` | `requests/mod.rs:692-704`, `local_write_finish.rs:98-160`, `committed_cov.rs:77-108` |
| A due deadline | `CovTable::expire`, `event::sweep`, `event::fault_pass`, `event::enrollment::pass` | `server/lifecycle.rs:661-672`, `:688-742`; `intrinsic_reporting_lifecycle.rs:5-76` |
| How a notification ended | `CovTable::delivered` | `cov_notifications.rs:538-565`, `cov_notifications/confirmed.rs:47-288` |
| AcknowledgeAlarm, local acknowledgement, GetEventInformation | `event::acknowledge`, `acknowledge_local`, `get_event_information` | `server/requests/acknowledge_alarm.rs`, `requests/event_information.rs`; local: new |
| An object or a peer gone | `CovTable::remove_object`, `remove_peer` | `requests/mutations/object_lifecycle.rs:165-168`, `server/mod.rs:843-851` |

"Objects a commit changed" is what phase 2 already reports for network
writes (`Written.objects`, the `WriteCommitObserver` hooks). A firmware
application that changes a value itself (a sensor sample, a Reliability it
sets) reports the object the same way; the core doesn't poll. The server
works the same way: its local writes fan out explicitly (`local_writes.rs`).

Every `CovTable` mutation that ends or replaces flights or contexts (a
renewal, a route change, `remove_*`, `expire`) returns them as `Fenced {
tokens, contexts }`, so a host that keeps wake-ups or timed histories
outside the table (§4.5) can act on exactly those.

### 4.4 The `Reporter` facade (firmware, parity tests)

```rust
pub struct ReportConfig {
    pub cov: CovPolicy,                   // moved, cov/policy.rs:16-54; firmware lowers its caps
    pub outbox: OutboxLimits,             // entries and octets (§4.2)
    pub confirmed: Option<Confirmed>,     // APDU timeout and retries, for COV and events
    pub sweep: Option<Duration>,          // intrinsic sweep (§6.1); the server keeps 1 s
    pub enrollment_interval: Option<u32>, // seconds; None runs no Event Enrollment pass
    pub fault_detection: bool,            // the 10 s ReliabilityEval pass
}

pub trait Bindings { fn address_of(&self, device: ObjectIdentifier) -> Option<BACnetAddress>; }
pub trait InvokeIds { fn take(&mut self, to: &BACnetAddress) -> Option<u8>; fn release(&mut self, to: &BACnetAddress, id: u8); }

impl Reporter {
    pub fn new(identity: &DeviceIdentity, config: ReportConfig, now: Instant) -> Result<Self, Error>;
    /// COV services, AcknowledgeAlarm and GetEventInformation here, the rest
    /// through `responder.handle`; then evaluates what the request changed.
    pub fn handle(&mut self, responder: &Responder, db: &mut ObjectDatabase,
        peer: &SubscriberEndpoint, request: Inbound, now: Instant, out: &mut BytesMut) -> Written;
    /// A SimpleACK, Error, Reject or Abort that may answer one of its notifications.
    pub fn reply(&mut self, from: &BACnetAddress, apdu: &[u8], ids: &mut dyn InvokeIds, now: Instant) -> bool;
    pub fn changed(&mut self, db: &mut ObjectDatabase, objects: &[ObjectIdentifier], now: Instant);
    pub fn acknowledge_local(&mut self, db: &mut ObjectDatabase, object: ObjectIdentifier,
        state: EventState, now: Instant) -> Result<(), Error>;   // §6.4
    pub fn next_deadline(&self) -> Option<Instant>;
    pub fn on_deadline(&mut self, db: &mut ObjectDatabase, ids: &mut dyn InvokeIds, now: Instant);
    /// The next notification, addressed and, if confirmed, with its invoke ID.
    pub fn pop(&mut self, bindings: &dyn Bindings, ids: &mut dyn InvokeIds,
        out: &mut BytesMut) -> Option<Outgoing>;
    pub fn delivered(&mut self, token: Token, outcome: Delivery, now: Instant);
    pub fn overlay(&self, now: Instant) -> ActiveCovOverlay<'_>;   // §5.7
    pub fn counters(&self) -> ReporterCounters;
}
```

The facade carries the confirmed-request table firmware lacks, for COV and
event notifications alike: per report its invoke ID, peer, deadline and
retries left, on `bacnet-time`; `reply` matches answers by invoke ID and
peer, and `on_deadline` resends or gives up. Invoke IDs come from the host
(`InvokeIds`), so the device's own client requests share one ID space with
its notifications. A Device recipient is resolved through `Bindings`, which
firmware may back with a small Who-Is/I-Am cache; one that doesn't resolve
is counted (`ReporterCounters::unresolved`), never dropped silently.
`peer` is a parameter rather than a field of `Inbound` because COV identity
needs the route (the link MAC and, behind a router, the original network
address), which `CommandOrigin` doesn't carry.

### 4.5 The server on the staged functions; lock order and #1494

The server keeps `Arc<RwLock<ObjectDatabase>>` and
`Arc<RwLock<CovSubscriptionTable>>`; its table type becomes a wrapper:

```rust
pub struct CovSubscriptionTable {
    core: bacnet_device_core::cov::CovTable,  // all table state, flights and tickets included
    wakes: BTreeMap<Token, Arc<Notify>>,      // wake-ups for workers to fence; no state
    revisits: Arc<CovRevisits>,               // the revisit task's wake-up
    counters: Arc<AtomicCovCounters>,         // lock-free metrics, fed from the core's counts
    timed: HistoryStore,                      // the server's lock, wake-up and guards around
                                              // the core's TimedHistories (#2320, §5.6)
}
```

The wrapper keeps no copy of any flight's state: a confirmed worker
registers a wake-up for its token, and when a core mutation returns
`Fenced` the wrapper notifies those tokens and prunes the timed store of
the contexts named. A fenced worker's later `delivered` names a token the
core no longer knows, and is ignored.

Every path takes the database guard, then the table guard, calls the core
with both borrowed, and drops both before any send; the core takes no lock,
so it can't invert them. Two paths change. The Single fanout, which closes
the database guard before reading the table today (`cov_notifications.rs:314`,
`:388`, `:473`), holds database (read) then table (write) for one
synchronous `evaluate`, the order the subscribe services already use
(`mutations.rs:410-411`). Completing an observation
(`cov_notifications.rs:562`, `cov_notifications/confirmed.rs:103`, `:207`)
becomes `delivered`, under the table guard alone.

Tokio's `RwLock` is fair: a queued writer holds back later readers. So
holding the database read guard while waiting for the table write guard is
safe only if no path ever holds a table guard and then waits for the
database. PR 1's and PR 2's briefs carry that audit (every `cov_table`
acquisition, the revisit task and `delivered` included) beside #1494's.

The core adds no future, `select!`, `join` or `FuturesUnordered`. The
`select!`s that exist stay hosted and hold no lock: `cov_fanout.rs:116`
(guard dropped at 112), `cov_notifications/confirmed.rs:194` (delivery
against the fence) and `cov/timed.rs:1014` (store mutex released; since
#2320 PR 1 in `cov/timed_store.rs`, `HistoryStore::next_due`, §5.6). PR 1
rewrites the second over the wake-up map; no sibling future may sit
unpolled while an arm waits for the table lock.

## 5. COV

### 5.1 Services

| Service | Core | Server today |
|---|---|---|
| SubscribeCOV | `CovTable::subscribe` | `handlers/cov.rs:62-143` |
| SubscribeCOVProperty | `CovTable::subscribe_property` | `handlers/cov.rs:148-247`; `validate_cov_property`, `:34-56` |
| SubscribeCOVPropertyMultiple | `cov::subscribe_cov_property_multiple` (#2320 PR 2a), over `CovTable::subscribe_multiple` (PR 1) | `handlers/cov.rs:252-496`, `cov/admission.rs:89-201` |

Each takes the service parameters, as phase 2's `write_property` does,
decodes them with `bacnet-services` (a failure is a Reject), checks the
object and property against the object's `CovReporting` (`bacnet-objects`
`traits/capabilities/cov_reporting.rs:96-162`), applies the caps, and then
renews, cancels or adds. A renewal restarts the lifetime; a cancellation
that matches nothing still succeeds (Clause 13.14.2). A subscription or
renewal owes an initial notification, which the core queues behind the
reply (§4.2).

### 5.2 The table and its identity

The table moves whole in PR 1 (`cov.rs:51-422`, `cov/identity.rs`,
`cov/admission.rs`, `cov/policy.rs:1-304`), and every entry with it: each
snapshot holds a `FlightMarker` (a std mutex and a Tokio `Notify`,
`cov/identity.rs:179`, `cov/confirmed.rs:70-90`) and an
`Arc<ObservationOwner>` around an `AtomicU64` ticket (`identity.rs:171`,
`cov/observation_order.rs:7-9`), and neither builds in the core. So PR 1
also brings the flight state machine of §5.5 and a plain ticket counter in
the table; the snapshot's owner check becomes a table identifier. Beside
those:

- **Collections.** A lookup by object scans the `HashMap` today
  (`subscriptions_for`, `cov.rs:342`). The core keys entries with the
  monitored object first, in a `BTreeMap` or a sorted `Vec`; PR 1 measures
  both (code size on the image, the `cov` stress test) and keeps the
  smaller unless the server slows. The key needs `Ord`: `MacAddr` (a
  `SmallVec`) has it, `NpduAddress` derives it.
- **No shared state inside.** `Arc<AtomicCovCounters>`, the in-flight
  tracker (a std mutex and a Tokio semaphore, `cov/policy.rs:306-381`) and
  `CovRevisits` stay in the wrapper (§4.5). The core keeps plain counts,
  which the server adds to its atomics: 32-bit targets lack `AtomicU64`.
- **Time.** `expires_at` becomes an `Option<bacnet_time::Instant>`, and
  every lookup that purges takes `now` instead of reading `runtime_clock`.

Identity doesn't change: client address (`CovRecipient`, a direct MAC or
the routed SNET/SADR), process, object, and for a property subscription the
property and index (`cov/identity.rs:29-34`, `:85-110`), as Clause 13.14.2
ties a context to them; the route rides beside it in `SubscriberEndpoint`.
The caps are `CovPolicy`'s (1,024 in all, 64 per peer, 64 reserved, 16
indefinite and 16 in flight per peer; `cov/policy.rs:39-54`), which
firmware lowers.

### 5.3 Lifetimes and expiry

Lifetimes are seconds, and the core keeps the server's rules. SubscribeCOV
takes any lifetime, zero meaning indefinite (Clause 13.14.1.1.4 makes
indefinite optional and sets a 28,800 s floor; `handlers/cov.rs:120-126`).
SubscribeCOVProperty and the Multiple form don't allow zero (Clauses
13.15.1.1.4, 13.16.1.1.3), and the server refuses it (`:180-187`).
Indefinite subscriptions are a policy switch and a per-peer cap
(`allow_indefinite_subscriptions`, 16 per peer).

An expired entry goes three ways, as now: a lookup that meets it purges it;
`expire(now)` purges what is due (the server's 30 s task,
`lifecycle.rs:661-672`); a renewal replaces it. The facade's
`next_deadline` names the earliest expiry, so firmware wakes exactly then.
The server keeps its 30 s cadence, which no reply can tell apart, since
lookups purge first.

### 5.4 Change detection and increments

The decision code moves as is (`cov/sample.rs:1-110`,
`cov/observation.rs:1-76`, `cov/prepare.rs:1-144`, `cov/flags.rs`,
`cov/reported.rs`, `cov/value_source.rs`, `life_safety_cov.rs`), over the
objects crate's `cov_change` (`effective_increment`, `reports_change`).
Whole-object subscriptions test the leading value against COV_Increment,
Status_Flags, then the Table 13-1 row (`CovReportedProperty`: a trigger
sends, a value rides along); property subscriptions use their own
increment, else the object's for a numeric Present_Value, else any change
(Table 13-1a); commandable values add Table 13-1a-2's properties; Life
Safety objects compare whole snapshots. The comparison is with the last
*delivered* observation, and `evaluate` applies the per-input budget
(`EventBudget`, `cov_notifications.rs:13-76`: 64 notifications, 65,536
octets).

### 5.5 Confirmed flights

Today a confirmed report's state is spread over a `FlightMarker`, an RAII
`ConfirmedFlight` and a Tokio hold-off (`cov/confirmed.rs:70-216`), and
dropping the `ConfirmedFlight` settles it without the table lock, whether
on an Ack, a fence, DCC, a closed transaction, an abort or shutdown
(`:177-216`). `delivered` needs the table's write guard, which a `Drop`
can't await; so the core keeps the state, and each flight carries a
deadline at which it ends itself if nobody settled it. A flight belongs to
a coordinate: an ordinary or Single subscription, or a whole Multiple
context, whose references share one flight per route (`identity.rs:174-178`)
and which remembers the references a report carried.

```
          evaluate (a change)                     delivered(Acknowledged)
  Idle ──────────────────────► InFlight{token, deadline} ───────────────► Idle, then the owed revisit
   ▲                             │   │   │  a change meanwhile: marked owed
   │ delivered(NotSent), fenced  │   │   └── now >= deadline (abandoned) ──► Idle, revisit if owed
   ├─────────────────────────────┘   │ delivered(Failed)
   │                                 ▼
   └────── now >= until ────── HoldOff{until = now + timeout × (retries + 1)}
```

The deadline is the flight's start plus the retry budget, timeout ×
(retries + 1), plus one timeout of margin (15 s with the defaults), so a
live worker always reports first. An abandoned flight ends as a `Drop`
ends one today, with no hold-off and then the owed revisit; only an aborted
worker's coordinate is freed later than now, at the deadline. The hold-off
is the server's (`cov_notifications/confirmed.rs:158`), and an owed change
goes out once the flight ends (#896, `cov_fanout.rs:107-134`). The server
keeps the send, the invoke ID lease and the retries, and its worker calls
`delivered` once it returns.

### 5.6 The Multiple form

Clause 13.16's SubscribeCOVPropertyMultiple is the only Multiple form; its
notifications are the COVNotificationMultiple services. PR 1 moves its
admission (`cov/admission.rs:89-201`) and its per-context flights with the
table, and PR 2 its active list, so the table is one type in both hosts.
#2320 moves the rest of the path, in three PRs (Appendix A.5 has the
table): PR 1, then PR 2 in two parts, 2a and 2b.

**Moved by #2320 PR 1**, into `bacnet-device-core` behind the `cov-multiple`
feature, which `std` turns on and a firmware image leaves off:

- `cov::timed`: the timestamped histories (`TimedHistories`, `TimedChange`),
  their bound, drop reports, owed references and claims (`TimedClaim`).
- `cov::multiple_reads`: the read cache a report and a capture share
  (`MultipleReads`).

The histories were synchronous already; what was not is now the host's or
explicit:

| On dev | In the core | The server |
|---|---|---|
| `tokio::time::Instant`, read where needed | every call that stores or compares an instant takes `now`, a `bacnet_time::Instant` | passes `runtime_clock::core_now()` |
| the `TimedStore` mutex | none: plain state the host guards | `cov/timed_store.rs` keeps the mutex |
| the `select!` of `next_due` over a sleep and a `Notify` | `take_due(now)` gives the next deadline, `next_deadline()` asks for it, and `take_moved()` says one may have moved earlier | keeps the `select!`, which holds no lock; the store's guard fires the `Notify` as it drops |
| the server's atomics, `warn!` and `debug!` | plain counts (`take_counts`) and `Dropped` reports (`take_drops`), each saying whether it is its context's first for that cause | the guard adds the counts to the atomics and logs the reports |
| a claim requeued by `Drop` | a claim is data: its holder calls `commit` or `give_back(claim, now)`; or lodges it (`lodge(claim, deadline)`) and settles it by `ClaimId`, and an unsettled one is given back by `take_due` at its deadline | guards that settle on drop (below) |
| the send turn's RAII | `begin_send`, `end_send` | `SendTurn` keeps its `Drop` |
| std hash maps | B-tree maps, keyed by monitored object first | due references come out in key order |

Clause 13.16.1.1.4's Max_Notification_Delay backstop was a deadline
already (`take_due`); it is now one of two kinds `next_deadline` merges,
the other a lodged claim's. A lodged claim follows §5.5: a confirmed
worker's claim is lodged until its flight's own deadline, which the server
reads off the table (`CovTable::report_deadline`) so the two end at one
instant from one clock read. The worker settles it when it ends the flight
(half a timeout earlier), and a claim nobody settled goes back to its
references at the deadline. A holder that settles later finds it gone and
changes nothing.

The server's Drop guards stay, because its fanout returns early in many
places and a worker can be aborted: `ClaimGuard` (gives back unless
committed; named `TimedClaim` until PR 2a, like the core's claim it
holds), `LodgedClaim` (the worker's; returns unless committed) and
`SendTurn`. Each takes only the store's mutex, a leaf after the database
and the table, for a synchronous section, so none can invert the order or
hold a lock across an await. The store is `HistoryStore` since PR 2a
(`TimedStore` before).

A pending change still counts 144 bytes on a 64-bit host against its
context's memory ceiling, though it takes 136 since its instant is
`bacnet-time`'s (`CAPTURE_ALLOWANCE`): which change overflow drops is
visible to a subscriber, so it stays as it was.

**Two things do change on the wire**, both accepted (decision 15):

- A confirmed report whose worker is still alive past the report's
  deadline no longer has its timestamped changes retired by a late Ack.
  The table already ignored that Ack; now the changes went back to their
  references at the deadline too, and go out again with the next report.
  It takes an Ack followed by half a timeout's wait for the table.
- The items of a report the Max_Notification_Delay backstop sends come out
  in one fixed order, by monitored object. The backstop's references came
  out of a hash map before, in an order that varied from run to run.

Nothing else differs: timing, timestamps, chunking and what overflow drops
are as on dev. One log difference: past 64 repeat drops in one locked
section the server writes one debug line for the rest, where it wrote one
each; the counters hold every drop.

**Moved by #2320 PR 2a**, behind the same feature. Nothing changes on the
wire.

- `cov::subscribe_cov_property_multiple`: the service. Its checks (Clause
  13.16.1.1: a lifetime and a Max_Notification_Delay come together or not
  at all, the lifetime is not zero, the delay is at most an hour and less
  than the lifetime; then, per reference in request order, the object, the
  property, and a valid Device clock for a reference that asks for
  timestamps), the cancellation, the admission through
  `CovTable::subscribe_multiple`, the histories brought in line, and the
  initial capture of each accepted timestamped reference. It takes the
  table and the histories together and one `now`.
- `cov::timed::follow_table`: how the histories follow the table. The
  server's wrapper did this in its `sync`, between the admission and the
  capture; as a function it serves the wrapper after every mutation and
  the service inside its one call. The service reads what the table
  fenced through a crate-private accessor and leaves it for its host.
- `cov::timed_capture`: the capture at the mutation boundary, in three
  steps. `TimedCapture::object`, `exact`, `all` and `initial` take the
  table and `now` and collect the references; `read` takes the database
  and gives a `CaptureRead` of their values at one Device clock frame;
  `CaptureRead::queue` takes the histories and `now`, decides which values
  are changes and queues them. `TimedWriteCapture`, the write observer,
  hands each attempt's read to a `CaptureQueue`, the host's way to its
  histories.
- `cov::multiple_items` and `cov::multiple_chunks`: item assembly and the
  split to the subscriber's APDU size. `split` works on a plain
  `TimedClaim` and returns a `Split`: the parts, each with the claim its
  delivery retires, and what the histories have to be told, which the
  server's claim guard used to tell them from inside the split under the
  store's lock: `lost` (a change that lost a value too large for any
  notification), `forgone` (untimestamped references whose values fit no
  notification) and `rest` (changes no part conveys, to give back).

| On dev before PR 2a | In the core | The server |
|---|---|---|
| the capture held the store and read the clock under its lock | `read(db)`, then `queue(histories, now)` | `cov/store_capture.rs`: a `StoreCapture` carries the store; it locks only to queue, after the objects are read, and reads the clock there |
| capture entry points on the table wrapper | constructors over `&CovTable` and `now` | the wrapper keeps `timed_capture`, `timed_capture_exact` and `timed_capture_all`, which call them |
| the service read the clock in the handler, the wrapper and the capture | one `now` for the request | the wrapper's `subscribe_cov_property_multiple` reads it once, locks the store for the call and then does the host's part (counters, revisits, wake-ups) |
| the split locked the store to count and let a guard's drop give back | `Split { parts, lost, forgone, rest }` | the fanout tells the histories in the same order and sections, logs what was left out, and guards each part's claim again |
| std hash sets and one hash map in the items and the split | B-tree sets and map; only looked up, so no order follows from them | |

The clock (decision 16): a capture has one instant, the `now` its
queueing is given. dev before PR 1 read the clock once per change; the
branch since PR 1 once per capture. That instant only starts the
context's Max_Notification_Delay wait (Clause 13.16.1.1.4). What a
subscriber sees is the Device clock frame, which was one per capture
before PR 1 and is now, so timestamps are as on dev.

The store's mutex is held across the whole service call now, object reads
included, where each step took it by itself. It is still taken last, the
section is synchronous, and the core drops no claim guard, lodged claim or
send turn. A capture still reads its objects before it locks.

**Moved by #2320 PR 2b**, behind the same feature: the fanout's decisions,
as `cov::multiple_report`. A host fans a change out context by context
(`by_context`, in key order from the start `in_turn` picks), each in three steps, because
the server reads its objects under one guard and its table under another:

- `Reading::read(source, subscriptions, force)` reads the fanned-out
  references from the database or a producer's snapshot, with no table.
  Untimestamped references qualify here, against the observation their
  snapshot last reported; timestamped ones only read their current state.
- `Reading::gather(table, histories, now)` decides. A confirmed context
  with a report outstanding, or holding off after a failed one, drains
  nothing (`Busy`), and a hold-off moves the histories' backstop to its
  end. One whose hold-off has passed hands every reference to one
  follow-up (`FollowUp`). An unconfirmed context takes its send turn, or
  stands back when a report is going out (`StoodBack`). Otherwise the step
  issues the tickets, drains the pending changes of the whole context
  into one plain claim, adopts a change no producer captured at the Device
  clock, takes the owed marks and puts the owed references first
  (`Gathered`).
- `Gathered::split(histories, device, local_apdu)` splits the report and
  tells the histories what the split gave up, where the server's fanout
  did. It returns the `Report`: the parts with their claims, what each
  reference's delivery establishes, whether the report needs its turn,
  and the references left out, for the host's log.

`Report::into_first` is a confirmed report's first part with its ticket,
what its Ack completes and the deferred rest; `begin_confirmed` begins it
in the table and, in the same step, tells the histories (the part going
out, the deferred parts back and owed, the claim lodged at the report's
deadline); `hold_off` moves the backstop past a failed report's hold-off;
`carried` says what a delivered part completes.

| On dev before PR 2b | In the core | The server |
|---|---|---|
| contexts grouped in a std hash map, sent in its order | `by_context`: a B-tree map, so key order | iterates it; the follow-up pass too |
| Tokio guards around the decisions | three steps that borrow the objects, then the table and the histories | `db.read()`, then `cov_table.write()` for `gather`, released before `split` |
| the store's mutex, one short section per call | none | locked once across `gather` and once across `split`; still last, both synchronous |
| a clock read at each step | one `now` for `gather` | `runtime_clock::core_now()` once |
| the claim guard's and the send turn's `Drop` | a plain claim in `Gathered` and in each `Part`; `take_turn` hands the turn over | `SendTurn::held` right after `gather`; `ClaimGuard::of` on every part before the first send |
| `warn!` for a change with no valid clock, and for values left out | `unclocked`, `Report::left_out` | logs them |
| three guard calls after the table released a begun report | `begin_confirmed`, under the table | `begin_confirmed_report` on the table wrapper; the worker gets a `LodgedClaim` over the lodged id |
| the hold-off's end from the config and a clock read | `hold_off`, from the table's timing | one call in the worker's failure path |

The order between contexts, and of a follow-up's references, is the one
difference on the wire (decision 17). Contexts that share one input's
budget take their turns in key order from a start that moves per fanout,
so a spent budget, by count or by octets, keeps back different contexts
each time: an even spread, as the hash order gave, and no promise to any
one context. A list of the contexts kept back, served first next time,
was tried and dropped: it was per context where a kept-back report is per
object, it missed the octet budget, and the server could note a context
just cancelled. The rest is as on dev: which references a report takes, its
timestamps and chunking, the turn, what a failed send gives back and when
the backstop retries. Four things differ off the wire. One instant serves
a fanout's table step: the idle and hold-off checks, the lifetime each
reference has left (which is the Time_Remaining it reports) and the
capture instant of an adopted change, which anchors its
Max_Notification_Delay wait. On dev each of those read the clock for
itself, under the same table guard, so the readings were the same instant
to the clock's resolution and no check can come out differently. The
claims of a begun confirmed report are settled under the table guard, not
just after it, so no capture or fanout runs in between. A fanout that
takes a turn and finds nothing to report ends the turn under the table
guard, so no other fanout stands back behind it. And between `gather` and
the guards, the claim and the parts are plain data: nothing awaits or
returns there except on a spent budget, which guards the parts to give
them back, and a panic would lose the claim, as one inside the split
already could.

The sends stay hosted: the unconfirmed loop with its budget, DCC check
and counters, the confirmed admission (in-flight slot, invoke ID), the
worker with its retries, cutoff and outcome paths, and every guard.
`ReportContent`, `Stamp` and the split's other inputs are private to the
crate: `Gathered::split` is how a host reaches the split.

**The `Reporter` runs the service** (PR 2b, `report/reporter_multiple.rs`,
behind the feature). It holds its `TimedHistories` outright and does
explicitly, around its outbox, what the server does around its sends with
guards:

| The server | The `Reporter` |
|---|---|
| `store_capture`: producers capture under the database write guard | `TimedWriteCapture` around each write request's attempts; `TimedCapture::object` or `exact` in `changed` |
| the fanout's unconfirmed loop: send, commit, stop on a failure | all parts queued as `NotifyService::CovMultiple`, each under a `part_token`; `delivered(token, Sent)` commits the part's claim and records what it completes, any other outcome stops the report: the later parts leave the outbox and their claims go back, owing once a part went |
| `SendTurn`'s `Drop` | the turn is held while a part is queued; `end_send` when the report ends, and a fanout that stood back is looked at again |
| `send_confirmed_cov` and the worker's `LodgedClaim` | `begin_confirmed` when the first part is queued; the lodged claim settled in `end_confirmed` (delivered on the Ack, returned otherwise) and for a fenced token; `hold_off` after a failure |
| the follow-up task over `HistoryStore::next_due` and the revisit queue | `next_deadline` merges `TimedHistories::next_deadline`; `on_deadline` fans out `take_due`'s references and the looks owed, each context under a budget of its own; a context whose failed report's hold-off has passed is fanned out whole in the same call |
| the table wrapper's `sync` | `settle` runs `follow_table` after every mutation but the service's own, takes the histories' counts and drops, and ends an unconfirmed report whose reference left the table |

Where the outbox makes a choice the server has none for, the `Reporter`
keeps PR 2's (decision 13): a report that finds no room is looked at
again as room frees, and a part that finds none stops its report like one
that was not sent. Two differences from the server remain, neither in the
parity corpus: an object's Single and COV-multiple reports each take a
budget of their own, where the server shares one and alternates which
kind goes first; and the `Reporter` has no DeviceCommunicationControl.

### 5.7 Active_COV_Subscriptions

The Device's two lists come from the table (`cov/active.rs:1-332`,
`cov/active/multiple.rs:1-206`, moved): per row the recipient and process,
the monitored reference, the notification form, the time remaining at
`now` and the increment in use (`active.rs:319`). Phase 1 left the seam: a
read plan names the server-owned lists it reads (`LiveLists`,
`read/mod.rs:120`), and the server samples them between plan and read
(device-core.md §4.4). The server keeps that sampling, over the core's
projection. Firmware reads through `ActiveCovOverlay`, whose view answers
the two properties from the table and forwards the rest, so
`Responder::read` needs no change. Once PR 2's parity test reads through
the overlay, the corpus's one pinned Device row (`DIFFER`,
Active_COV_Subscriptions) matches, and PR 2 removes the pin.

### 5.8 Errors

| Condition | Answer | Clause | Today |
|---|---|---|---|
| Object missing | OBJECT / UNKNOWN_OBJECT | 13.14.1.3.1, 13.15.1.3.1 | `handlers/cov.rs:104-118` |
| No `CovReporting`, or whole-object subscriptions refused | OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED | 13.14.1.3.1 | same |
| Property absent | PROPERTY / UNKNOWN_PROPERTY | 13.15.1.3.1 | `:47-49` |
| Property not reportable | PROPERTY / NOT_COV_PROPERTY | 13.15.1.3.1 | `:50-55` |
| An index on a property that isn't an array | PROPERTY / PROPERTY_IS_NOT_AN_ARRAY | 13.15.1.1.5 | `cov/prepare.rs:60` |
| A cap reached | RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT | 13.14.1.3.1 | `cov/admission.rs:404-413`, `:425-452` |
| SubscribeCOVProperty with a zero lifetime | SERVICES / VALUE_OUT_OF_RANGE | 13.15.1.1.4, 13.15.1.3.1 | `:180-187` |
| SubscribeCOV indefinite while the policy refuses it | SERVICES / VALUE_OUT_OF_RANGE (#2317; before it, OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, which the standard's list lacks) | 13.14.1.1.4, 13.14.1.3.1 | `CovTable::check_admission`, in `bacnet-device-core`'s `cov/admission.rs` |
| A lifetime without the confirmed flag | Reject INCONSISTENT_PARAMETERS | 13.14.1.1.3, 18.9 | `:91-93`, `:189-191` |
| Parameters don't decode | Reject, the decoder's reason | 18.9, 20.1.8 | `:87`, `:176` |

A cancellation is checked before the object, so cancelling a subscription
to an object that is gone succeeds (Clause 13.14.2).

## 6. Events

### 6.1 Intrinsic reporting and Time_Delay

The objects hold the detectors and the commit kernel already
(`bacnet-objects` `event.rs`, `event/history.rs:69-110`) behind
`EventReporting`: OUT_OF_RANGE (Analog Input, Output, Value),
CHANGE_OF_STATE (Binary and Multi-state Inputs and Values, Access Zone,
Access Door), COMMAND_FAILURE (Binary and Multi-state Outputs) and
BUFFER_READY (the logs), each with fault precedence and
CHANGE_OF_RELIABILITY into and out of FAULT (Clause 13.2.5.3). What moves
is the driver: `event::after_write` calls `evaluate_intrinsic_reporting`
on each changed object (`server/event_notifications.rs:220-253`), and
`event::sweep` walks every object, refreshes Event_Algorithm_Inhibit from
its reference and calls `tick_intrinsic_reporting`
(`intrinsic_reporting_lifecycle.rs:26-65`), which also turns a Reliability
change into a FAULT transition (Clause 13.2.2).

**Delays become instants.** Today a detector seeds `remaining = delay`, and
each one-second tick takes one off and fires when one is left
(`bacnet-objects` `event.rs:507-520`, `:559-574`). So a condition a write
reveals between ticks fires up to a second early: written at 0.9 s with
Time_Delay 2, it fires at the 2 s tick, 1.1 s later, though Clause 13.3
wants the condition held for the whole delay. Instead the detector stores
its due instant (`PendingTransition { state, due }`, the condition's start
plus the delay), the capability methods take `now`, and a new
`EventReporting::next_due` feeds `next_deadline`; a transition fires at the
first evaluation at or after its due instant. That breaks `EventReporting`'s
signature (a Rust API change) and moves the server's timing (a **Wire:**
changelog entry: a write-seeded transition fires up to a second later,
when its delay ends). PR 3's test writes at 0.9 s with Time_Delay 2 and
expects the notification at 2.9 s, not 2.0 s. Event Enrollment keeps its
pass model (§6.6): it sees conditions only at passes, so it is never early.

**The sweep.** Delays need no tick now, but two standing duties do:
re-deriving FAULT from Reliability, and following an inhibit reference
whose target changed. Both happen at once when the change arrives through
`changed` or a write; the sweep catches the rest (a Reliability an object
set itself, a reference no host reported). The server sweeps once a second,
as now, and its task sleeps until the core's next deadline rather than the
next whole second, so the facade and the server fire at the same instant.
Firmware sets `ReportConfig::sweep` (§11).

Each proposal is committed before anyone asks whether to send it
(`commit_intrinsic_transition`, `event_notifications.rs:128-203`): the
Notification Class gives Ack_Required, the transition gets its timestamp,
and the object commits Event_State, Acked_Transitions, Event_Time_Stamps
and the message text at once (Clause 13.2.2.1.4). Event_Enable then only
decides whether the transition goes out (Clause 13.2.5), as the
`distribute` flag does today. A commit changes Status_Flags, so the
committed objects join the same call's COV input, after the event
notifications (#889).

### 6.2 Timestamps and message text

Timestamps move as they are (`server/event_timestamp.rs:1-75`): the Device
clock's frame when it holds a valid date and time, otherwise the
database's sequence number (`EventSequence`, reserved then confirmed),
which Clause 13.2 permits a device without a clock. The intrinsic message
text (`server/event_message_policy.rs:9`) uses `format!`; the core formats
nothing (#2187), so it is built by hand from literal fragments, which add a
little `.rodata` (the PR 3 row shows it).

### 6.3 Notification Class distribution

`event::distribute` turns a committed transition into notifications:

1. Priority and Ack_Required for the transition, from the class
   (`bacnet-objects` `notification_class/mod.rs:401-432`).
2. The destinations whose days, window and transitions match
   (`lookup_notification_recipients`, `:475-503`; window test `:308-321`).
   A list past the 32-destination cap, or one that won't decode, sends
   nothing, as now.
3. The parameters (Table 13-3), with the event values from
   `server/event_notification_payload.rs:1-753` (moved), one notification
   per destination with its process and confirmed flag, at the network
   priority Table 13-6 gives the event priority
   (`event_recipient_route.rs:11-18`).
4. Local distribution to Event Log objects (`db.log_event_notification`,
   `event_notifications.rs:431`), under the same guard.

The window needs a day and a local time, which a valid clock frame gives.
Without one the server falls back to system UTC
(`event_recipient_route.rs:20-24`), which needs a wall clock and 64-bit
division (`notification_class/mod.rs:331-345`). The core takes that
fallback as an `Option<(DaysOfWeek, Time)>` argument: the server passes its
system-UTC value, unchanged. With `None` the core fails closed: only a
destination valid every day, all day, matches, an unspecified hour counting
as all day as the window test already has it (`:309-311`). A clockless
build's Notification Class objects also refuse writes of any other
destination, which Clause 12.21.8 permits, so the configuration can't
promise a window the device can't keep. Routing stays the host's
(`event_awaited_recipients.rs`, `event_forwarding.rs:216-256`; `Bindings`).

Firmware with a `clock::DeviceClock` (#2321, decision 14) passes
`SyncedClock::window(now)`, through `Reporter::clock`: `None` until a
TimeSynchronization, UTCTimeSynchronization or the host's own real-time
clock has set it. `clock::destination_open` judges each destination by
the server's test (`notification_class::in_window`) or, with `None`, by
`always_in_window`. Never work the window out from `db.clock_frame()`:
an unsynchronized `DeviceClock` gives a valid 1990 frame there, and the
windows would open on it. Such a build stores restricted destinations;
only a `Responder` with no clock refuses them (decision 14).

### 6.4 Acknowledgement

`event::acknowledge` is `handle_acknowledge_alarm`
(`handlers/alarm_event/acknowledge_alarm.rs:37-95`), moved. The object
matches the acknowledged state and timestamp against its committed history
and sets that transition's Acked_Transitions bit (Clause 13.2.3), or
answers INVALID_EVENT_STATE, INVALID_TIME_STAMP or NO_ALARM_CONFIGURED.
When it knows the transition's states and the object's algorithm, the core
queues the acknowledgement notification (ACK_NOTIFICATION, Table 13-3's
acknowledgement column), stamped when the acknowledgement runs. It goes out
after the SimpleACK, as now (`requests/mod.rs:665-671`).

Clause 13.2.3 also allows acknowledgement by local means, a reset button
say, and nothing in the stack offers that today. `acknowledge_local` names
an object and a transition's state, takes the latest committed timestamp
of that transition, and runs the same path (PR 3).

### 6.5 GetEventInformation

Clause 13.2.4 makes GetEventInformation part of event reporting, so the
`event-reporting` feature carries it: `get_event_information.rs` (412
lines) and `event_information_page.rs:1-138` move, their `tracing` and
`format!` lines turned into returned outcomes. The response budget
(`event_information_budget.rs`) stays a host input.

### 6.6 Event Enrollment

The evaluator is portable already: no Tokio, synchronous over
`&mut ObjectDatabase` (`event_enrollment/mod.rs:128-861`, `algorithms.rs`,
`commit.rs:339-615`, `fault.rs`, `reference.rs`, `support.rs`). Its event
algorithms are OUT_OF_RANGE, FLOATING_LIMIT, CHANGE_OF_STATE,
CHANGE_OF_BITSTRING, CHANGE_OF_VALUE and NONE; its fault algorithms NONE,
STATUS_FLAGS and FAULT_OUT_OF_RANGE. The move swaps `HashMap` and `HashSet`
for the B-tree forms, returns the pass report the server logs
(`commit.rs:152-177`), and takes the interval and `now` as arguments. The
pass runs every `enrollment_interval` seconds (10 by default,
`config.rs:274`).

It is the costliest feature for an image: it reads Event_Parameters with
`decode_event_parameter` (`fault.rs:39-54`, 11,424 octets of thumbv7m
`.text`), the decoder #2210 removed from the `objects` image.

### 6.7 The fault pass

`FaultDetector::evaluate` (`fault_detection.rs:93-135`) visits each
object's `ReliabilityEval` every 10 s while `enable_fault_detection` is on
(off by default, `config.rs:272`). It moves with its warning dedup turned
into a returned list and its unused `comm_timeout` dropped. The objects
whose Reliability changed feed COV and `after_write` in the same call.

### 6.8 Event-state persistence

Event_State, Acked_Transitions, Event_Time_Stamps, Event_Message_Texts and
Event Enrollment's pending state live in memory today, and after a restart
the objects start from their configured state. The core keeps that. The
Notification Class saves its own Recipient_List, which Clause 12.21.8 says
outlives a restart, through its persistence trait, unchanged. Saving event
state would be a feature with its own design (§11: none in v1).

## 7. What moves, what stays, and the slices

Moved means moved: the server imports the core's item, no alias is left,
and every caller and test changes in the same PR (AGENTS.md, pre-1.0).
Each PR is pure moves first, then the logic changes (time, collections,
the flight state, instants) in commits of their own. A move commit changes
the server's existing tests in their imports only: those tests passing
unchanged is the proof that the move kept the behaviour. Appendix A has
the tables, with line counts (non-test lines).

| PR | Scope | Lines | Parity corpus (§7.2) | Budget row |
|---|---|---|---|---|
| 1 | The COV table, flights and tickets, subscribe services | ~2,060 | subscribe, renew, cancel, errors, caps | `reporting` image starts: subscribe through staged calls |
| 2 | Active lists; COV evaluation and notifications; the `Reporter`, outbox and confirmed table; `ActiveCovOverlay` | ~1,585 moved, ~800 new | Active_COV, notifications, increments, flights | the image runs through the `Reporter` |
| 3 | Intrinsic events with instant delays, distribution, both acknowledgements, GetEventInformation, the fault pass | ~1,990 moved, ~200 new | delays, recipients, acknowledgements | `event-reporting` on |
| #2319 | Event Enrollment, on §6.6 and Appendix A.4 | ~2,720 | each algorithm | `event-enrollment` on |

#2303 closes on PRs 1-3. #2319 can split its pure algorithm files first,
the pass and commit second (it passes 2,500 lines). The active lists sit
in PR 2 to keep PR 1 under that line.

### 7.1 What stays hosted

- Tasks: purge, intrinsic, fault and Event Enrollment (`lifecycle.rs:661-742`,
  `:789-795`), now staged calls on Tokio timers; `committed_cov.rs`,
  `cov_fanout.rs`, `cov_notify_context.rs`.
- Sending: the send half of `cov_notifications.rs`,
  `cov_notifications/confirmed.rs`, `event_send.rs`,
  `notification_transactions.rs`, `notification_attempts.rs`,
  `event_delivery.rs`, `event_suppression.rs`.
- Addresses and receiving: `event_recipient_route.rs`,
  `event_awaited_recipients.rs`, the bindings, `event_forwarding*.rs`,
  `received_event_log.rs`, `server/clock.rs`; GetAlarmSummary and
  GetEnrollmentSummary. Of the Multiple fanout
  (`cov_notifications/multiple.rs`) the sends stay: its decisions are the
  core's since #2320 PR 2b (§5.6). `cov/timed_store.rs` stays for good:
  the lock, wake-up, counters, log lines and Drop guards around the core's
  histories; so does `cov/store_capture.rs`, the store around a capture
  (§5.6).

### 7.2 Parity tests

The parity tests prove orchestration, that the facade strings the staged
functions together as the server does; they can't prove a move faithful,
since both sides run the moved code. The server's unchanged tests (§7) and
the Python suite, which asserts the PDUs a real server sends, are the
drift oracle, and every PR runs both.

They are in-crate `bacnet-server` tests, in new submodules
`device_core_parity_tests/cov.rs` and `events.rs` (the parent file is at
the 700-line cap), on the shared `TestTransport`, paused Tokio time, no
sleeps. One fixture database is built twice: a `BACnetServer` serves one,
a `Reporter` with the same identity and config the other, both with one
fixed `ClockReader`. Both start at one paused instant, and the test calls
`on_deadline` whenever a server timer fires, so their phases agree. The
test compares the notifications: unconfirmed APDUs octet for octet,
confirmed ones by their service parameters (the invoke IDs are each
host's), with destinations. PR 2's tests compare each step's
notifications as a set, not in order: since PR 2a the server starts its
confirmed workers before it sends the unconfirmed notifications of a
change, so the order between subscriptions is the server's scheduling,
which the `Reporter`'s queue doesn't follow. It pins what differs by
design, as `DIFFER` does, and fails on any other drift or on a pin that
stops differing.

| PR | Corpus |
|---|---|
| 1 | Subscribe, renew, cancel, cancel of nothing; every row of §5.8; the caps; a renewal that replaces a route. The facade sends nothing yet, so the flight machine (acknowledged, fenced, an aborted worker settling at its deadline) is tested in the core and through the server's own tests |
| 2 | Active_COV_Subscriptions before and after an expiry; the initial notification; changes under and over COV_Increment; Status_Flags alone; a Table 13-1 trigger; a property subscription's own increment; a confirmed report acknowledged, refused and timed out (hold-off); a change in flight (revisit); a renewal that fences a flight; an abandoned flight (the server's worker aborted, the facade never told) ending at its deadline, after which the next change is reported; Time_Remaining after time passes |
| 3 | OUT_OF_RANGE with Time_Delay 2 seeded by a write at 0.9 s (sent at 2.9 s) and back with Time_Delay_Normal; a value that reverts inside the delay; Event_Enable cleared (committed, not sent); recipients by day, window and transition; no clock (sequence numbers, fail-closed windows); AcknowledgeAlarm good, wrong state, wrong timestamp, local acknowledgement, and their notifications; GetEventInformation |
| #2319 | Each algorithm both ways; a FAULT_OUT_OF_RANGE fault; the pass-count delay at two intervals |
| #2320 PR 2b | SubscribeCOVPropertyMultiple, unconfirmed and confirmed: a timestamped reference; its history after a failed send, or held behind an outstanding report; an untimestamped reference; two contexts reporting one change; a report split across APDUs (all parts at once; one part per Ack); the Max_Notification_Delay backstop; a refused confirmed report and the follow-up at its hold-off's end; a cancellation with changes pending; Active_COV_Multiple_Subscriptions. On the fixture's fixed Device clock, so a capture at the wrong moment shows only in the core's own tests |

Mutation checks: PR 1, drop the flight deadline (the aborted-worker tests
must fail); PR 2, change the increment test's `>=` to `>`; PR 3, compare
the due instant with `>` instead of `>=`.

### 7.3 Changelog and docs

Each PR adds a `changelog.d` fragment, **Breaking (Rust API):** with a
migration note for its moved public items (PR 3 also `EventReporting`'s
`now` parameters, and a **Wire:** entry for the delay timing), and updates
`docs/rust-api.md`, `docs/architecture.md` and the core's README.

## 8. Footprint

### 8.1 Today and with a probe

The `objects` image (thumbv7m, Rust 1.99.0, the `firmware` profile, dev at
`c308eda25`) has 127,452 octets of `.text`, 15,108 of `.rodata` and
143,584 of flash: `bacnet-objects` 38,838, `bacnet-device-core` 23,648,
`bacnet-encoding` 20,534. Its budgets are 142,752 / 16,928 / 160,816.

For this design its scenario was extended by hand, measured and reverted:
a COV probe (a `Vec` table, the two subscribe services under the caps, an
increment test, an UnconfirmedCOVNotification) and an events probe
(`event-reporting` on, a Notification Class, AI-1's evaluate, tick and
commit, its recipients, an event notification, an AcknowledgeAlarm).

| Build (thumbv7m) | `.text` | `.rodata` | flash | memmove fns |
|---|---|---|---|---|
| `objects` today | 127,452 | 15,108 | 143,584 | 2 |
| + COV probe | 133,536 (+6,084) | 15,724 | 150,284 (+6,700) | 2 |
| + COV and events probes | 150,188 (+22,736) | 17,216 | 168,428 (+24,844) | 3 |

The events probe's 16,652 octets: 6,502 in `bacnet-objects`
(`event-reporting`, about 2,900 by #2215, and the Notification Class),
4,332 in `bacnet-encoding` (every event-values arm links), 3,752 in its own
logic, and 2,066 in `core`, compiler builtins and the smaller crates. No
`core::fmt` or 64-bit division; one more memmove routine
(`__aeabi_memmove4`), so the `reporting` rows budget three.

### 8.2 Estimates

The probes are floors. They lack the policy and identity checks, the
flight state, the Table 13-1 rows, Value_Source, Life Safety, the active
lists, timestamps, payloads for five event types, GetEventInformation and
the facade with its confirmed table. For those, the ratio
`bacnet-device-core` shows today (23,648 octets of `.text` from about 3,300
non-test lines, comments included, so about 7 a line), over the lines an
image would link:

| `reporting` image (thumbv7m) | `.text` estimate | From |
|---|---|---|
| PR 1, measured | 148.4 KB (flash 165.8 KB) | the `objects` image's 127.3 KB, plus the table and the two subscribe services (#2303) |
| PR 2, measured | 171.4 KB (flash 189.7 KB) | the image through the `Reporter`, trimmed by #2363 (§8.3) |
| + `event-reporting` (after PR 3) | 172-181 KB | ~2,200 lines, plus the measured objects and encoder share |
| + `event-enrollment` (#2319) | 207-221 KB | ~2,700 lines, the Event Enrollment object, `decode_event_parameter` |
| + the Multiple path (#2320) | +15-25 KB estimated; +137 KB measured before any trim (decision 18) | ~3,300 lines, the COVNotificationMultiple codec |

Flash runs about 18 KB above `.text`. So COV with intrinsic reporting fits
a 256 KB part with room; with Event Enrollment it is tight there and easy
at 512 KB. Each PR replaces its estimate with the measured row; PR 2's
came in about 15 KB over the estimate's top even after #2363's trims,
so PR 3's and #2319's rows should expect as much. The `firmware/link`
images share one 256 KiB `memory.x`, but riscv32imac's, the largest
(227,702 octets of flash after PR 2), have their own since #2363,
`memory-riscv32imac.x`: 512 KiB of flash, sized for an ESP32-C6-class
part, an RV32IMAC running in place from 4 MiB or more of external flash.

### 8.3 The `reporting` image and its heap

The `objects` image keeps its features off and its rows, the standing
check that reporting costs a non-reporting device nothing. PR 1 adds a
`reporting` image (feature `reporting = ["objects",
"bacnet-device-core/cov"]`) for thumbv7m, thumbv7em and riscv32imac, rows
from `--suggest`, so every PR has a budget row: PR 1's scenario subscribes,
renews and cancels through the staged functions; PR 2's runs through the
`Reporter`, reads Active_COV_Subscriptions and checks what it pops; PR 3
and #2319 add a Time_Delay, an acknowledgement and an enrollment.

A table entry is about 250 octets on a 32-bit target (two addresses, the
lifetime, flight and ticket, and the last delivered values on the heap);
PR 1's host test measures it. The scenario caps subscriptions at 8 and
sizes the outbox buffer; the 16 KiB arena grows only if the host test's
peak says so.

Measured for PR 1 (`firmware/link`, `tests/reporting.rs`; the outbox comes
with PR 2):

| | Measured |
|---|---|
| Entry, inline in the table's vector | 208 octets on thumbv7m, 288 on a 64-bit host |
| A table of 8 subscriptions from one subscriber, heap in all (64-bit host) | 2,848 octets, 356 each: the vector at capacity and the per-peer counts |
| The `reporting` scenario's peak heap (64-bit host) | 13,888 of the 16,384-octet arena (the `objects` scenario alone: 10,512) |

So the arena stays 16 KiB. Later an entry also holds the observation it
last reported (shared samples) and, while a confirmed report is out, its
flight's state.

Measured for PR 2 (2b, 2026-11-03), the scenario through the `Reporter`:

| | Measured |
|---|---|
| Outbox entry (token, reference, route, flags, the begin instant, place in the buffer) | 184 octets on a 64-bit host, beside the parameters in the buffer |
| Confirmed-table entry | 152 octets on a 64-bit host, and its parameters on the heap |
| The scenario's outbox | 6 entries and 256 octets: one change's six notifications |
| The `reporting` scenario's peak heap (64-bit host) | 19,800 octets with the image's features, 19,952 with the workspace's (20,840 when 2b landed): the arena grows to 24 KiB for the image |
| The `reporting` image (thumbv7m), as PR 2 left it | 192,856 `.text`, 211,200 flash, 24,672 RAM |
| The same after #2363's trims | 170,760 `.text`, 189,060 flash, 24,680 RAM |
| And after its owed-look fixes (decision 13) | 171,440 `.text`, 189,744 flash, 24,680 RAM |

#2363 attributed the 65.6 KB of `.text` the image added over `objects`
(thumbv7m, symbol sizes with LTO, so callers hold what they inline): the
largest pieces were `evaluate` with the value preparation it inlines (9.4
KB), the scenario and the `Reporter` calls inlined into it (about 17 KB),
out-of-line copies of the `Responder`'s write and discovery and of the
database builder, for the `objects` scenario the image also ran (13 KB,
most of which the one request path below inlines instead), B-tree code for the
table's flights, per-peer counts and two small sets (8.8 KB), three sort
instantiations (4.2 KB), the Multiple list's projection and encoder, and
no `core::fmt`. It trimmed, each change measured on its own (thumbv7m
`.text`; thumbv7em follows within a few hundred octets, riscv32imac by
about a third more):

| Trim | `.text` saved |
|---|---|
| The `objects` scenario's requests go through the same `Reporter`: one request path | 2,740 |
| The `Reporter`'s overlay leaves Active_COV_Multiple_Subscriptions to the Device (`ActiveCovOverlay<false>`): its table holds no COV-multiple reference until #2320 | 3,852 |
| The table's keys order as the Device's Active_COV_Subscriptions does, so the list needs no sort | 2,356 |
| Owed references are looked at with no sort of their keys | 2,068 |
| The table's flights and per-peer counts are sorted vectors (`VecMap`), and a flight's carried references and the Life Safety snapshots' objects vectors, not B-trees | 11,080 |

Together 22,096 octets (11.5%) on thumbv7m, 21,720 on thumbv7em and
29,480 on riscv32imac, with the wire bytes unchanged: the core tests, the
parity corpus (the Device's list included) and the image's own expected
summary, which the `objects` requests now meet through the `Reporter`,
hold to it. The image still passes §8.2's estimate (147-156 KB) by about
15 KB: it links the evaluation of every report form (Value_Source, Life
Safety, the Table 13-1 rows), and the overlay's read path.

## 9. Risks

**Hot paths.** Every committed write runs `evaluate` and `after_write`:
pure moves with logic changes apart, a lookup by object where the server
scans, no `dyn`. Before and after each PR, one at a time on an idle
machine: `stress-test -- cov`, `-- mixed --clients 5`, `bench bip_latency`,
and `bench encoding` as the control.

**Behaviour changes, deliberate.** A write-seeded delay fires when it ends,
up to a second later than today (§6.1). An aborted confirmed worker frees
its coordinate at the flight's deadline, not at once (§5.5). Iteration is
by object, where a `HashMap` was random, and the facade's expiry is exact
against the server's 30 s purge, which no reply shows. The server's
unchanged tests, the Python suite and the parity corpus hold the rest.

**Lock order.** Database, COV table, timed store, as documented
(`cov/timed_capture.rs:9`). The core takes borrows, not locks, so it can't
invert them; the one new pairing follows the order, and the PR 1 and PR 2
briefs audit the reverse under Tokio's fair lock, with #1494's check
(§4.5). The observer rule of device-core.md §4.5 still holds for COV
capture: a leaf mutex after the database, never an await.

**Time sources.** The monotonic `now` for lifetimes, flights, delays,
sweeps and passes; the Device clock frame for timestamps and windows. Wall
time passed as `now` would expire subscriptions when the clock is set; the
docs say so.

**Notification storms.** Today's bounds stay: 64 notifications and 65,536
octets of COV per input, 16 confirmed reports in flight per peer, one owed
revisit instead of a second report, the caps. Firmware adds the fixed
outbox, where COV yields to events and becomes owed (§4.2).

**Size and scope.** §8; each feature is checked alone in CI. Moved lines:
~5,640 in PRs 1-3, then ~2,720 under #2319 and ~3,300 under #2320.

## 10. Questions, as decided

All decided on 2026-11-02 (§11); kept here with the options weighed.

1. **Placement.** Modules in `bacnet-device-core` (§3.1), over a sibling
   crate or `bacnet-objects`.
2. **Features.** `cov`, `event-reporting` and `event-enrollment`, each kept
   only if its PR measures a saving, over a single feature.
3. **The Multiple form.** Its admission and flights in PR 1 and active list
   in PR 2; the service, timestamped histories and splitting in #2320,
   over moving all ~3,300 lines here.
4. **Event-state persistence.** None in v1, over a save hook; a hook would
   need its own design (stale timestamps, which delays restart).
5. **The image.** A separate `reporting` image, so the `objects` rows keep
   the non-reporting cost.
6. **A clock on firmware without one.** A portable clock set by
   TimeSynchronization and UTCTimeSynchronization on the monotonic `now`,
   lent to the database as its `ClockReader`, in #2321. Until it lands,
   recipient windows fail closed without a clock (§6.3).
7. **Local acknowledgement.** `acknowledge_local` in PR 3 (§6.4), over
   waiting for demand.
8. **Event Enrollment's home.** Its own issue, #2319, on §6.6 and
   Appendix A.4; #2303 closes on PRs 1-3.
9. **The sweep.** Deadline-driven, with the sweep period in `ReportConfig`
   (the server keeps 1 s), over a fixed 1 Hz sweep.

## 11. Decisions

**Maintainer, 2026-11-02:**

1. The design is approved as the plan of record (draft PR #2318).
2. Event Enrollment moves to its own issue, #2319. #2303 closes on PRs 1-3:
   the COV table, subscribe services and flights; COV notifications and the
   `Reporter`; intrinsic events, acknowledgement and GetEventInformation.
3. SubscribeCOVPropertyMultiple's timestamped notification path is
   follow-up #2320.
4. The synced firmware clock is follow-up #2321; recipient windows fail
   closed without a clock until then.

**Lead, 2026-11-02:**

5. From the first review: the flight state machine and a plain ticket land
   in PR 1, and the server keeps wake-ups only (§4.5, §5.2); a flight
   carries its own deadline and ends itself there, with a parity row
   (§5.5, §7.2); delays are anchored to instants (§6.1); the facade's
   confirmed table serves COV and events, with host invoke IDs, a host
   binding trait and counted unresolved recipients (§4.4); the outbox is a
   fixed buffer bounded by count and octets, where events may evict COV,
   and PR 1 measures a `BTreeMap` against a sorted `Vec` (§4.2, §5.2);
   without a clock, windows fail closed, an unspecified hour is all day,
   and clockless builds refuse restricted Recipient_List entries (§6.3).
6. Placement in `bacnet-device-core`; the three features; no event-state
   persistence in v1; a separate `reporting` image; local acknowledgement
   in PR 3; a deadline-driven sweep (§10).
7. The indefinite-lifetime refusal's error code (§5.8) is fixed in #2317,
   apart from the move.

**PR 1, 2026-11-02 (measured):**

8. The table keeps its entries in a sorted `Vec`, not a `BTreeMap` (§5.2).
   Both key on `CovSubscriptionKey`, ordered by monitored object first.
   - Size: the `objects` image with a probe subscribing, renewing,
     cancelling and expiring through `CovTable` (thumbv7m, Rust 1.99.0,
     `firmware` profile; probe reverted) has 148,228 octets of `.text` and
     165,344 of flash with the `Vec`, 153,004 and 170,120 with the
     `BTreeMap`: 4,776 octets less. The image without the probe has
     127,288 and 143,420. Both link no `core::fmt` and two more memmove
     routines (four in all).
   - Speed: a release-build probe on an Apple silicon host shared with
     other work (1,024 subscriptions on 64 objects, best of 20 runs, each
     variant run three times) admits all of them in 1.56 ms
     with the `Vec` and 4.1 ms with the `BTreeMap`, and finds one object's
     subscriptions (the fanout's lookup, with its expiry pass) in 382 ns
     and 2,609 ns. Removing every subscription one object at a time takes
     2.07 ms and 0.14 ms; removal follows only a cancellation, a deleted
     object or an expiry. `stress-test -- cov --steps 64,256,1024
     --duration 5` delivers the same 10, 39 and 156 notifications a second
     with either, bound by its change rate; peak resident memory was 18.4
     and 18.7 MB.
9. The facade isn't in PR 1, so a `cov` feature would save nothing yet: the
   `objects` image links none of the table. PR 2 measures it with the
   `Reporter` (§3.2).
10. From PR 1's review: a live worker stops half a timeout before its
    deadline, however long a send stalls, and reports a failure (hold-off).
    Its retries end the usual way before that, the last answer window
    whole, so the deadline only ever frees an aborted worker's coordinate;
    and a report that begins on a coordinate still marked by an unsettled
    one past its deadline fences it (§5.5). The cutoff runs from the
    instant the table began the report, which for the Single fanout is its
    evaluation (PR 2a): so the fanout starts every confirmed worker before
    it awaits any unconfirmed send, and a send that stalls can't shorten a
    report's answer window.
11. Lead, 2026-11-02: PR 2 splits in two. PR 2a moves the active lists,
    value preparation and Life Safety snapshots, and puts the server's
    Single fanout on `cov::evaluate`; PR 2b adds the `report` module, the
    confirmed table, `Bindings`/`InvokeIds`, `ActiveCovOverlay`, the
    image through the `Reporter`, the parity corpus and the `cov` feature
    measurement.

**PR 2b, 2026-11-03 (measured):**

12. No `cov` feature (decision 9, #2215's rule). With the core's `cov` and
    `report` modules gated off, the `objects` image (thumbv7m, `firmware`
    profile) has 127,544 octets of `.text`, and 127,360 with them: the
    linker drops what nothing reaches, so the gate saves nothing an image
    keeps. The core's `alloc` rlib for thumbv7m (release, opt-level z, one
    codegen unit, no LTO) holds 97,418 octets of `.text` with them and
    20,950 without, compiled but never linked by a device that doesn't
    report.
13. The `Reporter` asks the host's admission for every report, so the
    outbox checks room for each; a report it refuses leaves its
    subscription owed, looked at again as room frees: oldest first, as
    many as there are free entries. One refused again, or whose report
    the budget dropped, keeps its place and is counted once; an
    evaluation that reports it or finds nothing to report ends the debt,
    and a look drops one whose object or subscription is gone and hands a
    still-free entry to the next. A report too large for the outbox, or
    for one input's budget of octets, is counted and not owed. The owed
    queue is the `Reporter`'s alone: the server has no outbox, and the
    parity tests' outbox (64 entries, 65,536 octets) never owes, so the
    core tests, not parity, hold these paths. A subscription holds
    one unconfirmed entry: a newer report replaces it and keeps its turn,
    so a subscription changing faster than the link sends neither crowds
    out the others nor loses its place to every change. A confirmed
    report fails at the cutoff of decision 10, counted from when the table
    began it, however late it was popped, so its last answer is never
    awaited past the table's deadline. Its Life Safety writes report from
    the values each write changed, as the server's do.

**#2321, 2026-11-04 (measured):**

14. A Device clock in the core, `clock::SyncedClock`: the host's
    `bacnet-time` instant, the UTC day and hundredths it stood for, the
    zone and a synchronized flag; 32-bit arithmetic past `bacnet-time`'s
    long division of the elapsed seconds, and no allocation.
    - Until set it counts from local midnight, 1990-01-01, at the instant
      it was built (Clauses 12.11.23, 12.11.24), so the Device always has
      Local_Date and Local_Time and lists both services; windows fail
      closed and timestamps carry that date until a sync.
    - A local TimeSynchronization reads back as given; a UTC one less
      UTC_Offset, plus an hour in daylight-saving time (Clauses 16.7,
      16.8). The clock keeps the UTC instant with a fixed zone, as the
      server's `ServerClock` does; the parity tests hold the two to the
      same Local_Date and Local_Time octets and the same open destinations.
    - `DeviceClock` pairs it with the host's monotonic timer and is lent to
      the database as its `ClockReader` (`lend`). Each accepted sync lends
      a new copy (one `Arc` and a rebind of every object's clocks) rather
      than mutate a shared one, so the core takes no lock and no 64-bit
      atomic. The `Responder` runs a sync only when the Device lists the
      service; admission policy (sources, rate, step) is the host's, which
      hands over only what it accepts. A host's real-time clock sets it
      through `Responder::set_time`, and that counts as synchronized.
    - The zone changes at run time (#2395): `Responder::set_zone` and
      `Reporter::set_zone` move the clock and lend it again, as `set_time`
      does. A synchronized clock keeps its UTC instant and stays
      synchronized, so windows already open or closed are judged by the
      new local time from the next look; one never set keeps its count of
      local time from 1990. UTC_Offset and Daylight_Savings_Status stay
      read-only to a peer.
    - Restricted Recipient_List entries: stored, not refused, on a build
      with a clock. The list survives restarts (Clause 12.21.8) and the
      clock restarts unsynchronized, so a refusal that lifted only once
      synced would take an entry before a restart that it refused after.
      Stored, an entry fails closed until the clock is set and is judged
      by its own days and window from then on.
    - For PR 3, the refusal's signal: a `Responder` built without
      `with_clock`, checked on its own write path before a Notification
      Class's Recipient_List write commits (a host that runs
      AddListElement itself applies the same rule). Not a database with no
      `ClockReader`, and not inside `bacnet-objects`' Recipient_List
      write: the server shares that write, and judges windows by system
      UTC even when started with `start_clockless`, so it keeps storing
      restricted entries.
    - For PR 3, the window: ask the synchronized state,
      `SyncedClock::window`, never `db.clock_frame()`, whose unsynchronized
      1990 frame is valid and would open windows on a made-up date.
    - For PR 3, the filter: Clause 12.21.8 lets a device restrict a
      writable list only to the one configuration that always sends:
      every day, 00:00:00.00 to 23:59:59.99, all three transitions.
      `always_in_window` passes that configuration and refuses only
      entries that differ from it, so as the write filter it stays within
      the clause. It also passes other whole-day windows (across midnight
      with no gap, an unspecified hour) and any transitions, all of which
      the device can honour; whether to refuse an unspecified hour, which
      Table 12-25 doesn't allow in a destination, is a separate check.
    - The zone stays fixed: changing UTC_Offset or daylight saving at run
      time (Clause 12.11.25) is a follow-up for the server and the core
      together.
    - Size (`check-firmware-link.sh`, Rust 1.99.0, flash in octets): the
      `objects` image 144,648 to 144,868 (thumbv7m), 144,380 to 144,984
      (thumbv7em), 173,410 to 173,990 (riscv32imac); `reporting` 191,184
      to 191,628, 191,924 to 192,364, 229,458 to 229,870; RAM unchanged,
      no `core::fmt` or 64-bit division. The `Responder` reaches the sync
      step only through a function pointer `with_clock` stores: called
      directly, the unused path cost the `objects` image 4,720 octets on
      thumbv7m. The `reporting` image then gives its `Responder` a clock,
      synchronizes it and reads it back, so the size check's `core::fmt`
      and 64-bit division counts cover the clock: 195,404 (thumbv7m),
      196,228 (thumbv7em) and 234,458 (riscv32imac) octets of flash, still
      none of either, within budget; its scenario's host heap peaks at
      20,016 of 24,576 octets.

**#2320 PR 1, 2026-11-10:**

15. The Multiple form's timestamped histories and read cache are in the
    core (§5.6, Appendix A.5).
    - A `cov-multiple` feature gates them, on with `std`. Decision 12
      found a `cov` feature saved an image nothing, because the linker
      drops what nothing reaches; this one is kept for now on the same
      test, deferred: no image links the path until PR 2, which measures
      it and drops the feature if it saves nothing.
    - Claims are plain data with an explicit settle; a lodged claim has a
      deadline. The server lodges only a confirmed worker's claim, until
      its flight's deadline, and keeps Drop guards for everything else:
      its unconfirmed sends have no bound a deadline could sit behind
      without returning a claim under a send that is only slow.
    - One corner differs from dev, inside what decision 10 already calls
      abandoned: a worker still running past its flight's deadline no
      longer retires its claim with a late Ack. The claim went back at the
      deadline, and its changes go out again with the next report.
    - The memory charge of a pending change keeps the 8 octets its smaller
      instant saves (`CAPTURE_ALLOWANCE`), so overflow drops what it
      dropped. Dropping the allowance is a wire change for its own PR: a
      context at a 1476-octet local maximum would hold 120 of the
      smallest changes for 116.
    - The maps are B-tree maps, not the table's `VecMap`: the histories
      walk, retain and insert by entry, and no image links them yet. PR 2
      looks at them with the size row, as #2363 did for the table.
    - The histories' unit tests moved with them and run without std. They
      keep their bodies on a test host of some 150 lines (a hand-set
      clock, and a claim that gives itself back when dropped), so the
      assertions that held the server's store hold the core's explicit
      calls.
    - From the review: the server lodges the claim with the deadline it
      reads off the table (`CovTable::report_deadline`), never a second
      clock read, so claim and report end at one instant. The sizing
      constants and `envelope_len` are private to the crate; the module is
      public but unstable until PR 2 (Appendix A.5).
    - Size (`check-firmware-link.sh`, Rust 1.99.0): no image's numbers
      moved, since the feature is off in every one. Flash before and
      after, in octets: `objects` 144,868 (thumbv7m), 144,984 (thumbv7em),
      174,254 (riscv32imac); `reporting` 195,688, 196,500, 235,082; RAM
      16,480 and 24,680; no `core::fmt` or 64-bit division.

**#2320 PR 2a, 2026-11-10:**

16. The service, the capture, item assembly and the split are in the core
    (§5.6, Appendix A.5); the fanout follows in PR 2b.
    - The cut. A.5 gave about 1,890 lines to move and 600 new. The four
      items here came to about 1,500 lines in the core and 250 changed in
      the server. The fanout's decisions, the outbox variant, the
      `Reporter` and the image would pass the 2,500-line split point, so
      they are PR 2b.
    - The capture's clock: one instant per capture, passed in. It anchors
      only the Max_Notification_Delay wait, to the millisecond a timer
      resolves; the times a subscriber sees come from the Device clock
      frame, one per capture on dev too. Whether a reference is live is
      asked at one instant per capture as well.
    - The capture is three steps, not one call, so a host that locks reads
      its objects before it takes the histories, as the server did. The
      service is one call and holds them throughout: it runs once per
      request, and one function serves the server and, in PR 2b, the
      `Reporter`, so parity has less to prove.
    - The split returns what it gave up as data. A trait for the split to
      call the histories through would have kept the server's locked
      sections with no change at the call site, but a `Reporter` holds
      its histories outright and would have had to lend them twice, to the
      split and to the closure that times a field.
    - `follow_table` is a function of the histories module, not of the
      table: the table doesn't know the histories, and a build without
      `cov-multiple` has none.
    - `ITEM_FRAMING`, `value_len` and `request_header` are private to the
      crate again. `split`'s inputs (`ReportContent`, `Stamp`) and the
      claim's settling calls stay unstable until PR 2b. The changelog
      calls this Changed, not Breaking (lead, 2026-11-10): the three were
      public for a few days on dev and in no release.
    - The hosted names: the server's `TimedStore` is `HistoryStore` and
      its `TimedClaim` is `ClaimGuard`.
    - Size (`check-firmware-link.sh`, Rust 1.99.0): no image's numbers
      moved; the feature is off in every one.

**#2320 PR 2b, 2026-11-10:**

17. The Multiple fanout's decisions are in the core (§5.6, Appendix A.5).
    - The order between contexts. The fanout kept a change's contexts in a
      std hash map and sent them in its order, which varied from run to
      run. `by_context` keeps a B-tree map, so contexts report in key
      order: by recipient (direct before routed, then by address), then
      subscriber process identifier, the unconfirmed form before the
      confirmed one. A subscriber's contexts differ in process identifier
      and form, so that is the order it sees their reports in when one
      change or one deadline reports to several: a wire difference of the
      kind decision 15 accepted for the backstop's items, and named in
      the changelog. When one change spends its notification budget, the
      contexts left for later are the last in that order. Confirmed
      reports start in it too, each on a worker of its own.
    - Under a spent budget. Key order alone would make the last keys
      wait every time one change spends its budget
      (`CovPolicy::max_notifications_per_event`, 64 by default, or
      `max_notification_bytes_per_event`), where the hash order changed
      the loser per fanout. Timestamped changes are bounded by the
      backstop, which gives each context a budget of its own; a context
      with only untimestamped references has no history and no deadline
      and would never report. So a fanout over several contexts takes
      them in key order from a start that moves: `in_turn` rotates the
      contexts by an index the histories draw from a xorshift32 sequence
      (`TimedHistories::next_start`), advanced once per such fanout, with
      a fixed seed, no division and no state per context. Both hosts
      advance it through `in_turn` at the same point, so their turns
      agree. It is not a counter: a counter falls into step with a
      regular pattern of changes (two contexts, two objects changing in
      alternation, and each context always loses the same object). It
      promises an even spread, not a turn for any one context within a
      given number of fanouts: over the sequence's first 60 fanouts of
      three contexts under a budget of one, the three are served 18, 18
      and 24 times and none waits more than ten changes; with two served
      a change and two objects alternating, each context hears of each
      object 17 to 23 times in 60 changes. A follow-up pass gives each
      context a budget of its own and keeps plain key order.
    - What was tried first, and dropped (the first fix round): a list of
      the contexts a spent budget kept back, served first at their next
      turn. It was per context, while a report kept back is per object,
      so a context served in another object's fanout lost its place
      without the object it waited for. The octet budget refuses without
      ever counting as spent, so nobody was noted. And the server noted
      the turn after the table guard dropped, so it could note a context
      a cancellation had just removed. Fixing those meant waiters keyed
      by context and object, noted on two kinds of refusal, with a
      cleanup path per reference: too much state for a move.
    - The follow-up pass. Its keys came out of a hash set (the revisit
      queue), or timestamped before owed (the deadline), and a report
      lists its untimestamped items in the order its references are
      fanned out: one context's follow-up listed two changed references
      either way round. `fire_cov_revisits` sorts and deduplicates its
      keys, as the `Reporter`'s look does, so a follow-up's items, and
      the follow-ups of one pass, are in key order. Named in the
      changelog beside the context order.
    - Three steps, not one call, for the reason the capture has three
      (decision 16): the server reads its objects before it takes the
      table, and holds the table only for the decision. One call would
      have held the table's write guard across the split.
    - The split is lent the histories through a `RefCell` inside
      `Gathered::split`, for the field times it asks for while it sizes
      notifications. That keeps `Stamp` a closure and the split as it
      was, and makes both private: no host builds a `ReportContent`.
    - The steps settle nothing by themselves. A claim in a `Gathered`, a
      `Part` or a `FirstPart` is plain data, and the turn is the host's
      from `take_turn`. The server keeps its Drop guards for both, and
      for the worker's lodged claim: its sends can be cancelled, and its
      fanout returns early. Only `Gathering::Nothing` ends a turn inside
      a step.
    - The server's send loop for an unconfirmed report and its worker's
      outcome paths stay as they were, on the guards. What the core gives
      them is `carried`, `into_first`, `begin_confirmed` and `hold_off`.
      `hold_off` keeps dev's rule, one hold-off from the failure whatever
      the table made of the outcome, with the table's timing in place of
      a second reading of the config.
    - The store's mutex is held across `gather` and across `split`. A
      section that drops more than 64 repeats logs one debug line for the
      rest (decision 15's log difference); the counters hold every drop.
    - Narrowed with the move: `cov::multiple_items`, `cov::multiple_reads`
      and `cov::multiple_chunks` are private modules, `Part` re-exported
      from `cov::multiple_report`; the claim's splits and the histories'
      `field_time`, `note_field`, `adopt`, `note_reserve`, `set_sizing`
      and `lost_values` are private to the crate. The "unstable" notes
      are gone. A host names `Source`, the three steps' types, `Report`,
      `Part`, `Carried`, `FirstPart` and `Begun`.
    - A panic inside `gather` or `split` loses the claim the step held
      and leaves an unconfirmed context's send turn taken. Neither is
      known to panic: a reference that is not a COV-multiple one is left
      out, not unwrapped.
    - Size (`check-firmware-link.sh`, Rust 1.99.0): the move changed no
      image's numbers; the feature is off in every one (decision 18).

18. The `Reporter` runs SubscribeCOVPropertyMultiple behind
    `cov-multiple` (§5.6). The `reporting` image does not link the path
    yet: it is too large as it stands.
    - Measured with the image's scenario extended by one COV-multiple
      context (a timestamped Present_Value, its initial report and one
      change), the `firmware` profile, Rust 1.99.0, on a 512 KiB layout
      (the `reporting-multiple` build, below). Flash, without the feature
      and with it: 195,840 and 332,576 octets (thumbv7m), 196,652 and
      333,496 (thumbv7em), 235,290 and 402,354 (riscv32imac). RAM: 24,680
      and 36,968 on each, the arena from 24 to 36 KiB, for a heap peak of
      31,920 octets on a 64-bit host where the scenario without the
      context peaks at 19,136. No `core::fmt`; the same memmove routines;
      three 64-bit division routines on the thumb targets and two on
      riscv32imac, where the row allows none.
    - So the image passes the thumb targets' 256 KiB region by 70 KB and
      fails a standing limit of its row. §8.2 had estimated 15-25 KB.
    - Where it goes (thumbv7m `.text` by symbol, with LTO, so callers
      hold what they inline; taken before the fix round added 3 KB): B-tree code 36 KB (the histories' six
      maps, the read cache's two, the steps' maps and sets), sort
      instantiations 27 KB, the histories 40 KB, the report steps with
      the split 36 KB, the `Reporter` 44 KB (`handle`, with the service
      and the write path inlined, 27 KB of it), the COV-multiple codec
      9 KB. By crate: `bacnet-device-core` 52.5 to 108.8 KB, `core` 13.3
      to 46.5 KB, `alloc` from under 7 to 37.5 KB.
    - The feature stays (#2215's rule; decisions 12 and 15 left the
      measurement to this PR): an image without it keeps all of that. The
      `reporting` image leaves it off. Its rows still moved, by 152
      octets of flash on thumbv7m (195,840), RAM unchanged, for what is
      not behind the feature: the service an outbox entry and a
      confirmed-table entry carry, the two matches that turn it into a
      service choice when `pop` encodes, the second service `reply`
      accepts, and the look's test for a COV-multiple key. Gating them
      would put a `cfg` on the entry types, on `push` and `pop` and on
      every place that builds a `Notification`, for 152 octets.
    - To repeat the measurement: the firmware crate's `reporting-multiple`
      feature, off by default and not built by `check-firmware-link.sh`,
      turns the path on, adds the context to the scenario and gives the
      image the 512 KiB layout. Per target, `cargo build -p firmware-link
      --profile firmware --target <triple> --no-default-features
      --features reporting-multiple --lib --bin reporting`, then
      `python3 scripts/ci/firmware_size.py
      target/<triple>/firmware/reporting firmware/link/size-budget.toml
      <triple>.reporting`, which prints the sizes against the
      `reporting` rows; `cargo nextest run
      -p firmware-link --features reporting-multiple` checks the
      scenario and its heap peak on the host.
    - What the row needs first, as #2363 was for PR 2: the histories' and
      the read cache's maps as `VecMap`s, as the table's are; the sorts
      (`sort_by_key` on the retained references, `TimedClaim::in_order`,
      `rejoin`, `split_values`) merged or avoided; the 64-bit division
      found and removed. Then the scenario above and the row.
    - The `Reporter` fans a `Gathering::FollowUp` out in the same call,
      where it could have queued a look for the next `on_deadline`: the
      server's follow-up task runs at once, and the parity rows compare
      each instant's notifications.
    - A part of an unconfirmed report has a token of its own
      (`multiple_report::part_token`), a ticket the table issues and
      otherwise ignores: `Token` stays the one name a host reports a
      send's end by, and the `Reporter`'s numbering of tickets differs
      from the server's, which nothing on the wire shows.

## 12. Evidence

Code read at dev `c308eda25`. Sizes (§8): the `objects` image built with
`--profile firmware --features objects` for thumbv7m, measured with
`scripts/ci/firmware_size.py`, the probes reverted. Standard: Clauses
12.12, 12.21, 13.1, 13.2 (13.2.2, 13.2.2.1.4, 13.2.3, 13.2.4, 13.2.5,
13.2.5.3), 13.3, 13.4, 13.5, 13.6, 13.7, 13.14, 13.15, 13.16; Tables 13-1,
13-1a, 13-1a-2, 13-3 and 13-6; paraphrased.

## Appendix A. What moves, by PR

### A.1 PR 1: the COV table, flights, subscribe services

| Item | From | Lines |
|---|---|---|
| `CovSubscription`, the table (as `CovTable`), removal, purge, lookups | `cov.rs:51-422` | ~370 |
| `SubscriberEndpoint`, `CovRecipient`, keys, snapshot | `cov/identity.rs` | 221 |
| Admission, Multiple contexts included | `cov/admission.rs` | 463 |
| `CovPolicy`, validation, counts (atomics stay hosted) | `cov/policy.rs:1-304` | ~290 |
| `CovTimeRemaining`, `remaining_lifetime` | `cov/lifetime.rs:1-61` | 61 |
| Flight decisions as plain state (§5.5); the worker's `select!` stays | `cov/confirmed.rs:1-446` | ~300 |
| Ticket and completion order as a plain counter | `cov/observation_order.rs:1-109` | ~110 |
| SubscribeCOV and SubscribeCOVProperty checks | `handlers/cov.rs:1-247` | ~245 |
| **Total** | | **~2,060** |

Prep commits come first: the server's `HostClock`, `Ord` on `NpduAddress`
(`bacnet-encoding`). `SERVICES` names SubscribeCOV only from PR 2, once the
core sends what it subscribes to.

### A.2 PR 2: COV notifications and the facade

| Item | From | Lines |
|---|---|---|
| Active lists | `cov/active.rs:1-332`, `cov/active/multiple.rs:1-206` | ~530 |
| Samples and observations | `cov/sample.rs:1-110`, `cov/observation.rs:1-76` | ~185 |
| Value preparation | `cov/prepare.rs:1-144`, `flags.rs`, `reported.rs`, `value_source.rs` | ~470 |
| Life Safety snapshots | `life_safety_cov.rs` | 150 |
| Per-subscription decision and budget, out of the fanout | `server/cov_notifications.rs:13-76`, `:287-567` | ~250 |
| New: `report` (outbox, tokens, `Reporter`), the confirmed table, `Bindings` and `InvokeIds`, `ActiveCovOverlay` | | ~800 |
| **Total** | | **~1,585 moved, ~800 new** |

### A.3 PR 3: intrinsic events, distribution, acknowledgement

| Item | From | Lines |
|---|---|---|
| Intrinsic commit | `server/event_notifications.rs:128-203` | 76 |
| After-write evaluation, the sweep body | `event_notifications.rs:220-253`, `intrinsic_reporting_lifecycle.rs:26-65` | ~75 |
| Distribution (class, window, recipients, Event Log) | `event_notifications.rs:291-438`, less the send | ~110 |
| Profile and payload | `server/event_notification_profile.rs`, `event_notification_payload.rs:1-753` | ~850 |
| Timestamps, message text | `server/event_timestamp.rs:1-75`, `event_message_policy.rs` | ~85 |
| AcknowledgeAlarm and its notification | `handlers/alarm_event/acknowledge_alarm.rs`, `event_notifications.rs:442-458` | ~110 |
| GetEventInformation | `handlers/alarm_event/get_event_information.rs`, `event_information_page.rs:1-138` | ~550 |
| Fault pass | `fault_detection.rs:1-136` | 136 |
| New: due instants in the detectors (`bacnet-objects` `event.rs`), `next_due`, `acknowledge_local`, clockless windows | | ~200 |
| **Total** | | **~1,990 moved, ~200 new** |

### A.4 #2319: Event Enrollment

| Item | From | Lines |
|---|---|---|
| Algorithms | `event_enrollment/algorithms.rs` | 819 |
| Fault algorithms, reference, support | `event_enrollment/fault.rs`, `reference.rs`, `support.rs` | 534 |
| The pass | `event_enrollment/mod.rs:1-862`, non-test | ~770 |
| Commit and projection | `event_enrollment/commit.rs` | ~600 |
| Public wrappers, replaced by `event::enrollment` | `event_enrollment/api.rs` | -60 |
| **Total** | | **~2,720** |

### A.5 #2320: the Multiple notification path

Lines are non-test lines: on dev at `56a61f3f9` for PR 1's rows, and at
`3ea4624a1`, after PR 1, for PR 2's.

| Item | From | Lines | PR |
|---|---|---|---|
| Timestamped histories, the backstop | `cov/timed.rs:1-975` | 975 | 1, moved |
| `TimedStore` (`HistoryStore` since 2a): the mutex, `next_due` | `cov/timed.rs:977-1034` | 58 | hosted, in `cov/timed_store.rs` |
| History bound, envelope, eviction | `cov/timed/bound.rs:1-174` | 175 | 1, moved |
| Claims, splits, settling | `cov/timed/claim.rs:1-299` | ~300 | 1, moved; the Drop guards stay hosted |
| Drop accounting | `cov/timed/drops.rs:1-91` | 94 | 1, moved as counts and reports; the log lines stay hosted |
| Owed untimestamped references | `cov/timed/owed.rs` | 80 | 1, moved |
| Read cache | `cov/multiple_reads.rs` | 186 | 1, moved |
| Capture at the mutation boundary | `cov/timed_capture.rs` | 330 | 2a, moved; `cov/store_capture.rs` (124) stays hosted |
| Item assembly | `cov_notifications/multiple_items.rs` | 262 | 2a, moved |
| Split to the APDU size | `cov_notifications/multiple_chunks.rs:1-427` | 427 | 2a, moved |
| Service checks | `handlers/cov.rs:5-28`, `:153-351` | ~225 | 2a, moved |
| The histories follow the table | `cov.rs:527-554` (in `sync`) | ~30 | 2a, moved |
| Multiple fanout: its decisions | `cov_notifications/multiple.rs` | 626 | 2b, moved as `cov::multiple_report` (553 lines without comments); the sends stay hosted (254) |
| New: `NotifyService::CovMultiple`, the `Reporter` running the service, parity | | ~640 (522 in `report/reporter_multiple.rs`) | 2b |
| The image's size row | | | not in 2b: the path needs trimming first (decision 18) |
| **Total** | | **~1,810 moved in PR 1; ~1,275 in PR 2a; ~550 moved and ~640 new in PR 2b** | |

PR 1's order differs from §7's rule. The read cache moved first, in one
commit that also made its two maps B-tree maps, as the core has no hash
map; they are only looked up by object, never walked, so no order follows
from them. The histories could not move as they were: they held Tokio
instants and a Tokio `Notify`, the server's atomics, `tracing` and std
hash maps. So their four logic changes (time, plain state, claims,
collections) were made in place in the server, one commit each with the
server's suites as the proof, and the histories then moved in a pure move
commit. Apart from the read cache's two maps, no commit both moves and
changes logic. The capture stays for PR 2 because its entry points are
methods of the server's table wrapper and its write observer needs the
table and the histories borrowed together, which the `Reporter` has.

PR 2a follows §7's rule with no exception: none of its files was sans-IO
as it stood, so each got its logic changes in place first and moved in a
pure move commit after.

| Item | What was not sans-IO | Changed in place |
|---|---|---|
| Item assembly | a std hash map and hash set | B-tree forms |
| Split | the server's claim guard (the store's mutex, a `Drop` that gives back), a `warn!`, std hash sets, the server's timestamp helper | takes a plain claim and returns a `Split`; the helper moves beside it |
| Capture | the store handle and its mutex, a clock read under it, entry points on the server's table wrapper | plain `TimedCapture`, `CaptureRead` and `CaptureQueue`; the store part moves to `cov/store_capture.rs` |
| Service checks | a clock read, and the wrapper's `sync` between admission and capture | one function over the table, the histories and `now`; `follow_table` out of `sync` |
| Multiple fanout (2b) | Tokio locks on the database and the table, the store's mutex, the claim guard's and the send turn's `Drop`, the transport sends, the invoke ID lease, `tracing`, the server's atomics, three std hash maps, a clock read per step | the decisions as plain steps in `cov/multiple_report.rs` over the table, the histories and one `now`; the sends, the lease, the logs, the atomics and the guards stay in the fanout |

The two move commits changed imports, visibility, module declarations and
feature gates, and added the field docs a public item needs. The first
did two things more, both because the split now lives beside the
histories: it narrowed `cov::timed`'s `ITEM_FRAMING`, `value_len` and
`request_header` to the crate, and rewrote that module's paragraph on
what is unstable to match. No logic changed in either.

PR 1's review left three notes for PR 2. All are done in 2a: the hosted
names are `HistoryStore` and `ClaimGuard`; the capture reads the clock
once (decision 16); `ITEM_FRAMING`, `value_len` and `request_header` are
private to the crate.

PR 2b follows §7's rule too. Its logic commits, each with the server's
suites as the proof, then one pure move and one commit of visibility:

| Commit | What changed |
|---|---|
| Contexts in key order | the fanout's and the follow-up pass's hash maps are B-tree maps: the wire-visible order (decision 17), with a wire test |
| Plain steps | `Reading::read`, `Reading::gather`, `Gathered::split`, `Report::into_first`, `carried` and `by_context` in `cov/multiple_report.rs`; the fanout calls them and guards what they return; one instant per fanout |
| A confirmed report's begin, the hold-off | `begin_confirmed` and `hold_off`; the table wrapper runs the first under the table guard with the store locked last |
| Move | the file to `bacnet-device-core`: imports, `pub` for `pub(crate)`, the module declaration, five doc links |
| Visibility | the split, its inputs and the calls only the steps need are private to the crate; the "unstable" notes go |

The notes PR 2a left for 2b came out as follows. The contexts are in a
B-tree map, named in the changelog. `Stamp` stays a closure: the split is
lent the histories through a cell inside `Gathered::split`, so no host
builds a `ReportContent` and the split's inputs are private rather than
reshaped. The `Reporter` gets the explicit forms of the hosted guards
with the steps: `take_turn` and `end_send`, `begin_confirmed` with
`lodge` and `settle`, and `give_back`.

The notes for the `Reporter` came out as planned: an outbox entry carries
its service, and only a `NotifyService::Cov` entry replaces or is
replaced; a part has its own token (`multiple_report::part_token`); a
confirmed report begins when it is queued; `settle` runs `follow_table`;
`next_deadline` merges the histories'. One thing was added: a
`Gathering::FollowUp` is fanned out in the same call, as the server's
follow-up task does at once, so a host needs one `on_deadline` at a
hold-off's end, not two.

Left for a follow-up, with the size row: the path's footprint (decision
18). The scenario the `reporting` image would run is in the firmware
crate behind `reporting-multiple` (a SubscribeCOVPropertyMultiple for
one timestamped Present_Value, its initial report and one change, two
58-octet notifications, the arena at 36 KiB).

A host of the `Reporter` has one duty the server gets from a guard: it
reports the end of every unconfirmed part it pops (`delivered`). Until
then the report holds its context's send turn, and every later fanout of
the context stands back; an outbox entry has no deadline that would end
the turn for a host that never reports. Whether the turn should get one,
as a confirmed entry has its cutoff, is open; this PR adds none.
