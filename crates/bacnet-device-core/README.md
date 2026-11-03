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
- `bacnet-server` runs its ReadProperty, ReadPropertyMultiple,
  WriteProperty, WritePropertyMultiple, Who-Is and Who-Has on it, through
  the staged functions in `read`, `write` and `discovery`.
- Firmware hands each request to `responder::Responder::handle`, which
  answers those six services, and refuses every other confirmed request
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
  guard; the array-index gate and value decoding every write path shares;
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
  nothing to await: it answers SubscribeCOV and SubscribeCOVProperty,
  queues the notifications a request or the host's own change calls for in
  an outbox allocated once (`OutboxLimits`; one unconfirmed entry per
  subscription, the newest), hands them out through `pop`
  (a confirmed one under an invoke ID the host lends through `InvokeIds`),
  matches each answer (`reply`), resends or gives up at the deadline
  (`on_deadline`), and answers the Device's subscription lists from its
  table (`cov::active::ActiveCovOverlay`). Build the database with
  `report::SERVICES`.
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
