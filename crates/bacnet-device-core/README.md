# bacnet-device-core

A sans-IO BACnet request core for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet): it takes a
request APDU, runs it against a borrowed `bacnet-objects` `ObjectDatabase`
and writes the reply. It has no I/O, clock, locks or async; the host owns
the database guard, the transport, addressing and time.

- `no_std` with an allocator (the `alloc` feature), or hosted (`std`, the
  default). Like `bacnet-objects`, it needs atomic compare-and-swap, so it
  doesn't build for ARMv6-M.
- `bacnet-server` runs its ReadProperty, WriteProperty, Who-Is and Who-Has
  on it, through the staged functions in `read`, `write` and `discovery`.
- Firmware hands each request to `responder::Responder::handle`, which
  answers ReadProperty, WriteProperty, Who-Is and Who-Has, and refuses
  every other confirmed request with a Reject, or with an Abort when it
  arrives segmented. An APDU that doesn't decode is dropped without an
  answer, as the server drops it.

## What it has

- `read`: `ReadScope` (the work limit and the wildcard targets),
  `ReadOverlay` (how a host serves an object differently from the stored
  one; `NoOverlay` for none), and the staged reads: plan a ReadProperty or
  ReadPropertyMultiple, see which server-owned Device lists the plan reads,
  then read it.
- `write`: WriteProperty, with a `WriteCommitObserver` the host's audit and
  change capture run in under its guard; the array-index gate and value
  decoding every write path shares; the NULL-relinquish rule
  (`write::relinquish`); the local form of references naming this device;
  and the end of a Command's or Channel's run a host can't make.
- `discovery`: Who-Is and I-Am, Who-Has and I-Have.
- `reply`: the reply PDUs, and the one map from an error to its Reject or
  Error PDU.
- `responder`: `Responder`, which strings the steps together for a host
  with nothing to await between them.

The `Responder` has no requester's address, so it writes with no command
origin: an object with a commandable Present_Value refuses that write,
and a Command's or Channel's run that a write starts ends at once, unmade.
ReadPropertyMultiple and WritePropertyMultiple through the `Responder`
follow ([#2178](https://github.com/jscott3201/rusty-bacnet/issues/2178)).
[The design note](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/design/device-core.md)
has the details.
