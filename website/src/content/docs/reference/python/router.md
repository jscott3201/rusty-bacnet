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

This binding has no other transport types or route administration. It
uses native RouterOptions defaults, including their permissive
wire-control policy. The first asynchronous method binds the object to
the running asyncio loop; another loop raises RuntimeError.

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

### BACnetRouter.__init__

```python
def __init__(
    self,
    ports: Sequence[BipRouterPort],
    *,
    application: Optional[Literal["client", "client_and_server"]] = None,
) -> None
```

An `application` other than None, `"client"` or
`"client_and_server"` raises ValueError.

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

## BipRouterPort

Immutable unopened B/IP port configuration; network\_number is 1..65534.

Port 0 requests an ephemeral UDP port. Mode is normal unless bbmd or
foreign\_device is explicitly supplied; supplying both raises ValueError.
Bbmd's default refusal of foreign-device registrations is unchanged.
Invalid IPv4 addresses raise ValueError; out-of-range integers raise
OverflowError. Construction opens no socket.

Properties:

- `network_number: int`
- `interface: str`
- `port: int`
- `broadcast_address: str`
- `mode: Literal["normal", "bbmd", "foreign_device"]`

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
<!-- generated:end -->
