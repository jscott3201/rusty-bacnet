# bacnet-device-core

A sans-IO BACnet request core for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet): it takes a
request APDU, runs it against a borrowed `bacnet-objects` `ObjectDatabase`
and writes the reply. It has no I/O, clock, locks or async; the host owns
the database guard, the transport, addressing and time.

- `no_std` with an allocator (the `alloc` feature), or hosted (`std`, the
  default). Like `bacnet-objects`, it needs atomic compare-and-swap, so it
  doesn't build for ARMv6-M.
- `structured-writes`, which `std` turns on, decodes writes of a
  BACnetEventParameter and of BACnetPortPermission and
  BACnetStageLimitValue array elements. With `alloc` alone it is off: those
  writes are refused with WRITE_ACCESS_DENIED, and a firmware image is
  about 20 KB of flash smaller. The metadata rows still call those
  properties writable, so list them as read-only in such a build's PICS.
- `event-reporting`, which `std` turns on, has intrinsic event reporting
  (`event`, #2303) and turns on `bacnet-objects`' feature of the same
  name. No firmware image turns it on yet.
- `cov-multiple`, which `std` turns on, has the SubscribeCOVPropertyMultiple
  path (#2320): the service with its checks
  (`cov::subscribe_cov_property_multiple`), the capture of timestamped
  changes where a mutation commits (`cov::timed_capture`), the timestamped
  histories with their claims and deadlines (`cov::timed`), and what a
  fanout reports to each context, split to the subscriber's APDU size
  (`cov::multiple_report`). A host sends what those decide, and with the
  feature a `Reporter` answers the service and queues its reports as it
  does the other COV notifications. A firmware image leaves it off unless
  it serves the service: it adds about 134 KB of flash on a Cortex-M3.
- `bacnet-server` runs its ReadProperty, ReadPropertyMultiple,
  WriteProperty, WritePropertyMultiple, Who-Is and Who-Has on it, through
  the staged functions in `read`, `write` and `discovery`.
- Firmware hands each request to `responder::Responder::handle`, which
  answers those six services (and, given a clock, TimeSynchronization and
  UTCTimeSynchronization), and refuses every other confirmed request
  with a Reject, or with an Abort when it arrives segmented. An APDU that
  doesn't decode is dropped without an answer, as the server drops it.

## What it has

- `read`: `ReadScope` (the work limit and the wildcard targets),
  `ReadOverlay` (how a host serves an object differently from the stored
  one; `NoOverlay` for none), and the staged reads: plan a ReadProperty or
  ReadPropertyMultiple, see which server-owned Device lists the plan reads,
  then read it.
- `write`: WriteProperty and WritePropertyMultiple, with a
  `WriteCommitObserver` the host's audit and change capture run in under its
  guard; the application's own writes (`write::local::apply`: an input such
  as a sensor's Present_Value, a Reliability it works out, or a write made
  as the local Device or one of its objects, which `bacnet-server`'s
  `*_local` methods make through it);
  the array-index gate and value decoding every write path shares;
  the NULL-relinquish rule (`write::relinquish`); the local form of
  references naming this device; and the end of a Command's or Channel's
  run a host can't make.
- `discovery`: Who-Is and I-Am, Who-Has and I-Have.
- `cov`: the COV subscription table (`CovTable`), with SubscribeCOV and
  SubscribeCOVProperty (`subscribe_cov`, `subscribe_cov_property`), the
  admission caps of a `CovPolicy`, lifetimes on `bacnet-time`, and the
  confirmed reports' state as plain data: a report begun with
  `begin_confirmed` ends through `delivered`, or by itself at its deadline.
  `evaluate` decides what a change of an object reports, within an
  `EventBudget`, and hands each notification's parameters to the host;
  the Device's two subscription lists come from the table (`cov::active`).
  The host keeps the table under its own guard, passes `now` to every call,
  and after each change takes what it fenced or removed (`take_fenced`) and
  the counts (`take_counts`). `bacnet-server` runs its table and its
  ordinary and Single notifications on it.
- `report`: `Reporter`, the `Responder` with COV reporting for a host with
  nothing to await: it answers SubscribeCOV and SubscribeCOVProperty (and
  SubscribeCOVPropertyMultiple with `cov-multiple`),
  queues the notifications a request or the host's own change calls for in
  an outbox allocated once (`OutboxLimits`; one unconfirmed entry per
  subscription, the newest, measured from the one it replaces, so a change
  back ends current, #2508), hands them out through `pop`
  (a confirmed one under an invoke ID the host lends through `InvokeIds`;
  one that finds none is looked at again as IDs come back, its own or those
  the host reports with `invoke_ids_released` while `owes_invoke_id` holds,
  and goes once it can take one, #2512),
  matches each answer (`reply`), resends or gives up at the deadline
  (`on_deadline`), and answers the Device's subscription lists from its
  table (`cov::active::ActiveCovOverlay`). Build the database with
  `report::SERVICES`. It runs no DeleteObject (`handle` rejects it as an
  unrecognized service): a host that serves it takes the request before
  `handle`, lists the service in its own Protocol_Services_Supported, and,
  like a host that deletes an object on its own, calls
  `remove_for_object`. That drops the object's subscriptions, their queued
  and owed reports and any confirmed one still out, giving its invoke ID
  back; `pop` hands out nothing more for the object. The application's
  own changes go through `apply_local`, which makes one with
  `write::local::apply` and queues what it reports, or the typed
  `update`, `set_reliability`, `command` and `relinquish`, which take a
  `LocalValue` (#2446). A change that wrote nothing, or was refused,
  reports nothing. With `event-reporting` and `ReportConfig::with_sweep`
  it reports intrinsic events too (#2303): it evaluates each write and
  change, sweeps at the period and when a Time_Delay ends (no sooner than
  `SWEEP_FLOOR` after the last sweep), runs the optional fault pass,
  answers AcknowledgeAlarm and GetEventInformation (`EVENT_SERVICES`) and
  acknowledges locally (`acknowledge_local`). Event notifications go to
  Notification Class recipients (`Destination::Recipient`, a Device one
  resolved through `Bindings`) through the same outbox and confirmed
  table, ahead of the COV ones of the same change; one that finds the
  outbox full evicts the newest COV notification, whose subscription is
  owed a look. For each address no event notification is first sent before
  an older one: one refused an invoke ID waits in its place, every `pop`
  tries it first, and later ones to its address wait behind it, whatever
  their process identifier or confirmed flag (COV notifications don't). An
  older one's resend can still follow a newer one, each on its own invoke
  ID; the receiver orders them by their timestamps. A Device recipient
  rebound while one of its events waits keeps no order across the
  rebinding. It waits only
  while `InvokeIds` has none, with no cutoff of its own; held ones fill the
  outbox at worst, and new events are then dropped and counted. Under
  DeviceCommunicationControl keep popping and report `NotSent`. Without a
  sweep the path is off; the endpoint refuses one, and its deadlines use
  `on_cov_deadline` over a shared database.
- `event` (behind `event-reporting`, which `std` turns on): intrinsic
  event reporting as staged functions under the host's guard, from
  `bacnet-server` (#2303). `after_write` evaluates and commits one changed
  object's transition, `sweep` does so for every object once a pass and
  fires each Time_Delay that has ended by its `now`, `next_due` says when
  the next one after a given instant ends, and
  `distribute_event` builds the EventNotification, picks its recipients by
  day, window and transition and records it in the Event Logs; the host
  sends it. The timestamps (the Device clock's, or a sequence number), the
  committed message text and the Event Values are here, as are
  AcknowledgeAlarm (`handle_acknowledge_alarm`), GetEventInformation
  within a `GetEventInformationBudget`, and the fault pass (`fault_pass`).
  What a step would have logged comes back as a value. `bacnet-server`
  runs its intrinsic reporting on it, and so does the `Reporter`, which
  judges windows by its clock's `SyncedClock::window` (`RecipientWindow`)
  and, without one, sends only to destinations open every day, all day.
- `clock`: the Device's clock for firmware with none of its own (#2321).
  `SyncedClock` is plain data on the host's monotonic timeline: until
  set it counts from local midnight, 1990-01-01, as a device does after a
  restart, and recipient windows fail closed; a TimeSynchronization sets it
  to the local time given, a UTCTimeSynchronization to UTC less UTC_Offset
  (plus an hour in daylight-saving time), as the server's clock does.
  `DeviceClock` pairs it with the host's timer and is lent to the database,
  so the Device serves Local_Date and Local_Time from it.
  `Responder::with_clock` runs the two services the Device lists
  (`clock::SERVICES`), and `Responder::set_time` (or `Reporter::set_time`)
  takes a board's real-time clock. `destination_open` judges a Notification Class destination
  against the clock's window. With `event-reporting`, a `Responder` with no
  clock refuses a Recipient_List write holding a destination not open every
  day, all day, to 23:59:59.99 (PROPERTY / VALUE_OUT_OF_RANGE), on WriteProperty,
  WritePropertyMultiple and the `Reporter`'s local writes; a host running
  AddListElement checks with `write::check_clockless_recipients`.
- `reply`: the reply PDUs, and the one map from an error to its Reject or
  Error PDU.
- `responder`: `Responder`, which strings the steps together for a host
  with nothing to await between them, within `Limits`: the ReadProperty work
  limit and the ReadPropertyMultiple budget.

A host names a request's sender with `Inbound::with_source` (a
`bacnet-objects` `CommandOrigin`), and the `Responder` commands a
commandable Present_Value for it, as the server does. A request with no
source draws WRITE_ACCESS_DENIED from such an object. A Command's or
Channel's run that a write starts ends at once, unmade, with none of its
actions performed.
[The design note](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/device-core.md)
has the details.
