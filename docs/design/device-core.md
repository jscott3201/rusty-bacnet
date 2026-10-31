# Design: a sans-IO device core that serves an ObjectDatabase (#2178)

Status: **approved** by the maintainer on 2026-10-30; the crate name and
§10 are decided. Milestone 0.16.0. Line references are to dev at
`294dc32e9`. BACnet references are clause numbers in ANSI/ASHRAE 135-2020,
paraphrased.

## 1. Summary

`bacnet-objects` builds without std (#1615, [objects-no-std.md](objects-no-std.md)),
but nothing portable can answer a request from an `ObjectDatabase`:
`bacnet-server` is Tokio throughout, `bacnet-endpoint-core` depends on Tokio,
and the `device` firmware image answers from a hand-written Device and one
Analog Input (`firmware/link/src/device/app.rs`).

This design adds `bacnet-device-core`: a synchronous no_std crate (with
`alloc`) that takes a request APDU, decodes it with `bacnet-services`, runs
it against a borrowed `ObjectDatabase` and writes the reply. The server's
read and write planning moves into it, and the server runs on the moved
code, as the Tokio transports and router run on the B/IP, MS/TP and router
cores.

## 2. Goals and non-goals

**Goals**

- One implementation of ReadProperty, WriteProperty, Who-Is/I-Am,
  Who-Has/I-Have, then ReadPropertyMultiple and WritePropertyMultiple, that
  both a firmware image and `BACnetServer` run.
- No I/O, no clock, no locks, no async: the host owns the database guard,
  the transport, addressing and time.
- Byte-for-byte the replies the server sends today, proven by a parity test
  over the server's shared `TestTransport`.
- A `firmware/link` `objects` image (Device, 2 Analog Input, 2 Binary Value)
  with size-budget rows checked by `check-firmware-link.sh`.
- No measurable regression on the server's ReadProperty path, its hottest.

**Non-goals (v1)**

- COV, intrinsic events and Event Enrollment: phase 3, a separate design on
  `bacnet-time`. The core never reads or writes the COV table.
- Rate limiting, duplicate suppression and the directed-or-broadcast choice
  for I-Am and I-Have: they stay in `server/discovery.rs` and
  `server/requests/unconfirmed.rs`.
- Segmentation either way: the core answers with an Abort (§5), and the
  server keeps segmenting its own oversize ComplexACKs (`issue.rs:78-85`).
- Authorization, durable-write staging, Audit admission, DCC,
  ReinitializeDevice, time sync, Create/DeleteObject, ReadRange, the list
  and file services: hosted. The core has hooks (§4.5), not policies.
- Command and Channel runs: a write that queues one ends it unmade, as the
  bare handlers do now (`command_lists.rs:296`).
- A heap-free database (#1771).

## 3. Crate placement

### 3.1 Layer and dependencies

```
Layer 5:   bacnet-objects        object model, ObjectDatabase      (unchanged)
Layer 5.5: bacnet-device-core    sans-IO request core              (new)
             reply     PDU construction and the error map      (§5)
             read      ReadScope, ReadOverlay, plans, RP, RPM  (§4.3, §4.4)
             write     WriteCommitObserver, WP, WPM            (§4.5)
             discovery Who-Is/I-Am, Who-Has/I-Have             (§4.6)
             responder Responder: classify, read, write, handle (§4.2)
Layer 6:   bacnet-server, bacnet-endpoint                          (run on it)
```

Dependencies: `bacnet-types`, `bacnet-encoding`, `bacnet-services`,
`bacnet-objects` and `bytes`; nothing else. No `tracing`: the one `debug!`
line in the code it takes over (`command_lists.rs:298`, in `end_unmade`) is
dropped; `:261` stays with the server's ownerless-run sweep.

It is published, since the published `bacnet-server` depends on it, and
joins `default-members`.

### 3.2 Features and targets

| Feature | Turns on | For |
|---|---|---|
| `std` (default) | `std` of types, encoding, services, objects, bytes | hosted builds; compiles none of hashbrown, spinning_top or libm |
| `alloc` | `alloc` of types, encoding, services, objects | bare metal with a global allocator |

As in `bacnet-objects` (`lib.rs:11-18`), a build with neither feature stops
with a `compile_error!`, and so does a target without pointer-width
compare-and-swap (CAS), which `bacnet-objects` needs for `alloc::sync::Arc`.
So the core builds for thumbv7m, thumbv7em and riscv32imac, and not for
thumbv6m (ARMv6-M has no CAS) or riscv32imc (no atomics). The
allocation-free path stays the `device` image (#1614, #1771).

Like `bacnet-objects` (`lib.rs:4-9`), the crate is `no_std` without `std`
and declares `#[macro_use] extern crate alloc` for `format!` and `vec!`. The
moved code has a few std-only spellings, which a prep commit in
`bacnet-server` rewrites before each move, so the move commits stay pure:
`std::borrow::Cow` (`local_references.rs:29`) becomes `alloc::borrow::Cow`;
the `HashSet` of required properties (`handlers/rpm_budget.rs:71`, from
`handlers/mod.rs:6`) becomes `alloc::collections::BTreeSet`, used only for
membership; and `format!` (`server/rpm_budget.rs:39`) and `to_string`
(`handlers/object_mgmt.rs:26`) take the `alloc` imports (the prep commit
adds `extern crate alloc` to the server for this). These and the renames
in §6.5 are the only edits to moved code; logic changes get commits of
their own (§9).

### 3.3 CI scripts

- `scripts/ci/check-no-std.sh`: add `"bacnet-device-core alloc"` to
  `checks` and `bacnet-device-core` to `cas_only`, so ARMv6-M is passed over
  as it is for `bacnet-objects`. The existing graph checks then hold it to no
  std feature, no `tracing`/`tokio`/`socket2`, and no atomics backend.
- `scripts/ci/check-firmware-link.sh`: every image builds on every one of
  its `default_targets` (thumbv7m, thumbv6m, riscv32imc, thumbv7em). The
  `objects` image can't build on thumbv6m or riscv32imc, and riscv32imac
  isn't in the list. PR 1 adds a per-image target filter (a `case` on the
  image name, since the script stays bash 3.2 compatible): `objects` builds
  on thumbv7m, thumbv7em and riscv32imac, and riscv32imac builds only
  `objects`. Its feature set is a third entry in `builds`.
- `check-default-features.sh` picks the crate up as a published member.

## 4. API

### 4.1 Two layers

The server awaits between the steps of one request (to sample the live
Device lists, stage a durable write, ask a policy); the core can't. So the
core has **staged functions** the server calls between its awaits (plan a
read, read a plan, write a property, run a WPM with hooks), and a
**`Responder`** that strings them together for a host with nothing to
await: firmware and the parity test. Both borrow the database; the core
never locks, so the lock order stays the host's.

### 4.2 Responder

```rust
/// What a request needs from the database. Read: RP, Who-Is, Who-Has (and
/// RPM from PR 3). Write: WP (and WPM). None: answered without the
/// database (Reject, Abort), or not this core's.
pub enum Access { Read, Write, None }

pub struct Inbound {
    pub apdu: Bytes,     // the whole APDU, header included
    pub max_reply: u16,  // what the requester and route accept; the host works
                         // it out as endpoint_responder.rs:378-380 does today
}

/// Reply: a confirmed reply is in `out`. Announce: an I-Am or I-Have is in
/// `out`, for the host to address and rate-limit. Silent: nothing to send.
/// Unhandled: not a service this core runs.
pub enum Outcome { Reply, Announce, Silent, Unhandled }

/// `objects`: what a write changed, for the host's post-write work.
pub struct Written { pub outcome: Outcome, pub objects: Vec<ObjectIdentifier> }

/// `read_work`: rows one ReadProperty may expand (a Group's Present_Value).
/// PR 3 adds the RPM limits.
pub struct Limits { pub read_work: usize }

pub struct Responder { identity: DeviceIdentity, scope: ReadScope }

impl Responder {
    /// Refuses an identity that claims segmentation (the core segments
    /// neither way) and a `read_work` of zero.
    pub fn new(identity: DeviceIdentity, limits: Limits) -> Result<Self, Error>;
    pub fn with_registered_port(self, port: ObjectIdentifier) -> Self;
    /// PDU type and service choice only; never decodes the service.
    pub fn classify(apdu: &[u8]) -> Access;
    pub fn read<O: ReadOverlay>(&self, db: &ObjectDatabase, overlay: Option<&O>,
        request: Inbound, out: &mut BytesMut) -> Outcome;
    pub fn write(&self, db: &mut ObjectDatabase,
        observer: Option<&mut dyn WriteCommitObserver>,
        request: Inbound, out: &mut BytesMut) -> Written;
    /// Firmware convenience: classify, then `read` or `write`.
    pub fn handle(&self, db: &mut ObjectDatabase, request: Inbound,
        out: &mut BytesMut) -> Written;
}
```

`classify` exists for a host with a reader-writer lock: it picks the guard
before calling `read` or `write`. Firmware that owns its database calls
`handle`. `Inbound`, `Limits` and `Written` are `#[non_exhaustive]`;
`Inbound::new`, `Limits::new` and `Written::from(Outcome)` construct them.

`write` passes no command origin (as built, PR 2): `Inbound` carries no
source address, so the core can't name the requester a command came from
(Clause 19.5). An object whose Present_Value is commandable refuses such
a write with WRITE_ACCESS_DENIED, as it refused the bare handler's, where
the server commands it for the requester; a writable Present_Value and
every other writable property take the write. The parity test pins the
difference (§7.2). A host that knows its requester and needs commands
through `write` is a follow-up: `Inbound` would carry the origin. The `Responder`'s `ReadScope` (§4.3) takes its work limit from
`Limits` and its Device from the identity, as the server's does. `Inbound`
carries no source address and no time: whether a confirmed request that
arrived as a group (link-layer broadcast or multicast) is answered at all is
the host's call, made before the core sees the APDU; the endpoint responder
drops one on `received.is_group` (`endpoint_responder.rs:295-297`).

**The APDU is owned `Bytes` (lead decision, 2026-10-30).** `decode_apdu`
takes `Bytes` (`bacnet-encoding/src/apdu/mod.rs:539`) and there is no
borrowed decoder. The server already holds each received APDU as `Bytes`,
so it copies nothing; firmware makes one `Bytes` from its receive buffer
per request, an allocation the `alloc` profile already accepts. `classify`
keeps `&[u8]` and reads only the header octets (PDU type, segmented flag,
service choice); it decodes nothing. A borrowed decoder in
`bacnet-encoding` is out of scope, and a follow-up only if the `objects`
image's heap or cycle figures call for one.

**Requests the core doesn't run (lead decision, 2026-10-30).** `read` and
`write` return `Outcome::Unhandled` for a confirmed service they don't run,
so a host routes it on: the server goes on to ReadRange, the file services
and the rest. `handle`, for firmware, answers every confirmed request
itself, per §5: Reject UNRECOGNIZED_SERVICE for a service the core doesn't
run, Abort SEGMENTATION_NOT_SUPPORTED for a segmented request. An
unconfirmed service it doesn't run is `Silent`. It does so through `reply`
helpers that a host using `classify` can call on `Access::None` too:

```rust
pub fn refuse(apdu: &[u8], out: &mut BytesMut) -> Outcome;  // picks one of the two below,
                                                            // or Silent for unconfirmed
pub fn unrecognized_service(invoke_id: u8) -> Apdu;         // Reject UNRECOGNIZED_SERVICE
pub fn refuse_segmented(invoke_id: u8) -> Apdu;             // Abort SEGMENTATION_NOT_SUPPORTED
pub fn fit(reply: Apdu, max_reply: u16) -> Apdu;            // oversize ComplexACK -> Abort
```

### 4.3 ReadScope and ReadOverlay

Today every read takes `Option<&DeviceReadContext>` (`device_view.rs:67-79`),
which mixes plain data with the server's executor view. The core splits
the two.

```rust
/// Plain per-request read settings.
#[derive(Clone, Copy)]
pub struct ReadScope {
    /// Result rows a single-property read may expand.
    pub work_limit: usize,
    /// The port a wildcard Network_Port names, when one is registered.
    pub registered_port: Option<ObjectIdentifier>,
    /// The executor's fixed Device, for the wildcard Device instance.
    pub device: Option<ObjectIdentifier>,
}

/// How a host serves a stored object differently from the object itself.
pub trait ReadOverlay {
    type View<'a>: BACnetObject where Self: 'a;
    /// The served form of `stored`; `None` serves `stored` as it is.
    fn view<'a>(&'a self, stored: &'a dyn BACnetObject) -> Option<Self::View<'a>>;
}

/// No overlay: every object answers for itself. Its `View` is uninhabited.
pub struct NoOverlay;
```

Why each piece exists, by the server code that needs it:

| Piece | Server code |
|---|---|
| `work_limit` | A Group's Present_Value charges its member rows to the request (#1172, `handlers/group_present_value.rs:57`); the endpoint responder sets its own limit (#1215, `endpoint_responder.rs:122-125`) |
| `registered_port` | the Network_Port wildcard resolves only to the registered port (`handlers/read_property.rs:208-212`) |
| `device` | the Device wildcard resolves to the executor's fixed Device, even when its object is gone (`handlers/read_property.rs:213-219`) |
| `view` | the server serves Protocol_Services_Supported, Property_List, the two COV lists, Device_Address_Binding and the identity properties from the executor, not the Device object (`device_view.rs:199-275`), with a full-server or endpoint profile (`device_view.rs:32-64`) |

`view` runs once per result row (`handlers/rpm_budget.rs:194`, `:253`,
`group_present_value.rs:124`, `read_property.rs:36`, `:143`), and today's
`DeviceReadContext::object` returns a borrowed wrapper
(`device_view.rs:137-142`). Through `&dyn` that would need a box per row,
so the read functions are generic over `O: ReadOverlay` (decision 1, §10).
The server implements the trait for `DeviceReadContext`, which keeps
`DeviceReadView`, the profile, clock cache, live lists and identity values
and gives `registered_port`, `work_limit` and `device_oid()` to `ReadScope`.
Firmware passes `None::<&NoOverlay>`: a Device built with
`DeviceIdentity::with_services(device_core::SERVICES)` declares its own
identity and services.

### 4.4 Staged reads for live-list sampling (#1213)

A read is planned before any value is read, so the request samples the
server-owned Device lists once, and only those its plan reads. The server
does this for ReadProperty (`server/requests/confirmed_response.rs:166-191`:
plan at 166, sample at 174-182, read at 184-191), ReadPropertyMultiple
(`:111-123`) and `read_local` (`server/handles.rs:94-121`). The core keeps
the steps apart:

```rust
pub enum ReadFailure { Service(Error), Work, Bytes }  // handlers/read_budget.rs:8

/// `LiveCovSelection` (cov/active.rs:160), moved: which of the selected
/// Device's server-owned lists a plan reads.
pub struct LiveLists { pub device: ObjectIdentifier, pub active_cov: bool,
    pub active_cov_multiple: bool, pub address_bindings: bool }

pub fn resolve_target(db: &ObjectDatabase, scope: &ReadScope,
    requested: ObjectIdentifier) -> ObjectIdentifier;
pub fn plan_property<O: ReadOverlay>(db: &ObjectDatabase, scope: &ReadScope,
    overlay: Option<&O>, target: ObjectIdentifier, property: PropertyIdentifier,
    index: Option<u32>) -> Result<PropertyPlan, ReadFailure>;
impl PropertyPlan {
    pub fn live_lists(&self, db: &ObjectDatabase) -> Option<LiveLists>;
}
pub fn read_value<O: ReadOverlay>(db: &ObjectDatabase, overlay: Option<&O>,
    plan: PropertyPlan) -> Result<PropertyValue, Error>;
pub fn read_property_ack<O: ReadOverlay>(db: &ObjectDatabase, overlay: Option<&O>,
    request: &ReadPropertyRequest, plan: PropertyPlan, out: &mut BytesMut,
    completed: impl FnMut(ObjectIdentifier, &ReadPropertyRequest, &Result<(), Error>),
) -> Result<(), Error>;

// struct at handlers/rpm_budget.rs:142, `new` at :147, `read` (today `read_observed`) at :169.
impl RpmPlan {
    pub fn new<O: ReadOverlay>(db: &ObjectDatabase, scope: &ReadScope, overlay: Option<&O>,
        request: &ReadPropertyMultipleRequest, limit: usize) -> Result<Self, ReadFailure>;
    pub fn live_lists(&self, db: &ObjectDatabase) -> Option<LiveLists>;
    pub fn read<O: ReadOverlay>(self, db: &ObjectDatabase, overlay: Option<&O>,
        out: &mut BytesMut, max_service_ack: usize, completed: impl FnMut(..))
        -> Result<(), ReadFailure>;
}
```

A plan borrows nothing from the overlay, so the server plans with its
context, samples, then reads with `context.with_live(..)`, a context of a
shorter lifetime, as it does now (`device_view.rs:98-111`). The `completed`
callbacks carry the read audit intents (`server/requests/read_property.rs:118-137`,
`server/requests/mod.rs:225-227`); they see outcomes, never values.

The server's ReadProperty after PR 1, in outline:

```rust
let db = services.db.read().await;
let plan = read::plan_property(&db, &scope, Some(&context), target, property, index)?;
let live = match plan.live_lists(&db) {
    Some(lists) => Some(Box::pin(active_cov_snapshot(&db, tables, lists)).await),
    None => None,
};
let context = context.with_live(live.as_ref());
read::read_property_ack(&db, Some(&context), &request, plan, &mut ack, observe)
```

### 4.5 Writes and WriteCommitObserver

```rust
/// Moved from `handlers/write_property.rs:13-35`, unchanged.
pub trait WriteCommitObserver: Send {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>);
    fn commit_policy(&mut self, db: &mut ObjectDatabase, write: WriteTarget<'_>,
        value: &PropertyValue) -> Option<Result<(), Error>> { None }
    fn committed(&mut self, db: &mut ObjectDatabase);
    fn failed(&mut self, db: &mut ObjectDatabase, error: &Error);
    fn applied(&mut self, db: &ObjectDatabase, oid: ObjectIdentifier) {}
}

pub fn write_property(db: &mut ObjectDatabase, request: &[u8],
    observer: Option<&mut dyn WriteCommitObserver>,
    source: Option<&AuditWriteSource>, origin: Option<&CommandOrigin>,
) -> Result<(ObjectIdentifier, Applied), Error>;            // PR 2

pub fn write_property_multiple(db: &mut ObjectDatabase, request: &[u8],
    authorize: Option<&dyn Fn(&WritePropertyAttempt) -> Result<(), Error>>,
    observer: Option<&mut dyn WriteCommitObserver>,
    source: Option<&AuditWriteSource>, origin: Option<&CommandOrigin>,
) -> WpmOutcome;                                            // PR 3

/// Ends the runs Present_Value writes queued on Command and Channel objects
/// among `written`, unmade (`command_lists.rs:296`).
pub fn end_unmade(db: &mut ObjectDatabase, written: &[ObjectIdentifier]);
```

| Hook | Server code that needs it |
|---|---|
| `before`/`committed`/`failed`/`applied` | the Audit Reporter's records (`server/audit_reporter_write.rs:42`) and COV's per-attempt capture (`cov/timed_capture.rs:261`) |
| `commit_policy` | a sealed policy assignment admitted through Audit (`handlers/write_property.rs:15-24`) |
| `source` | the Audit_Notification_Recipient write's provenance (`handlers/write_property.rs:691-707`) |
| `origin` | writes a Command or Schedule makes on a local object (`command_lists/unattached.rs:100-111`, `server/requests/mutations.rs:228`) |
| `authorize` | WPM attempts decided ahead, without the guard (#2032, `server/requests/mutations.rs:307-329`) |

The observer is synchronous and runs under the host's database guard. Its
implementations must take no lock; the server's take none today, and the
COV capture they run was read from the COV table before the write, under
the database write guard (`server/requests/mutations.rs:316-319`).

WPM's Life Safety snapshots need no hook of their own:
`capture_before_write` runs just before `commit_attempt`
(`handlers/write_property.rs:232`, `:240`), whose first act is
`observer.before` on the same object (`:278-280`). PR 3 moves the capture
into the server's observer and drops the `snapshots` parameter.

### 4.6 Discovery

```rust
pub fn who_is(identity: &DeviceIdentity, db: &ObjectDatabase, request: &[u8])
    -> Option<IAmRequest>;  // unconfirmed.rs:92-109, discovery.rs:811, local_device.rs:39
pub fn who_has(db: &ObjectDatabase, request: &[u8], device: ObjectIdentifier)
    -> Result<Option<IHaveRequest>, Error>;  // handlers/object_mgmt.rs:8-43
```

The server keeps the limiter (`server/discovery.rs:419` onward: per-source
and global buckets, the negative Who-Has cache, the reply admission) and
the choice between a directed and a broadcast reply
(`unconfirmed.rs:124-157`, `:221-258`).

## 5. Error map

| Condition | Reply | Clause | Today |
|---|---|---|---|
| Service parameters don't decode | Reject, reason from the decoder | 18.9, 20.1.8 | `read_property.rs:91`, `write_property.rs:675`, `confirmed_response.rs:152-153` |
| Confirmed service the core doesn't run | Reject UNRECOGNIZED_SERVICE | 18.9 | `endpoint_responder.rs:369-373`, `server/requests/mod.rs:557-560` |
| Object, property, index or value refused | Error PDU, in the Clause 21 production for services that have one | 20.1.7, 21 | `confirmed_response.rs:233-303` |
| WPM syntax fault after an attempt succeeded | Error SERVICES / INVALID_TAG, naming the attempt | 20.1.8, 15.10 | `write_property.rs:143-165` |
| Read past the work limit (rows) | Abort OUT_OF_RESOURCES | 18.10 | `confirmed_response.rs:212-218`, `mod.rs:243` |
| RPM past its byte limit | Abort BUFFER_OVERFLOW | 18.10 | `mod.rs:244` |
| Segmented request (core v1) | Abort SEGMENTATION_NOT_SUPPORTED | 18.10 | `endpoint_responder.rs:310-315` |
| ComplexACK over `max_reply` (core v1) | Abort SEGMENTATION_NOT_SUPPORTED | 18.10, 5.4.5.3 | `endpoint_responder.rs:378-391` |
| Unconfirmed request doesn't decode, or is out of range | nothing | 18.9 (only confirmed requests can be refused) | `unconfirmed.rs:92-98`, `:173-179` |

**No Reject once execution has started (Clause 20.1.8).** Clause 20.1.8
confines Reject to the window between checking a request's syntax and
running it. When a fault could be reported either way, the device may pick,
unless running the service has already changed something visible on the
network; then the answer must be an Error. The core keeps today's line:

- Each service decodes its whole request first and turns a decode failure
  into a Reject with `Error::into_request_reject` (`bacnet-types`
  `error.rs:558`). A value that won't decode once the service is running,
  in a WriteProperty say, is an Error.
- WPM remembers whether an attempt has succeeded (`write_property.rs:143-147`);
  a syntax fault after that is an Error naming the attempt.
- The work limit fails in planning, before any value is read. RPM's byte
  limit fails after the reads, and RPM writes nothing. Both are Aborts, as
  today.

`error_apdu_from_error` (as `reply::error_apdu`) and `error_fields`
(`confirmed_response.rs:233-311`) move to the core's `reply` module whole,
the special productions for the
list, CreateObject and SubscribeCOVPropertyMultiple services included, so
the hosted services keep using the one map. The segmented-request and
oversize-reply Aborts become `reply::refuse_segmented` and `reply::fit`,
which the endpoint responder calls in place of its own copies.

## 6. What moves, and what stays

Moved means moved: the server imports the core's item, no `pub use` alias
is left behind, and every in-repository caller, test and doc changes in the
same PR (AGENTS.md, pre-1.0 rules). Line counts include doc comments.

### 6.1 PR 1, read path

| Item | From | Lines |
|---|---|---|
| `PropertyPlan`, `plan_read_property`, `read_property_value`, `read_property_request_observed`, `read_property_decoded`, `read_target_object`, `resolve_read_target`, `rpm_response_index` | `handlers/read_property.rs:8-221`, `:270-289` | ~235 |
| `ReadFailure`, `Work` | `handlers/read_budget.rs` | 47 |
| `PlannedObject`, `PlannedRow`, `expand`, `plan_specs`, `live_cov_selection`, `RpmPlan`, `read_row`, `element`, `Scratch` (Group planning needs `plan_specs`) | `handlers/rpm_budget.rs:1-373` | ~370 |
| Group Present_Value | `handlers/group_present_value.rs` | 170 |
| `ReadPropertyMultipleBudget`, its `Default` and `validate` (public; callers in `rusty-bacnet` `server_methods/constructor_budgets.rs` and `bacnet-endpoint` `session.rs:202`). The builder setters (`:46-69`) and the inline test module (`:71-133`: `#[tokio::test]`, `TestTransport`, `ServerConfig` checks) stay in `bacnet-server` as tests of the config wiring, imports retargeted | `server/rpm_budget.rs:1-44` | 44 |
| `LiveCovSelection`, as `LiveLists` | `cov/active.rs:157-177` | 21 |
| `handle_who_has` | `handlers/object_mgmt.rs:5-43` | 39 |
| `iam_request_for`, `announcement_device` | `server/discovery.rs:809-818`, `local_device.rs:37-47` | 21 |
| `error_apdu_from_error`, `error_fields` | `server/requests/confirmed_response.rs:222-311` | 90 |
| **Total** | | **~1,040** |

Deleted: `handlers::handle_read_property` (`read_property.rs:86-102`), whose
40-odd test callers switch to the core. The legacy unbounded
`handle_read_property_multiple` (`read_property.rs:291-420`) and its
`expand_property_reference` stay until PR 3.

### 6.2 PR 2, write path

| Item | From | Lines |
|---|---|---|
| `WriteTarget`, `WriteCommitObserver`, `Applied`, `check_and_prepare_name_write`, `commit_attempt`, `check_write_array_index`, `gate_and_decode_write`, `decode_write_property_value`, `decode_structured_array`, `handle_write_property_observed`, `write_with_source` | `handlers/write_property.rs` (all but WPM) | ~560 |
| NULL-relinquish rule | `handlers/relinquish.rs` | 153 |
| `localize`, `localize_member`, `DeviceQualified` (+ its tests) | `local_references.rs` | 225 (+300) |
| `check_executor_owned_write`, `owned`, `identity_property` | `device_view.rs:150-172`, `device_view_identity.rs:14-24` | ~35 |
| `Unfinished`, `take_queued`, `end_unmade` | `command_lists.rs:120-179`, `:280-304` | ~85 |
| **Total** | | **~1,060** |

Callers that switch imports: `server/durable_writes.rs`, `write_group.rs`,
`handles.rs`, `local_writes.rs`, `requests/wpm_ahead.rs`,
`requests/endpoint_responder.rs`, `requests/mutations.rs`,
`command_runs.rs`, `command_lists/unattached.rs`, `schedule.rs`,
`handlers/list.rs`, `handlers/object_mgmt.rs`. Deleted:
`handlers::handle_write_property`.

As built: the items sit in `write` (`write/mod.rs`, with `decode`,
`executor`, `local_references` and `runs` re-exported from it, and
`write::relinquish` public as a module). `owned` and `identity_property`
are public, since the server's Device view still reads them;
`null_in_datatype` is crate-private, and its table test moved with it.
`Unfinished`'s fields stay private, so the runner's progress record became
`Unfinished::progress`, the body of the runner's own `Owner::progress`.
`WriteTarget` (the service's five coordinates) and `Applied` (written or
left unchanged) stay exhaustive: hosts build a `WriteTarget` for their own
commits and match `Applied` whole. The server's other callers also include
`cov/timed_capture.rs`, `audit_reporter.rs`, `request_tasks.rs`,
`command_lists.rs` and `command_lists/taken.rs`.

### 6.3 PR 3, RPM and WPM

| Item | From | Lines |
|---|---|---|
| `WritePropertyMultipleOutcome`, `handle_write_property_multiple_observed` with its `_detailed` and `_authorized` wrappers, `semantic_failure`, `wpm_undecodable_coordinate` | `handlers/write_property.rs:59-72`, `:102-259`, `:335-345`, `:404-416` | ~205 |
| Deleted: legacy `handle_read_property_multiple`, `expand_property_reference` | `handlers/read_property.rs:223-268`, `:291-420` | -175 |
| Deleted: `handle_write_property_multiple` | `handlers/write_property.rs:74-100` | -27 |

### 6.4 What stays hosted

`DeviceReadContext`, `DeviceReadView` and `DeviceExecution`
(`device_view.rs`); live-list sampling (`active_cov_snapshot`,
`address_bindings`, `LiveDeviceCov`); `confirmed_response.rs`'s
`read_property_response*`, `read_property_multiple_observed` and every send
function; the mutation pipeline (`requests/mutations.rs`: authorization,
`durable_writes::stage`, Life Safety capture, staging plans, `TakenRuns`,
COV capture, Schedule re-evaluation); command runs (`command_lists`,
`command_runs.rs`); the discovery limiter and reply routing; segmentation
(`issue.rs`); CreateObject and every other service.

### 6.5 Renames

The move renames these, and nothing else; every other item keeps its name.

| Today | In the core |
|---|---|
| `handlers::resolve_read_target(db, oid, port, device)` | `read::resolve_target(db, scope, oid)` |
| `handlers::plan_read_property` | `read::plan_property` |
| `handlers::read_property_value` | `read::read_value` |
| `handlers::read_property_request_observed` | `read::read_property_ack` |
| `PropertyPlan::live_cov`, `RpmPlan::live_cov` | `live_lists` |
| `RpmPlan::read_observed` | `RpmPlan::read` |
| `cov::active::LiveCovSelection` (`active`, `multiple`, `address_bindings`) | `read::LiveLists` (`active_cov`, `active_cov_multiple`, `address_bindings`) |
| `handlers::handle_who_has` | `discovery::who_has` |
| `discovery::iam_request_for(&ServerConfig)` (and its `discovery_iam_for_test` alias) | `discovery::i_am(&DeviceIdentity)`, which `discovery::who_is` uses |
| `confirmed_response::error_apdu_from_error` | `reply::error_apdu` |
| `handlers::handle_write_property_observed` | `write::write_property` |
| `handlers::handle_write_property_multiple_observed` and its `_detailed`, `_authorized` wrappers | `write::write_property_multiple` |
| `handlers::WritePropertyMultipleOutcome` | `write::WpmOutcome` |

## 7. Delivery

### 7.1 Three PRs

| PR | Adds | Moves | Tests | New lines |
|---|---|---|---|---|
| 1. Read path | crate, features, `reply`, `ReadScope`/`ReadOverlay`/`NoOverlay`, `Responder` for RP, Who-Is, Who-Has; `objects` image and three budget rows; script changes (§3.3) | §6.1, ~1,040 | core host tests for every error row of §5 that RP, Who-Is and Who-Has reach; the parity test (§7.2); the image's host test (`firmware/link/tests/objects.rs`); server tests changed only in imports | ~900 |
| 2. Write path | `Responder` WP; WP rows in the parity corpus; the image writes a Binary Value and reads it back | §6.2, ~1,060 | parity for success, WRITE_ACCESS_DENIED, a NULL left unchanged, duplicate Object_Name, an index on a non-array, a Command Present_Value write ending unmade | ~300 |
| 3. RPM/WPM | `Responder` RPM and WPM under `Limits`; Life Safety capture folded into the server's observer; the image reads with RPM | §6.3, ~205, and ~200 deleted | parity for ALL/REQUIRED/OPTIONAL, the work and byte limits, WPM with an applied prefix then a syntax fault (20.1.8); the legacy-vs-bounded RPM comparisons rewired to the core with an unlimited budget | ~300 |

Each PR adds a `changelog.d` fragment with a **Breaking (Rust API):**
entry (the deleted `handlers::handle_*` helpers, the moved
`ReadPropertyMultipleBudget`) and a migration note, and updates
`docs/rust-api.md` (lines 4138, 5427-5428, 6049 and 7076 name the moved
helpers), `docs/rpm-budget.md:50`, `docs/architecture.md:99` and the crate
layering in `docs/architecture.md`. Nothing Python-visible changes.

### 7.2 The parity test

An in-crate test in `bacnet-server`, on the shared
`crate::server::test_transport::TestTransport` (no second `TransportPort`
implementation, per CLAUDE.md):

1. Build one fixture database twice, the server's way (`DeviceIdentity`
   with the executed services): a Device, an Analog Input, a Binary Value,
   a Multi-state Value with a state-text array, and a Group of the Analog
   Input and the Binary Value. Start a `BACnetServer` on one; build a `Responder` with the same
   identity and limits on the other.
2. Deliver each request of the corpus to both, and compare the APDU of the
   frame `TestTransport` sends with the `Responder`'s `out`, byte for byte.
   Paused Tokio time, no sleeps.
3. The corpus: every property of every object, with and without an index;
   ALL/REQUIRED/OPTIONAL (PR 3); an unknown object and property, an index
   on a non-array, malformed parameters, an unknown service, a segmented
   request, a reply over `max_reply`; Who-Is in and out of range; Who-Has by
   identifier and by name.

The server serves executor-owned Device properties from its view
(`device_view.rs:150-159`, `device_view_identity.rs:14-24`), and a
`Responder` with `NoOverlay` from the Device object, so some of those rows
differ by design. PR 2 pins two write exchanges as well: a commandable
Present_Value write, which the server commands for the requester and the
core refuses for want of an origin (§4.2), and All_Writes_Successful after
a Command's Present_Value write, since the server makes the run's writes
and the core ends the run unmade. The Command's write itself, and its
In_Process afterwards, match. The test pins the exact set that differs: a drift
anywhere else, or a pinned row that stops differing, fails. Mutation check
for PR 1: drop the array-index gate from the core's `read_value`; the
core's tests, the server's array-index tests and the parity test must all
fail.

## 8. The `objects` firmware image

`firmware/link` gains an `objects` image: `src/objects.rs` (the scenario),
`src/bin/objects.rs` (the image), `tests/objects.rs` (the host run), and a
feature `objects = ["alloc", "dep:bacnet-objects", "dep:bacnet-device-core"]`.
The database is a Device from `DeviceIdentity` plus two Analog Input and two
Binary Value objects. The scenario, through `Responder::handle`: Who-Is in
range and out of it, Who-Has, ReadProperty of each Present_Value, an
unknown object (Error), malformed parameters (Reject), a segmented request
(Abort); PR 2 adds a write, PR 3 an RPM. PR 2's write is to a Binary
Value whose Present_Value is writable rather than commandable (§4.2), and
the image then links the write path: 33 to 41 KB more flash, most of it
the structured decoders the generic value decoding reaches.

Heap: on a 64-bit host, a counting allocator measured the database at
8,907 octets live and 9,075 at peak; a 32-bit target needs less. PR 1 gives
the image a 16 KiB arena and records its high-water mark under the manual
emulation check (`scripts/emu/`). Budget rows: `[thumbv7m-none-eabi.objects]`,
`[thumbv7em-none-eabihf.objects]` and `[riscv32imac-unknown-none-elf.objects]`,
from `--suggest` (size plus 12%), with the fmt, div64 and memmove counts
held at what the image links.

### 8.1 Baseline

From the 2026-10-30 research (thumbv7em, opt-level z, LTO, `.text`): a
database of 2 AI and 2 BV with one `read_property` call, about 70 KB; with a
Device, about 84 KB; all 64 constructible types, 468 KB. Dead-code
elimination drops the types nobody constructs; the fixed cost is the
trait's helper code (#2179).

Reproduced for this design with Rust 1.99.0 and the `firmware` profile's
settings (opt-level z, LTO, one codegen unit, abort on panic), the whole
APDU path included (`decode_apdu`, the ReadProperty decode, the read, the
ACK and `encode_apdu`), in octets:

| Image | Target | `.text` | `.rodata` | fmt fns | memmove fns |
|---|---|---|---|---|---|
| 2 AI + 2 BV, ReadProperty | thumbv7m | 76,032 | 7,156 | 0 | 2 |
| | thumbv7em | 75,264 | 7,156 | 0 | 2 |
| | riscv32imac | 91,952 | 7,916 | 0 | 2 |
| + Device (`build_database_with_extra`) + Who-Is | thumbv7m | 100,776 | 12,256 | 7 | 2 |
| | thumbv7em | 100,052 | 12,256 | 7 | 2 |
| | riscv32imac | 124,660 | 13,128 | 7 | 2 |

These sit above the research's figures because they include the APDU codec,
Who-Is and `DeviceIdentity`'s database builder. The Device brings
`core::fmt` in (seven functions, about 1.5 KB): `DeviceIdentity::new`
formats its default name (`device_identity/mod.rs:67`). The PR 1 image will
be larger again by the core's own code; these rows are the floor it is
measured against, and #2179 then reports its saving on the same rows.

## 9. Risks

**The server's hottest path.** Every ReadProperty goes through the moved
code. Mitigations:

- Pure moves: a commit that moves code changes no logic, and its diff
  reviews as moves; logic changes (the `ReadScope` split, the `LiveLists`
  rename, the Life Safety fold) are separate commits.
- No `dyn` on the hot path: the overlay is generic (§4.3), and the server
  instantiates it once, for `DeviceReadContext`.
- The future-size guard: `server/requests/read_property.rs:197-216` holds
  the spawned ReadProperty future to 1,032 octets (#1713). Nothing the core
  returns may be held across the sampling await. PR 1 reports the sizes the
  test prints, before and after.
- No new copy of the request: `Inbound.apdu` is the `Bytes` the server
  already holds, a reference-counted clone (§4.2). Firmware pays one
  allocation per request to make its `Bytes`; if the `objects` image's heap
  or cycle figures show that matters, a borrowed APDU decoder in
  `bacnet-encoding` is the follow-up.
- Benchmarks, before and after, on the same machine, sequentially, with no
  competing load, each with `--save-baseline` then `--baseline`:

  ```bash
  cargo bench -p bacnet-benchmarks --bench bip_latency     # RP, WP, RPM of 10 objects
  cargo bench -p bacnet-benchmarks --bench bip_throughput  # RP and WP throughput
  cargo bench -p bacnet-benchmarks --bench encoding        # control: untouched code
  cargo run --release -p bacnet-benchmarks --bin stress-test -- clients --steps 1,5,10,25,50 --duration 5
  cargo run --release -p bacnet-benchmarks --bin stress-test -- mixed --clients 5 --duration 5
  ```

  `bip_read_property_latency` is the gate for PR 1,
  `bip_write_property_latency` for PR 2 and `bip_rpm_10_objects_latency`
  for PR 3. A median change past the control's own spread is explained or
  fixed before merge.

**Lock order.** The server takes `ObjectDatabase` before `COVTable`. The
core takes no lock and never sees the COV table: live lists reach it as a
sampled overlay (§4.4), and the observer's COV capture was read before the
write (§4.5). Both traits are synchronous; an implementation that blocked
on a lock inside a core call would hold it under the database guard, and
review rejects one.

**#1494 (unpolled lock waiters).** The core adds no futures, `select!`,
`join` or `FuturesUnordered`. The host's sequence stays plan, await the
sample, read; turning the sample into a join with other work under the
database guard would reopen #1494's failure.

**#2179 (capability accessors).** `BACnetObject` has 101 methods, 69 of
them `_internal`. The core calls a few: `registered_port_internal`,
`device_authority_internal`, `audit_reporter_authority_internal`, and three
command-run hooks in the moved run-ending code (`take_command_run_internal`,
`command_lists.rs:287`; `record_command_write_internal`, `:174`;
`complete_command_run_internal`, `:176`). `command_generation_internal`
(`:270`, `:402`) stays with the server's runners. `DeviceReadView` forwards
about thirty more
(`device_view.rs:413-513`). The maintainer decided on 2026-10-30 that #2179
is in 0.16.0 and lands after PR 1, so PR 1 baselines `.text` on the
`objects` rows first, and #2179 reports its delta on them, editing the
core's call sites rather than the server's. PR 2 and PR 3 overlap it only
in the command-run hooks.

**Test churn.** About 40 server test files call the deleted helpers. They
stay in `bacnet-server` with only their imports changed, so "server tests
pass unchanged" stays meaningful; moving pure database tests into the core
is a follow-up. No file passes the 700-line cap: the two largest sources,
`confirmed_response.rs` (608) and `write_property.rs` (744), shrink.

## 10. Decisions

1. **Overlay shape: (a)** (lead, 2026-10-30). The options were (a) generic
   `O: ReadOverlay` with an associated view type, `DeviceReadView` staying
   in the server (§4.3); (b) an object-safe `&dyn ReadOverlay` handing the
   core values (services, live lists, identity), with the view and context
   code (about 380 lines, `device_view.rs:137-513`) and its hook forwards
   moved into the core; (c) `&dyn` returning boxed views, an allocation per
   result row on the hottest path. (a) is today's code, monomorphized once,
   and keeps executor policy hosted; firmware doesn't need the view, and (b)
   would move the forwards #2179 rewrites.
2. **Crate name: `bacnet-device-core`** (maintainer, 2026-10-30), over
   `bacnet-server-core`. It serves one device's objects, matches the
   firmware images' names, and doesn't suggest the server's COV,
   scheduling or segmentation.
3. **`core::fmt` in the `objects` image: (a)** (lead). Budget today's
   count, 7, with no headroom (as `apdu` does memmove); #2187 formats
   `DeviceIdentity::new`'s default name without `core::fmt`. The options
   not taken: fix it in PR 1, or let it float within the 12%. PR 1 stays a
   move, and the count can only fall.
4. **Which Device an I-Have names: (a)** (lead). Who-Is answers from the
   frozen identity (`unconfirmed.rs:101`, `announcement_device`), Who-Has
   from `db.selected_device()` (`unconfirmed.rs:192`). PR 1 keeps that, and
   #2186 makes I-Have and I-Am name the Device picked the same way. The
   option not taken: unify in PR 1, a **Wire:** change inside a move.
5. **Owned APDU** (lead, from the review): `Inbound.apdu` is `Bytes`, and
   `classify` peeks a `&[u8]` header; no borrowed decoder for now (§4.2).
6. **Requests the core doesn't run** (lead, from the review): `read` and
   `write` return `Unhandled` so a host routes the request on; `handle`
   answers every confirmed request through the `reply` helpers (§4.2).

## 11. Evidence

Code read at dev `294dc32e9`; trait counts from `bacnet-objects`
`traits.rs:143` onward. Sizes (§8.1): a scratch no_std binary outside the
workspace, on these crates by path with the workspace lock file, measured
with `llvm-size -A` and `firmware_size.py --suggest`. Heap (§8): a host
binary with a counting allocator, `bacnet-objects` on `alloc` alone.
Standard: Clauses 5.4.5.3, 18.9, 18.10, 20.1.7 and 20.1.8, paraphrased.
