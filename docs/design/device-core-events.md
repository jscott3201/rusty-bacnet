# Design: COV and event reporting in the device core (#2303)

Status: **draft for the maintainer's approval**; nothing here is decided
until §11 records it. Phase 3 of #2178, milestone 0.16.0. Line references
are to dev at `c308eda25`; paths without a crate are in
`crates/bacnet-server/src/`. BACnet references are clause and table numbers
in ANSI/ASHRAE 135-2020, paraphrased.

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
called. The core owns the subscription table as a plain value. The host
owns the lock around it, the transport, address resolution and confirmed
delivery. The server runs on the moved code, as it does for phases 1 and 2,
and parity tests hold it to the same octets.

## 2. Goals and non-goals

**Goals (v1 of phase 3).** One implementation, run by firmware and by
`BACnetServer`, of:

- SubscribeCOV and SubscribeCOVProperty (Clauses 13.14, 13.15): admission,
  renewal, cancellation, lifetimes, caps and the initial notification;
- COV evaluation (Tables 13-1, 13-1a, 13-1a-2) after a committed write, a
  local change or a time-driven commit, and the notification it earns,
  confirmed or not (Clauses 13.6, 13.7); Active_COV_Subscriptions from the
  same table;
- intrinsic reporting for every type the catalogue implements it for
  (§6.1), with Time_Delay and Time_Delay_Normal measured on the caller's
  clock, and the periodic `ReliabilityEval` fault pass;
- Notification Class distribution: priority, Ack_Required, recipients by
  day, window and transition (Clauses 12.21, 13.2.5);
- AcknowledgeAlarm with Acked_Transitions and the acknowledgement
  notification (Clauses 13.2.3, 13.5), and GetEventInformation, which a
  device with event-initiating objects has to execute (Clause 13.2.4);
- Event Enrollment's algorithmic reporting (Clauses 12.12, 13.3, 13.4).

Also: no I/O, locks, async or clock in the core; the server's octets
unchanged, shown by parity tests under paused time; and a `reporting`
firmware image, while the `objects` image keeps its rows.

**Non-goals (v1)**, all staying hosted (§7.5):

- the SubscribeCOVPropertyMultiple service, its notifications and
  timestamped changes (§5.6, open question 3); the table admits its
  contexts from PR 1, so the server keeps working;
- transport, routing, segmentation, DCC, the server's confirmed-request
  transactions and retries, and resolving a Device recipient to an
  address: the core asks for a send and is told how it ended (§4.2);
- receiving notifications (forwarding, the received-event log), and
  GetAlarmSummary and GetEnrollmentSummary, which Clause 13.2.4 advises
  new devices against;
- persisting subscriptions (Clause 13.1 lets a COV server lose them on
  restart) or event state (§6.8); COV_Period, which the server lacks;
  policy, authorization and audit, as in phases 1 and 2.

## 3. Placement and dependencies

### 3.1 In `bacnet-device-core`, behind features

```
Layer 5:   bacnet-objects       objects; CovReporting, EventReporting, EnrollmentEval, ReliabilityEval
Layer 5.5: bacnet-device-core   requests (phases 1-2) and reporting (this design):
             report             Outbox, Notification, Token, Delivery; the Reporter facade   (§4)
             cov                CovTable, subscribe, evaluate, active lists                   (§5)
             event              after_write, tick, distribute, acknowledge, GetEventInfo      (§6)
             event::enrollment  the Event Enrollment pass                                     (§6.6)
Layer 6:   bacnet-server        locks, transport, routing, retries; tasks that call the above
```

The modules join the existing crate (open question 1), sharing `reply`,
`Inbound`, the `Responder`, `ReadOverlay` (§5.7) and `WriteCommitObserver`.
The one new dependency is `bacnet-time` (the core's `std` turns on its
`std`, for `HostClock`); `bacnet-services` already has the codecs.

### 3.2 Features

| Feature | Turns on | Gates |
|---|---|---|
| `cov` | `dep:bacnet-time` | `cov`, `report`; SubscribeCOV and SubscribeCOVProperty in `SERVICES` and the facade |
| `event-reporting` | `dep:bacnet-time`, `bacnet-objects/event-reporting` | `event`, `report`; AcknowledgeAlarm, GetEventInformation |
| `event-enrollment` | `event-reporting` | `event::enrollment`, which links `decode_event_parameter` (§6.6) |
| `std` (default) | the three above, beside `structured-writes` | hosted builds unchanged |

The linker drops what nothing calls, but the facade dispatches by service,
so an arm for SubscribeCOV links the COV path whether or not anyone
subscribes. #2215's rule applies: a feature stays only if its PR measures a
real saving (its spike found COV, audit and clock features worth under 400
octets each in `bacnet-objects`; these are worth kilobytes, §8).

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

`check-no-std.sh` gains a `bacnet-device-core` entry with every feature on
`alloc`; `check-firmware-link.sh` the `reporting` image (§8.3) on the
`objects` image's targets; `check-default-features.sh` each feature alone
with `alloc`, so a doc link into a gated module fails there.

## 4. API

### 4.1 Two layers, as in phases 1 and 2

The server awaits between steps (to take a lock, to send, to wait for an
ACK); the core can't. So the core has **staged functions** the server calls
under its guards (`CovTable` methods, `cov::evaluate`, `event::tick`,
`event::distribute`, ...), and a **`Reporter`** that strings them together
for a host with nothing to await: firmware and the parity tests. Every
staged function borrows what it touches and locks nothing.

### 4.2 Outbox, notifications and delivery

Evaluation runs under the host's database guard and encodes what it
decides to send, so nothing reads the database after the guard drops. The
notifications queue in an `Outbox` the host drains.

```rust
pub enum Destination {
    Subscriber(SubscriberEndpoint), // the route a subscription arrived on (cov/identity.rs:8-23, moved)
    Recipient(BACnetRecipient),     // a Notification Class destination; the host resolves a Device
}
pub enum NotifyService { Cov, Event }         // CovMultiple with open question 3
pub struct Notification {
    pub token: Token,                         // its end comes back through `delivered`
    pub to: Destination,
    pub service: NotifyService,
    pub confirmed: bool,
    pub priority: NetworkPriority,            // Table 13-6 for events; normal for COV
}
pub enum Delivery { Sent, Acknowledged, Failed, NotSent }  // NotSent: DCC, no route; no attempt made
impl Outbox { pub fn pop(&mut self, params: &mut BytesMut) -> Option<Notification>; }
// `reply` gains `unconfirmed_request(service, params, out)` and
// `confirmed_request(invoke_id, max_apdu, service, params, out)`.
```

The outbox holds service parameters, not APDUs: a confirmed request's
invoke ID is the host's. Unconfirmed sends report back too, since the
server completes a COV observation only once its send succeeded
(`cov_notifications.rs:559-563`). Within one input the order is the
server's on the write path (`server/requests/mod.rs:665-719`): the
acknowledgement notification, event notifications, COV, then initial
notifications of new subscriptions. `next_deadline` is the earliest of the
next tick, pass, expiry and hold-off end (§5.5), by `bacnet_time::earliest`.

### 4.3 Inputs

| Input | Staged function | Server caller today |
|---|---|---|
| A subscribe request | `CovTable::subscribe`, `subscribe_property` | `server/requests/mutations.rs:395-467` → `handlers/cov.rs:62-247` |
| Objects a commit changed | `cov::evaluate`, `event::after_write` | `requests/mod.rs:692-704`, `local_write_finish.rs:98-160`, `committed_cov.rs:77-108` |
| A due deadline | `CovTable::expire`, `event::tick`, `event::fault_pass`, `event::enrollment::pass` | `server/lifecycle.rs:661-672`, `:688-742`; `intrinsic_reporting_lifecycle.rs:5-76` |
| How a notification ended | `CovTable::delivered` | `cov_notifications.rs:538-565`, `cov_notifications/confirmed.rs:47-288` |
| AcknowledgeAlarm, GetEventInformation | `event::acknowledge`, `event::get_event_information` | `server/requests/acknowledge_alarm.rs`, `requests/event_information.rs` |
| An object or a peer gone | `CovTable::remove_object`, `remove_peer` | `requests/mutations/object_lifecycle.rs:165-168`, `server/mod.rs:843-851` |

"Objects a commit changed" is what phase 2 already reports for network
writes (`Written.objects`, the `WriteCommitObserver` hooks). A firmware
application that changes a value itself (a sensor sample, an output's
feedback) reports the object the same way; the core doesn't poll. The
server works the same way: its local writes fan out explicitly
(`local_writes.rs`).

### 4.4 The `Reporter` facade (firmware, parity tests)

```rust
pub struct ReportConfig {
    pub cov: CovPolicy,                   // moved, cov/policy.rs:16-54; firmware lowers its caps
    pub outbox: usize,                    // most notifications queued at once (§9)
    pub confirmed: Option<Confirmed>,     // timeout and retries; open question 4
    pub enrollment_interval: Option<u32>, // seconds; None runs no Event Enrollment pass
    pub fault_detection: bool,            // the 10 s ReliabilityEval pass
}

impl Reporter {
    pub fn new(identity: &DeviceIdentity, config: ReportConfig, now: Instant) -> Result<Self, Error>;
    /// COV services, AcknowledgeAlarm and GetEventInformation here, the rest
    /// through `responder.handle`; then evaluates what the request changed.
    pub fn handle(&mut self, responder: &Responder, db: &mut ObjectDatabase,
        peer: &SubscriberEndpoint, request: Inbound, now: Instant, out: &mut BytesMut) -> Written;
    pub fn changed(&mut self, db: &mut ObjectDatabase, objects: &[ObjectIdentifier], now: Instant);
    pub fn next_deadline(&self) -> Option<Instant>;
    pub fn on_deadline(&mut self, db: &mut ObjectDatabase, now: Instant);
    pub fn pop(&mut self, params: &mut BytesMut) -> Option<Notification>;
    pub fn delivered(&mut self, token: Token, outcome: Delivery, now: Instant);
    pub fn overlay(&self, now: Instant) -> ActiveCovOverlay<'_>;   // §5.7
}
```

A firmware loop waits for a frame or `next_deadline`, calls `handle` or
`on_deadline`, sends the reply, then pops notifications, sends each and
reports it with `delivered`. `peer` is a parameter rather than a field of
`Inbound` because COV identity needs the route (the link MAC and, behind a
router, the original network address), which `CommandOrigin` doesn't carry.

### 4.5 The server on the staged functions; lock order and #1494

The server keeps `Arc<RwLock<ObjectDatabase>>` and
`Arc<RwLock<CovSubscriptionTable>>`; its table type becomes a wrapper:

```rust
pub struct CovSubscriptionTable {
    core: bacnet_device_core::cov::CovTable,  // the moved table: plain data
    flights: FlightWaiters,                   // hosted: a Tokio Notify per token in flight
    revisits: Arc<CovRevisits>,               // hosted: the revisit task's wake-up
    counters: Arc<AtomicCovCounters>,         // hosted metrics, fed from the core's counts
    timed: TimedStore,                        // hosted until open question 3
}
```

Every path takes the database guard, then the table guard, calls the core
with both borrowed, and drops both before any send; the core takes no lock,
so it can't invert them. Two paths change. The Single fanout, which closes
the database guard before reading the table today (`cov_notifications.rs:314`,
`:388`, `:473`), holds database (read) then table (write) for one
synchronous `evaluate`, the documented order the subscribe services already
use (`mutations.rs:410-411`). Completing an observation
(`cov_notifications.rs:562`, `cov_notifications/confirmed.rs:103`, `:207`)
becomes `delivered`, under the table guard alone, as now.

The core adds no future, `select!`, `join` or `FuturesUnordered`. The
`select!`s that exist stay hosted and hold no lock: `cov_fanout.rs:116`
(guard dropped at 112), `cov_notifications/confirmed.rs:194` (delivery
against the fence) and `cov/timed.rs:1014` (store mutex released). PR 2
rewrites the second; its review brief carries #1494's check that no sibling
future sits unpolled while an arm waits for the table lock.

## 5. COV

### 5.1 Services

| Service | Core | Server today |
|---|---|---|
| SubscribeCOV | `CovTable::subscribe` | `handlers/cov.rs:62-143` |
| SubscribeCOVProperty | `CovTable::subscribe_property` | `handlers/cov.rs:148-247`; `validate_cov_property`, `:34-56` |
| SubscribeCOVPropertyMultiple | admission only, `CovTable::subscribe_multiple` (PR 1) | `handlers/cov.rs:252-496`, `cov/admission.rs:89-201` |

Each takes the service parameters, as phase 2's `write_property` does,
decodes them with `bacnet-services` (a failure is a Reject), checks the
object and property against the object's `CovReporting` (`bacnet-objects`
`traits/capabilities/cov_reporting.rs:96-162`), applies the caps, and then
renews, cancels or adds. A renewal restarts the lifetime; a cancellation
that matches nothing still succeeds (Clause 13.14.2). A subscription or
renewal owes an initial notification, which the core queues behind the
reply (§4.2).

### 5.2 The table and its identity

The table moves whole (`cov.rs:51-422`, `cov/identity.rs`,
`cov/admission.rs`, `cov/policy.rs:1-304`), with three changes:

- **Collections.** The `HashMap` becomes a `BTreeMap` keyed with the
  monitored object first, so a change to one object finds its
  subscriptions by range, where `subscriptions_for` (`cov.rs:342`) scans.
  That needs `Ord` on the key, so `MacAddr` and `NpduAddress` derive it
  (open question 7).
- **No shared state inside.** `Arc<AtomicCovCounters>`, the in-flight
  tracker (a std mutex and a Tokio semaphore, `cov/policy.rs:306-381`), the
  flight markers' `Notify` and `CovRevisits` stay in the server's wrapper
  (§4.5); the observation owner's atomic ticket (`cov/observation_order.rs`)
  becomes a plain counter in PR 2. The core keeps plain counts, which the
  server adds to its atomics: 32-bit targets have no `AtomicU64`.
- **Time.** `expires_at` becomes an `Option<bacnet_time::Instant>`, and
  every lookup that purges takes `now` instead of reading `runtime_clock`.

Identity doesn't change: the original client address (`CovRecipient`, a
direct MAC or the routed SNET/SADR), process, object and, for a property
subscription, property and index (`cov/identity.rs:29-34`, `:85-110`), as
Clause 13.14.2 ties a context to them. The route (the link MAC of the
client or its router) rides beside it in `SubscriberEndpoint`, refreshed
on renewal.

The caps are `CovPolicy`'s: 1,024 in all, 64 per peer, 64 reserved, 16
indefinite per peer, 16 confirmed in flight per peer (`cov/policy.rs:39-54`).
Firmware lowers them in `ReportConfig`; each entry is a heap allocation
(§8.3).

### 5.3 Lifetimes and expiry

Lifetimes are seconds (Clause 13.14.1.1.4), and the core keeps the server's
rules. SubscribeCOV takes any lifetime, zero meaning indefinite, which
covers the clause's 28,800 s floor (`handlers/cov.rs:120-126`).
SubscribeCOVProperty refuses zero with SERVICES / VALUE_OUT_OF_RANGE
(`:180-187`). Indefinite subscriptions are a policy cap
(`allow_indefinite_subscriptions`, 16 per peer).

An expired entry goes three ways, as now: a lookup that meets it purges it;
`expire(now)` purges what is due (the server's 30 s task,
`lifecycle.rs:661-672`); a renewal replaces it. The facade's
`next_deadline` names the earliest expiry, so firmware wakes exactly then.
The server keeps its 30 s cadence, which no reply can tell apart, since
lookups purge first.

### 5.4 Change detection and increments

The decision code moves as is (`cov/sample.rs:1-110`,
`cov/observation.rs:1-76`, `cov/observation_order.rs:1-109`,
`cov/prepare.rs:1-144`, `cov/flags.rs`, `cov/reported.rs`,
`cov/value_source.rs`, `life_safety_cov.rs`), over the objects crate's
`cov_change` (`effective_increment`, `reports_change`). Whole-object
subscriptions test the leading value against COV_Increment,
Status_Flags, then the Table 13-1 row (`CovReportedProperty`: a trigger
sends, a value rides along); property subscriptions use their own
increment, else the object's for a numeric Present_Value, else any change
(Table 13-1a); commandable values add Table 13-1a-2's properties; Life
Safety objects compare whole snapshots. The comparison is with the last
*delivered* observation, and `evaluate` applies the per-input budget
(`EventBudget`, `cov_notifications.rs:13-76`: 64 notifications, 65,536
octets).

### 5.5 Confirmed notifications

A confirmed report's state is spread today over a `FlightMarker` (a mutex
and a `Notify`), an RAII `ConfirmedFlight` whose drop settles it, and a
Tokio `Instant` hold-off (`cov/confirmed.rs:70-216`). The decisions in them
form a small state machine, which the core keeps as explicit state per
subscription:

```
        evaluate (a change)                     delivered(Acknowledged)
  Idle ─────────────────────► InFlight(token) ─────────────────────────► Idle, then the owed revisit
   ▲                            │  │  a change meanwhile: marked owed
   │  delivered(NotSent)        │  │
   ├────────────────────────────┘  │  delivered(Failed)
   │                               ▼
   └────── now >= until ────── HoldOff(until = now + timeout × (retries + 1))
```

The hold-off is the server's (`cov_notifications/confirmed.rs:158`; 12 s
with the defaults), and an owed change is reported once the flight ends
(#896's revisit, `cov_fanout.rs:107-134`). The server keeps the send, the
invoke ID lease, the retries and the `Notify` that wakes the fence, and
calls `delivered` where it settles a flight today.

### 5.6 The Multiple form

Clause 13.16's SubscribeCOVPropertyMultiple is the only Multiple form; its
notifications are the COVNotificationMultiple services. PR 1 moves its
admission (`cov/admission.rs:89-201`) and its active list with the table,
so the table is one type in both hosts. The rest stays hosted until open
question 3 is answered: the service checks (`handlers/cov.rs:252-496`), the
timestamped histories (`cov/timed.rs`, `cov/timed/*`,
`cov/timed_capture.rs`), the read cache (`cov/multiple_reads.rs`), the
split to the subscriber's APDU size (`cov_notifications/multiple_chunks.rs:1-426`,
`multiple_items.rs`) and the Multiple fanout (`cov_notifications/multiple.rs`).

Most of it is synchronous already (`take_due(now)` returns its next
deadline, `timed.rs:574-635`); the exceptions are the `TimedStore` mutex
and `select!` (`timed.rs:981-1034`) and the claim's RAII requeue
(`timed/claim.rs:272-290`). Clause 13.1's Max_Notification_Delay backstop
becomes one more deadline.

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
| Object missing | OBJECT / UNKNOWN_OBJECT | 13.14.1.3.1 | `handlers/cov.rs:104-118` |
| No `CovReporting`, or whole-object subscriptions refused | OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED | 13.14.1.3.1 | same |
| Property not reportable | PROPERTY / NOT_COV_PROPERTY | 13.15 | `:34-56` |
| A cap reached | RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT | 13.14.1.3.1 | `cov/admission.rs:425-452` |
| SubscribeCOVProperty with a zero lifetime | SERVICES / VALUE_OUT_OF_RANGE | 13.14.1.3.1 | `:180-187` |
| A lifetime without the confirmed flag | Reject INCONSISTENT_PARAMETERS | 13.14.1.1.3, 18.9 | `:91-93`, `:189-191` |
| Parameters don't decode | Reject, the decoder's reason | 18.9, 20.1.8 | `:87`, `:176` |

A cancellation is checked before the object, so cancelling a subscription
to an object that is gone succeeds (Clause 13.14.2).

## 6. Events

### 6.1 Intrinsic reporting

The objects hold the detectors and the commit kernel already
(`bacnet-objects` `event.rs`, `event/history.rs:69-110`) behind
`EventReporting`: OUT_OF_RANGE (Analog Input, Output, Value),
CHANGE_OF_STATE (Binary and Multi-state Inputs and Values, Access Zone,
Access Door), COMMAND_FAILURE (Binary and Multi-state Outputs) and
BUFFER_READY (the logs), each with fault precedence and
CHANGE_OF_RELIABILITY into and out of FAULT (Clause 13.2.5.3).

What moves is the driver:

- `event::after_write` calls `evaluate_intrinsic_reporting` on each changed
  object (`server/event_notifications.rs:220-253`). That seeds a delayed
  transition, or fires one whose delay is zero.
- `event::tick`, once a second, walks every object under the write guard,
  refreshes Event_Algorithm_Inhibit from its reference and calls
  `tick_intrinsic_reporting` (`intrinsic_reporting_lifecycle.rs:26-65`).
  Only the tick confirms a delayed transition, so repeated writes can't
  shorten a delay; it is also what turns a Reliability change into a FAULT
  transition (Clause 13.2.2).

Time_Delay and Time_Delay_Normal are seconds (Clause 13.3), counted in
ticks. A late wake must not run several ticks at once and squeeze a delay:
the server's interval collapses missed ticks (`MissedTickBehavior::Delay`,
`intrinsic_reporting_lifecycle.rs:16`), and `on_deadline` runs at most one
tick and schedules the next a second after `now`.

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
which Clause 13.2 permits a device without a clock. That code runs without
std already. The intrinsic message text (`server/event_message_policy.rs:9`)
uses `format!`; the core formats nothing (#2187), so it is built by hand.

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
system-UTC value, a clockless firmware `None` (open question 5). Routing
stays the host's: address routes, Device bindings
(`event_awaited_recipients.rs`) and the local Notification Forwarder
(`event_forwarding.rs:216-256`).

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
(`commit.rs:152-177`), and takes the interval and `now` as arguments.

The pass runs every `enrollment_interval` seconds (10 by default,
`config.rs:274`), and a delay of d seconds waits `ceil(d / interval)`
passes, so it never fires early; the core keeps that model. It is the
costliest feature for an image: Event_Parameters is read through
`decode_event_parameter` (`fault.rs:39-54`), the decoder #2210 removed from
the `objects` image (11,424 octets of thumbv7m `.text`).

### 6.7 The fault pass

`FaultDetector::evaluate` (`fault_detection.rs:93-135`) visits each
object's `ReliabilityEval` every 10 s while `enable_fault_detection` is on
(off by default, `config.rs:272`). It moves with its warning dedup turned
into a returned list and its unused `comm_timeout` dropped. The objects
whose Reliability changed feed COV in the same call, and event detection
sees the new Reliability at the next tick (§6.1), as now.

### 6.8 Event-state persistence

Event_State, Acked_Transitions, Event_Time_Stamps, Event_Message_Texts and
Event Enrollment's pending state live in memory today, and after a restart
the objects start from their configured state. The core keeps that. The
Notification Class saves its own Recipient_List, which Clause 12.21.8 says
outlives a restart, through its persistence trait, unchanged. Saving event
state would be a feature with its own design (open question 8).

## 7. What moves, what stays, and the slices

Moved means moved: the server imports the core's item, no alias is left,
and every caller and test changes in the same PR (AGENTS.md, pre-1.0).
Counts are non-test lines. Each PR is pure moves first, then the logic
changes (time, collections, the flight state) in commits of their own.

### 7.1 PR 1: the COV table and the subscribe services

| Item | From | Lines |
|---|---|---|
| `CovSubscription`, the table (as `CovTable`), removal, purge, lookups | `cov.rs:51-422` | ~370 |
| `SubscriberEndpoint`, `CovRecipient`, keys, snapshot | `cov/identity.rs` | 221 |
| Admission, Multiple contexts included | `cov/admission.rs` | 463 |
| `CovPolicy`, validation, counts (atomics stay hosted) | `cov/policy.rs:1-304` | ~290 |
| `CovTimeRemaining`, `remaining_lifetime` | `cov/lifetime.rs:1-61` | 61 |
| Active lists | `cov/active.rs:1-332`, `cov/active/multiple.rs:1-206` | ~530 |
| SubscribeCOV and SubscribeCOVProperty checks | `handlers/cov.rs:1-247` | ~245 |
| **Total** | | **~2,180** |

Split point: the active lists can go to PR 2. Prep commits come first: the
server's `HostClock`, `Ord` on `MacAddr` (`bacnet-types`) and `NpduAddress`
(`bacnet-encoding`), the `BTreeMap`. PR 1 also starts the `reporting` image
(§8.3), and `SERVICES` names SubscribeCOV only from PR 2, once the core
sends what it subscribes to.

### 7.2 PR 2: COV notifications and the facade

| Item | From | Lines |
|---|---|---|
| Samples, observations, completion order | `cov/sample.rs:1-110`, `observation.rs:1-76`, `observation_order.rs:1-109` | ~295 |
| Value preparation | `cov/prepare.rs:1-144`, `flags.rs`, `reported.rs`, `value_source.rs` | ~470 |
| Life Safety snapshots | `life_safety_cov.rs` | 150 |
| Flight decisions as state (§5.5); the `Notify`, RAII and send stay | `cov/confirmed.rs:1-446` | ~300 |
| Per-subscription decision and budget, out of the fanout | `server/cov_notifications.rs:13-76`, `:287-567` | ~250 |
| New: `report`, the `Reporter` for COV, its confirmations (open question 4), `ActiveCovOverlay` | | ~650 |
| **Total** | | **~1,465 moved, ~650 new** |

### 7.3 PR 3: intrinsic events, distribution, acknowledgement

| Item | From | Lines |
|---|---|---|
| Intrinsic commit | `server/event_notifications.rs:128-203` | 76 |
| After-write evaluation, the tick body | `event_notifications.rs:220-253`, `intrinsic_reporting_lifecycle.rs:26-65` | ~75 |
| Distribution (class, window, recipients, Event Log) | `event_notifications.rs:291-438`, less the send | ~110 |
| Profile and payload | `server/event_notification_profile.rs`, `event_notification_payload.rs:1-753` | ~850 |
| Timestamps, message text | `server/event_timestamp.rs:1-75`, `event_message_policy.rs` | ~85 |
| AcknowledgeAlarm and its notification | `handlers/alarm_event/acknowledge_alarm.rs`, `event_notifications.rs:442-458` | ~110 |
| GetEventInformation | `handlers/alarm_event/get_event_information.rs`, `event_information_page.rs:1-138` | ~550 |
| Fault pass | `fault_detection.rs:1-136` | 136 |
| **Total** | | **~1,990** |

### 7.4 PR 4: Event Enrollment

| Item | From | Lines |
|---|---|---|
| Algorithms | `event_enrollment/algorithms.rs` | 819 |
| Fault algorithms, reference, support | `event_enrollment/fault.rs`, `reference.rs`, `support.rs` | 534 |
| The pass | `event_enrollment/mod.rs:1-862`, non-test | ~770 |
| Commit and projection | `event_enrollment/commit.rs` | ~600 |
| Public wrappers, replaced by `event::enrollment` | `event_enrollment/api.rs` | -60 |
| **Total** | | **~2,720** |

Split point, since it passes 2,500: algorithms, fault algorithms,
reference and support first (~1,350, all pure); the pass and commit second.

### 7.5 What stays hosted

- Tasks: purge, intrinsic, fault and Event Enrollment (`lifecycle.rs:661-742`,
  `:789-795`), now staged calls on Tokio intervals; `committed_cov.rs`,
  `cov_fanout.rs`, `cov_notify_context.rs`.
- Sending: the send half of `cov_notifications.rs`,
  `cov_notifications/confirmed.rs`, `event_send.rs`,
  `notification_transactions.rs`, `notification_attempts.rs`,
  `event_delivery.rs`, `event_suppression.rs`.
- Addresses and receiving: `event_recipient_route.rs`,
  `event_awaited_recipients.rs`, the bindings, `event_forwarding*.rs`,
  `received_event_log.rs`, `server/clock.rs`; GetAlarmSummary and
  GetEnrollmentSummary; until open question 3, the Multiple path
  (`cov/timed*`, `cov/multiple_reads.rs`, `cov_notifications/multiple*.rs`).

### 7.6 Parity tests

As in phases 1 and 2: in-crate `bacnet-server` tests, in new submodules
`device_core_parity_tests/cov.rs` and `events.rs` (the parent file is at the
700-line cap), on the shared `TestTransport`, paused Tokio time, no sleeps.
One fixture database is built twice: a `BACnetServer` serves one, a
`Reporter` with the same identity and config the other, both with one fixed
`ClockReader` and the facade's `now` from the same paused clock. Each step
goes to both, and the test compares the notifications: unconfirmed APDUs
octet for octet, confirmed ones by their service parameters (the invoke IDs
are each host's), with destinations and order. It pins what differs by
design, as `DIFFER` does, and fails on any other drift or on a pin that
stops differing.

| PR | Corpus |
|---|---|
| 1 | Subscribe, renew, cancel, cancel of nothing; every row of §5.8; the caps; Active_COV_Subscriptions before and after an expiry |
| 2 | The initial notification; changes under and over COV_Increment; Status_Flags alone; a Table 13-1 trigger; a property subscription's own increment; a confirmed report acknowledged, refused and timed out (hold-off); a change in flight (revisit); Time_Remaining after time passes |
| 3 | OUT_OF_RANGE with Time_Delay 2 (silent at 1 s, sent at 2 s) and back with Time_Delay_Normal; a value that reverts inside the delay; Event_Enable cleared (committed, not sent); recipients by day, window and transition; no clock (sequence numbers); AcknowledgeAlarm good, wrong state, wrong timestamp, and its notification; GetEventInformation |
| 4 | Each algorithm both ways; a FAULT_OUT_OF_RANGE fault; the pass-count delay at two intervals |

Mutation check for PR 2: change the increment test's `>=` to `>`; the
core's tests, the server's increment tests and the parity row must fail.

### 7.7 Changelog and docs

Each PR adds a `changelog.d` fragment, **Breaking (Rust API):** with a
migration note for the moved public items (the table's methods,
`CovPolicy`, `handle_subscribe_cov*`, `handle_acknowledge_alarm`,
`evaluate_event_enrollments*`), and updates `docs/rust-api.md`,
`docs/architecture.md` and the core's README. No **Wire:** entry. The
notifications are Python-visible, so every PR runs the Python suite.

## 8. Footprint

### 8.1 Today and with a probe

The `objects` image (thumbv7m, Rust 1.99.0, the `firmware` profile, dev at
`c308eda25`) has 127,452 octets of `.text`, 15,108 of `.rodata` and
143,584 of flash: `bacnet-objects` 38,838, `bacnet-device-core` 23,648,
`bacnet-encoding` 20,534. Its budgets are 142,752 / 16,928 / 160,816.

For this design its scenario was extended by hand, measured and reverted.
The COV probe: a `Vec` table, the two subscribe services decoded and
admitted under the caps, a change test with the increment, an
UnconfirmedCOVNotification encoded. The events probe: `event-reporting` on,
a Notification Class in the database, AI-1's evaluate, tick and commit,
the class's priority and recipients, an event notification encoded, an
AcknowledgeAlarm applied.

| Build (thumbv7m) | `.text` | `.rodata` | flash | memmove fns |
|---|---|---|---|---|
| `objects` today | 127,452 | 15,108 | 143,584 | 2 |
| + COV probe | 133,536 (+6,084) | 15,724 | 150,284 (+6,700) | 2 |
| + COV and events probes | 150,188 (+22,736) | 17,216 | 168,428 (+24,844) | 3 |

The events probe's 16,652 octets split about 6,500 in `bacnet-objects`
(`event-reporting`, ~2,900 by #2215, and the Notification Class), 4,300 in
`bacnet-encoding` (every event-values arm links) and 3,750 in its logic. No
`core::fmt` or 64-bit division; one more memmove routine
(`__aeabi_memmove4`), so the `reporting` rows budget three.

### 8.2 Estimates

The probes are floors. They lack the policy and identity checks, the
flight state, the Table 13-1 rows, Value_Source, Life Safety, the active
lists, timestamps, payloads for five event types, GetEventInformation and
the facade. For those, the ratio `bacnet-device-core` shows today (23,648
octets of `.text` from about 3,300 non-test lines, comments included, so
about 7 a line), over the lines an image would link:

| `reporting` image (thumbv7m) | `.text` estimate | From |
|---|---|---|
| `cov` (after PR 2) | 145-153 KB | ~3,800 linked lines of PR 1 and 2 |
| + `event-reporting` (after PR 3) | 170-178 KB | ~2,000 lines, plus the measured objects and encoder share |
| + `event-enrollment` (after PR 4) | 205-218 KB | ~2,700 lines, the Event Enrollment object, `decode_event_parameter` |
| + the Multiple path (open question 3) | +15-25 KB | ~3,300 lines, the COVNotificationMultiple codec |

Flash runs about 18 KB above `.text`. So COV with intrinsic reporting fits
a 256 KB part with room; with Event Enrollment it is tight there and easy
at 512 KB. Each PR replaces its estimate with the measured row.

### 8.3 The `reporting` image

The `objects` image keeps its features off and its rows, the standing
check that reporting costs a non-reporting device nothing. PR 1 adds a
`reporting` image (`src/reporting.rs`, `src/bin/reporting.rs`,
`tests/reporting.rs`; feature `reporting = ["objects",
"bacnet-device-core/cov"]`) for thumbv7m, thumbv7em and riscv32imac, with
rows from `--suggest`, so every PR has a budget row: in PR 1 its scenario
subscribes, renews, cancels and reads Active_COV_Subscriptions through the
staged functions; PR 2 runs it through the `Reporter` and checks the
notifications it pops; PR 3 and PR 4 widen the feature and add a
Time_Delay, an acknowledgement and an enrollment.

Heap: a subscription entry is about 120 octets on a 32-bit target. The
scenario caps subscriptions at 8 and starts from the `objects` arena of 16
KiB, growing it only if the host test's peak says so; the outbox is bounded
by `ReportConfig::outbox`.

## 9. Risks

**Hot paths.** Every committed write runs `evaluate` and `after_write`:
pure moves with logic changes apart, a range lookup where the server scans,
no `dyn`. Before and after each PR, one at a time on an idle machine:
`stress-test -- cov`, `-- mixed --clients 5`, `bench bip_latency`, and
`bench encoding` as the control.

**Behaviour drift.** Deliberate, and visible to tests only: iteration
order (by object, where a `HashMap` was random), and the facade's exact
expiry against the server's 30 s purge. The parity corpus holds the rest;
the rewrites (§5.5, §6.2) get their own commits and mutation checks.

**Lock order.** Database, COV table, timed store, as documented
(`cov/timed_capture.rs:9`). The core takes borrows, not locks, so it can't
invert them, and the one new pairing (§4.5) follows the order. The observer
rule of device-core.md §4.5 still holds for COV capture: a leaf mutex after
the database, never an await.

**#1494.** No futures in the core; §4.5 lists the `select!`s that stay and
the one PR 2 rewrites.

**Time sources.** The monotonic `now` for lifetimes, hold-offs, ticks and
passes; the Device clock frame for timestamps and windows. Wall time passed
as `now` would expire subscriptions when the clock is set; the docs say so.

**Notification storms.** A burst of writes, a tick that fires many delayed
transitions, a fault that flips every object at once. Today's bounds stay:
64 notifications and 65,536 octets of COV per input, 16 confirmed reports
in flight per peer, one owed revisit instead of a second report, the
subscription caps. Firmware adds the outbox bound (open question 6).

**Size and scope.** §8; each feature is checked alone in CI. About 8,400
moved lines over four PRs, more if open question 3 brings Multiple in;
each PR names a split point.

## 10. Open questions

1. **Placement.** (a) Modules in `bacnet-device-core` behind features;
   (b) a sibling crate; (c) `bacnet-objects`. **Recommend (a)** (§3.1):
   one crate for firmware, and the error map, overlay and `Responder`
   shared as they are.
2. **Features.** (a) `cov`, `event-reporting`, `event-enrollment`, each kept
   only if its PR measures a real saving; (b) one `reporting` feature;
   (c) also one per COV form. **Recommend (a)**: COV-only and
   intrinsic-only devices are common, and Event Enrollment costs most.
3. **The Multiple form.** (a) Move it all in this issue (two more PRs,
   ~3,300 lines, untimed then timed); (b) its admission and active list in
   PR 1, the rest in a follow-up issue. **Recommend (b)**: the service is
   optional, its code the most intricate here, and firmware rarely needs
   it; the shared table keeps the server whole meanwhile.
4. **Confirmed notifications in firmware.** (a) The host brings its own
   transactions; (b) the facade carries a small table on `bacnet-time`
   (invoke IDs, a deadline and retry count per report, replies matched by
   invoke ID and peer; ~250 lines); (c) move the server's into the core.
   **Recommend (b)**, in PR 2: a device that takes a confirmed subscription
   should honour it, and (c) would pull in endpoint-core's coordinator.
5. **Recipient windows without a clock.** (a) Fail closed: only a
   destination valid every day, all day, matches; (b) ignore days and
   windows; (c) refuse event reporting without a clock. **Recommend (a)**,
   documented with Clause 12.21.8, which lets a device restrict
   Recipient_List entries to that always-valid form. The server is
   unaffected: it passes its system-UTC fallback.
6. **Outbox overflow in firmware.** (a) COV marks the subscription owed
   (one later notification, current values) and an event notification is
   dropped and counted; (b) refuse input until drained; (c) unbounded.
   **Recommend (a)**: (b) stalls the tick, and GetEventInformation lets a
   client recover a dropped event's state.
7. **Table collection.** (a) `BTreeMap` keyed object first, with `Ord` on
   `MacAddr` and `NpduAddress`; (b) hashbrown, as `bacnet-objects` uses
   without std. **Recommend (a)**: a range per changed object, a fixed
   order, and no new dependency.
8. **Event-state persistence.** (a) None, as today; (b) a save hook for
   Event_State, Acked_Transitions and timestamps. **Recommend (a)**, with
   (b) as its own issue on demand: restoring it raises questions (stale
   timestamps, which delays restart) that need a design.
9. **The image.** (a) A separate `reporting` image; (b) grow `objects`.
   **Recommend (a)**: the `objects` rows keep the non-reporting cost.

## 11. Decisions

None yet; the answers to §10 go here, as in device-core.md §10.

## 12. Evidence

Code read at dev `c308eda25`. Sizes (§8): the `objects` image built with
`--profile firmware --features objects` for thumbv7m, measured with
`scripts/ci/firmware_size.py`, the probes reverted. Standard: Clauses 12.12, 12.21, 13.1, 13.2 (13.2.2, 13.2.2.1.4, 13.2.3,
13.2.4, 13.2.5, 13.2.5.3), 13.3, 13.4, 13.5, 13.6, 13.7, 13.14, 13.15,
13.16; Tables 13-1, 13-1a, 13-1a-2, 13-3 and 13-6; paraphrased.
