---
title: "Python router"
description: "BACnetRouter and its ports and routes, generated from the type stub."
---

:::note[On `dev` after the release]
This page is generated from the type stub on the `dev` branch and tracks the next release, so it can list classes, methods and keywords merged after v0.15.0 that the released wheel doesn't have. The [v0.15.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) and the [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d) say what changed.
:::

Owned B/IP routing between numbered networks. The [Python reference](/rusty-bacnet/reference/python/) lists every group. Each page opens with its main classes, then the other classes, the dictionaries (`TypedDict`) and the type aliases, in stub order.

<!-- generated:start by website/scripts/reference-python.py; do not edit this page -->

## BACnetRouter

Own B/IP forwarding across one to 255 uniquely numbered ports.

This binding has no other transport types and no wire-control
authorizer. The first asynchronous method binds the object to the
running asyncio loop; another loop raises RuntimeError.

Keyword-only `application` shares the router's own application entity
(#2071): None (the default) only forwards; `"client"` gives the whole
entity to one `BACnetClient`; `"client_and_server"` splits it
between a `BACnetClient` and a `BACnetServer`. Build each with
`transport="router", router=router`; each takes its share when it
starts, once per router start. Closing the router ends both shares:
their requests then fail with BacnetTransportError, and a restarted
router makes new shares for newly started stacks. The network-control
receiver stays internal.

Successful awaited close/context exit joins router and transport cleanup.
Cancellation removes only a waiter: await close to observe the barrier.
A cleanup failure raises BacnetRouterCloseError with retained cause details;
subsequent close/start calls re-expose the failure without retry. Restart
is supported only after successful cleanup. Destruction requests best-effort
cleanup on the shared runtime; it is not an awaited cleanup guarantee.

Properties:

- `application: Optional[Literal["client", "client_and_server"]]`: How the router shares its application entity.
- `remote_table_updates: bool`: Whether the router takes routing-table updates from another network.
- `next_hop_rejects: bool`: Whether only a route's next router may change it with a reject.
- `control_policy: Literal["permissive", "hardened"]`: The wire-control policy.

### BACnetRouter.__init__

```python
def __init__(
    self,
    ports: Sequence[BipRouterPort],
    *,
    application: Optional[Literal["client", "client_and_server"]] = None,
    remote_table_updates: bool = False,
    next_hop_rejects: bool = False,
    control_policy: Literal["permissive", "hardened"] = "permissive",
) -> None
```

An `application` other than None, `"client"` or
`"client_and_server"` raises ValueError.

The router's own options (#2725), Rust's `RouterOptions`, apply at
each start. `remote_table_updates=True` takes an
Initialize-Routing-Table update that arrives from another network
(it carries an SNET), which the router otherwise drops and counts
in `denied`: turn it on only for a site that manages router tables
from a station on another network. `next_hop_rejects=True` lets a
received Reject-Message-To-Network change a learned route only when
it comes from that route's next router; one from another router is
still passed on and counted in `not_next_hop`. `control_policy`
is `"permissive"` (the default), which admits the wire controls
that change routes, or `"hardened"`, which denies every protected
one, route announcements and table updates among them, since this
binding installs no authorizer: the router then keeps its direct
routes and learns none from the wire, so only its directly attached
networks are reachable. Another policy raises ValueError, as does
`"hardened"` beside `remote_table_updates=True` or
`next_hop_rejects=True`: it denies every routing-table update and
every reject, so neither option could ever apply. A flag that isn't
a bool raises TypeError.

### BACnetRouter.start

```python
def start(self) -> Awaitable[None]
```

Start; duplicate start raises RuntimeError. Native startup errors
retain their exception class and errno. An unsuccessful native start
finishes rollback before reporting its original error.

### BACnetRouter.close

```python
def close(self) -> Awaitable[None]
```

### BACnetRouter.__aenter__

```python
def __aenter__(self) -> Awaitable[BACnetRouter]
```

### BACnetRouter.__aexit__

```python
def __aexit__(
    self,
    exc_type: Any = None,
    exc_value: Any = None,
    traceback: Any = None,
) -> Awaitable[None]
```

Await cleanup and preserve a context-body exception as primary.
A secondary cleanup failure remains observable through later close.

### BACnetRouter.ports

```python
def ports(self) -> Awaitable[list[RouterPortInfo]]
```

Copy actual addresses in input order; raises RuntimeError before
start or after close begins. Snapshots survive close.

### BACnetRouter.routes

```python
def routes(self) -> Awaitable[list[RouterRoute]]
```

Copy direct/learned routes; raises RuntimeError before start or
after close begins. No lock or wall-clock timestamp is exposed.

### BACnetRouter.counters

```python
def counters(self) -> Awaitable[RouterCounters]
```

Copy the router's counts since this start (#2725), zero at each
start; raises RuntimeError before start or after close begins.
`isolated_drops` counts frames whose way on lay between two
isolated ports, and `isolated_refusals` route changes refused
because an isolated port sent them (`BipRouterPort(isolated=True)`).
`denied` counts protected controls the policy refused and remote
table updates dropped; `control` has each class's decisions. The
fields are sampled one by one, not as one atomic snapshot.

## BipRouterPort

Immutable unopened B/IP port configuration; network\_number is 1..65534.

Port 0 requests an ephemeral UDP port. Mode is normal unless bbmd or
foreign\_device is explicitly supplied; supplying both raises ValueError.
Bbmd's default refusal of foreign-device registrations is unchanged.
Invalid IPv4 addresses raise ValueError; out-of-range integers raise
OverflowError. Construction opens no socket.

For B/IP, `interface` accepts an IPv4 literal or the exact OS interface
name (the adapter friendly name on Windows), resolved once, at
construction, to one usable IPv4 address; `broadcast_address` is not
derived from it. See "Selecting an interface" in docs/python-api.md.

`isolated=True` keeps the port apart from the router's other isolated
ports (#2716): the router carries nothing between two isolated ports and
hides each from the other, while both still reach the ordinary ports.
A request from one isolated port for one device behind another draws
Reject-Message-To-Network, reason 1, which a client raises at once as
`BacnetNetworkRejectError` (#2724); a broadcast is dropped quietly. One
isolated port alone changes nothing. It limits transit only and is no
trust level for a port; see "Isolated ports" in docs/python-api.md.

Properties:

- `network_number: int`
- `interface: str`
- `port: int`
- `broadcast_address: str`
- `mode: Literal["normal", "bbmd", "foreign_device"]`
- `isolated: bool`: Whether the router keeps this port apart from its other isolated ports.

### BipRouterPort.__init__

```python
def __init__(
    self,
    network_number: int,
    *,
    interface: str = "0.0.0.0",
    port: int = 47808,
    broadcast_address: str = "255.255.255.255",
    bbmd: Bbmd | None = None,
    foreign_device: ForeignDevice | None = None,
    isolated: bool = False,
) -> None
```

## RouterPortInfo

Frozen owned snapshot of an actual started B/IP port.

Properties:

- `index: int`
- `network_number: int`
- `mac: bytes`
- `address: str`

## RouterRoute

Frozen copied route; reachability is effective when captured.

Properties:

- `network_number: int`
- `port_index: int`
- `is_direct: bool`
- `next_hop: bytes | None`
- `reachability: Literal["reachable", "busy", "unreachable"]`

## RouterCleanupFailure

Frozen cause details from the retained native cleanup outcome.

Properties:

- `network_number: int | None`
- `port_index: int | None`
- `kind: Literal["transport", "worker"]`
- `message: str`
- `errno: int | None`
- `error_class: ErrorClass | None`
- `error_code: ErrorCode | None`
- `reason: RejectReason | AbortReason | None`: A remote Reject's `RejectReason` or Abort's `AbortReason`.
- `worker: str | None`

## RouterControlTotals

`class RouterControlTotals(TypedDict)`

One wire-control class's decisions since the router started:
`deny_total` counts every denial, `policy_deny_total` those the
hardened policy made with no authorizer to ask.

Keys:

- `allow_total: int`
- `deny_total: int`
- `policy_deny_total: int`

## RouterControlCounters

`class RouterControlCounters(TypedDict)`

The router's wire-control decisions by class, the Rust
`ControlDecisionCounters` field names: route announcements
(`i_am`), routing-table answers (`init_ack`), updates
(`init_mgmt`) and queries from another network (`init_query`),
flow control (`busy`, `available`), rejects and I-Could-Be-Router
offers.

Keys:

- `i_am: RouterControlTotals`
- `init_ack: RouterControlTotals`
- `init_mgmt: RouterControlTotals`
- `busy: RouterControlTotals`
- `available: RouterControlTotals`
- `reject: RouterControlTotals`
- `i_could_be: RouterControlTotals`
- `init_query: RouterControlTotals`

## RouterCounters

`class RouterCounters(TypedDict)`

`BACnetRouter.counters()`: every Rust router `Counters` field
under its own name, totals since this start that saturate at 2\*\*64-1
and name no peer, and the wire-control decisions under `control`.

Keys:

- `address_length_drops: int`
- `global_broadcast_dadr_drops: int`
- `broadcast_pdu_type_drops: int`
- `group_dadr_drops: int`
- `malformed: int`
- `bad_messages: int`
- `unknown_port: int`
- `hop_count_drops: int`
- `denied: int`
- `unroutable_rejects: int`
- `rejects: int`
- `solicitations: int`
- `write_failures: int`
- `egress_too_long: int`
- `refused: int`
- `undrained: int`
- `busy_announced: int`
- `available_announced: int`
- `duplicate_drops: int`
- `own_source_drops: int`
- `not_next_hop: int`
- `originated: int`
- `isolated_drops: int`
- `isolated_refusals: int`
- `control: RouterControlCounters`
<!-- generated:end -->
