---
title: "Python errors"
description: "The exceptions rusty_bacnet raises, generated from the type stub."
---

:::note[On `dev` after the release]
This page is generated from the type stub on the `dev` branch and tracks the next release, so it can list classes, methods and keywords merged after v0.15.0 that the released wheel doesn't have. The [v0.15.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) and the [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d) say what changed.
:::

The exceptions methods raise. The [Python reference](/rusty-bacnet/reference/python/) lists every group. Each page opens with its main classes, then the other classes, the dictionaries (`TypedDict`) and the type aliases, in stub order.

<!-- generated:start by website/scripts/reference-python.py; do not edit this page -->

## BacnetError

`class BacnetError(Exception)`

Base exception for all BACnet errors.

## BacnetProtocolError

`class BacnetProtocolError(BacnetError)`

Raised on BACnet protocol-level errors (Error PDU).

Some services answer with a structured error body that adds fields to the
class and code. Each such attribute is None unless the device's error
carried it.

Attributes:
error\_class: The BACnet error class, an `ErrorClass` member
(an int equal to the wire value, #1816).
error\_code: The BACnet error code, an `ErrorCode` member.
first\_failed\_element\_number: For an AddListElement or
RemoveListElement ChangeList-Error, or a CreateObject-Error, the
position (from 1) of the list element or initial value that
failed, or 0 when the request failed for another reason. A
server's write\_property\_local of a list whose object refuses one
element sets it to that element's position in the list written.
first\_failed\_write\_attempt: For a WritePropertyMultiple-Error, the
object, property and index of the first write that failed.
first\_failed\_subscription: For a SubscribeCOVPropertyMultiple-Error
about one COV reference, its object, property and index. None for
a general failure of the request.
vendor\_id: For a ConfirmedPrivateTransfer-Error, the vendor of the
private service.
service\_number: For a ConfirmedPrivateTransfer-Error, the private
service number.
error\_parameters: For a ConfirmedPrivateTransfer-Error, the encoded
vendor-defined error parameters, when the device sent any.
vt\_session\_identifiers: For a VT-Close error that lists them, the
local identifiers of the VT sessions that could not be closed.

Attributes:

- `error_class: ErrorClass`
- `error_code: ErrorCode`
- `first_failed_element_number: Optional[int]`
- `first_failed_write_attempt: Optional[ObjectPropertyReference]`
- `first_failed_subscription: Optional[ObjectPropertyReference]`
- `vendor_id: Optional[int]`
- `service_number: Optional[int]`
- `error_parameters: Optional[bytes]`
- `vt_session_identifiers: Optional[list[int]]`

## BacnetTimeoutError

`class BacnetTimeoutError(BacnetError)`

Raised when a BACnet operation times out.

## BacnetRejectError

`class BacnetRejectError(BacnetError)`

Raised when a BACnet request is rejected.

Attributes:
reason: The reject reason, a `RejectReason` member (an int equal
to the wire value, #1816).

Attributes:

- `reason: RejectReason`

## BacnetAbortError

`class BacnetAbortError(BacnetError)`

Raised when a BACnet transaction is aborted.

Attributes:
reason: The abort reason, an `AbortReason` member (an int equal
to the wire value, #1816).

Attributes:

- `reason: AbortReason`

## BacnetLogNotAdvancingError

`class BacnetLogNotAdvancingError(BacnetError)`

Raised when a paged log read can't advance (#1530).

The device answered a page by sequence number with records from before
the one asked for, as bacnet-stack 1.6.1 does past its sequence wrap, or
with none where its record counts say it holds that record. Read the log
by position instead.

Attributes:
requested: The sequence number asked for.
returned: What the device numbered the first record it sent; None
when it sent none.

Attributes:

- `requested: int`
- `returned: int | None`

## BacnetReadRangeViolationError

`class BacnetReadRangeViolationError(BacnetError)`

Raised when a strict `read_range` refuses an answer that breaks a
ReadRange rule (#1531). Read again with `validation="lenient"` to keep
the answer and the rules it broke.

Attributes:
rule: The rule broken, in snake case, such as
`"zero_first_sequence_number"`.

Attributes:

- `rule: str`

## BacnetBvlcNakError

`class BacnetBvlcNakError(BacnetError)`

Raised when a BBMD answers a management request (`read_bdt` and
its siblings, #1841) with a BVLC-Result NAK. No reply raises
`BacnetTimeoutError` instead, so a refusal and silence never share a
class.

Attributes:
result\_code: The BVLC result code it sent, such as 0x0010 (Write-BDT
refused), 0x0030 (registration refused) or 0x0050 (delete refused).

Attributes:

- `result_code: int`

## BacnetBvlcUnexpectedResponseError

`class BacnetBvlcUnexpectedResponseError(BacnetError)`

Raised when a management request (`read_bdt` and its siblings,
\#1841) is answered by a BVLC message it doesn't take, such as a success
result to a table read.

Attributes:
expected: The BVLC function code the request takes as its answer.
got: The BVLC function code that came.

Attributes:

- `expected: int`
- `got: int`

## BacnetNetworkRejectError

`class BacnetNetworkRejectError(BacnetError)`

Raised when a router answers a network-layer message the client sent
with a Reject-Message-To-Network, as `routing_table()` reports it
(#2612). Silence raises `BacnetTimeoutError` instead.

Attributes:
reason: The reject reason octet the router sent.
network: The network the router named.

Attributes:

- `reason: int`
- `network: int`

## BacnetLookupError

`class BacnetLookupError(BacnetError, LookupError)`

Raised when no device could be placed for the name a call gave
(#2561): no address in the discovery table claims the instance and none
answered a Who-Is for it, or no router is known for a routed address's
network. Also a `LookupError`. Nothing was sent to the device. The
typed `_from_device` and `_to_device` calls raise it for an
instance the table doesn't hold.

Attributes:
target: The device as it was named: an instance (`"1234"`,
`"2:1234"`) or an address (`"2:0x05"`).
instance: The instance named (`1234` for `"1234"` or
`"2:1234"`); None when the device was named by an address.
candidates: An empty list here.

Attributes:

- `target: Optional[str]`
- `instance: Optional[int]`
- `candidates: list[tuple[int, bytes]]`

## BacnetAmbiguousDeviceError

`class BacnetAmbiguousDeviceError(BacnetLookupError)`

Raised when a call names a device by its instance and two or more
addresses in the discovery table claim that instance, so which device is
meant is unknown (#2560). A `BacnetLookupError`. Nothing was sent.
Each claimant keeps its own entry in `discovered_devices()` and stays
reachable by its address; once only one claims the instance, by the
purge or a new I-Am, the instance works again.

Attributes:
target: The instance, as text.
instance: The device instance the call named.
candidates: Every claimant as `(network, mac)`, ordered by network
and then MAC: network 0 is the local network, where `mac` is
the device's own; on another network it is the device's MAC
there. Each is a `BACnetClient.device()` target.

Attributes:

- `instance: int`
- `candidates: list[tuple[int, bytes]]`

## BacnetPagingError

`class BacnetPagingError(BacnetError)`

Raised when a device breaks a rule of a paged read, so reading on
would repeat what was read or never end (#2561): an answer naming an
object an earlier answer named, or saying more remain with no new
object to go on after. No call of this release's Python API pages yet;
the alarm reads to come raise it.

Attributes:
service: The paged service, such as `"GetEventInformation"`.
object\_id: The object named again; None when none was.

Attributes:

- `service: str`
- `object_id: Optional[ObjectIdentifier]`

## BacnetWriteDeniedError

`class BacnetWriteDeniedError(BacnetError)`

Raised when the client's `write_policy` refuses the priority a
`write` or `relinquish` names (#2561): 1, 2, 5 or 6 by default.
Nothing was sent. The message says how to allow it:
`BACnetClient(write_policy=WritePolicy().allow_priorities([n]))`.

Attributes:
priority: The priority refused, 1 to 16.
relinquish: True for a relinquish.
object\_id: The object named.
property\_id: The property named (Present\_Value for a relinquish).
array\_index: The array index named, None for the whole property.

Attributes:

- `priority: int`
- `relinquish: bool`
- `object_id: ObjectIdentifier`
- `property_id: PropertyIdentifier`
- `array_index: Optional[int]`

## BacnetReadBackError

`class BacnetReadBackError(BacnetError)`

Raised when the device acknowledged a `relinquish`, so it took
effect, but the read of the object's priorities after it failed
(#2561). Don't send it again: read them with `priority_array` once the
device answers. `__cause__` is why the read failed, such as a
`BacnetTimeoutError`; this class is not one, so a retry on timeouts
leaves it alone.

Attributes:
written: The relinquish as the device acknowledged it.

Attributes:

- `written: Written`

## BacnetNoPriorityArrayError

`class BacnetNoPriorityArrayError(BacnetProtocolError)`

Raised when the device says the object has no Priority\_Array, so its
Present\_Value isn't commandable: a value object built without one
(#2561). `error_class` and `error_code` are the device's PROPERTY
and UNKNOWN\_PROPERTY. After a `relinquish`, the device took the NULL
first as a write to a property that isn't commandable, which changes
nothing, and `written` holds it.

Attributes:
object\_id: The object.
written: The relinquish acknowledged before the read; None for
`priority_array`, which writes nothing.

Attributes:

- `object_id: ObjectIdentifier`
- `written: Optional[Written]`

## BacnetTransportError

`class BacnetTransportError(BacnetError, OSError)`

Raised when a transport socket or I/O operation fails.

Covers a failed bind or listen (B/IP, B/IPv6, `ScHub.start`), a refused
or failed dial, and other socket errors. It is also an `OSError`:
`errno` is the operating system's code when it reported one, else the
code for the failure's kind (for example `errno.EADDRINUSE`), else
`None`; `strerror` is the message. `except OSError` and
`except BacnetError` both catch it. Unlike a bare `OSError` it is
not narrowed to `ConnectionRefusedError` and the like, so test
`errno`.

Attributes:

- `errno: int | None`
- `strerror: str | None`
- `sc_untrusted_peer: ScUntrustedPeerInfo | None`: Untrusted certificate evidence from the actual failed outbound TLS attempt. None when unavailable at the verification observation point. This is not an authenticated peer or a history of background reconnect failures.

## BacnetRouterCloseError

`class BacnetRouterCloseError(BacnetError)`

A settled cleanup failure. The router retains it and prevents restart.

Attributes:

- `failures: tuple[RouterCleanupFailure, ...]`

## ObjectPropertyReference

`class ObjectPropertyReference(TypedDict)`

The object, property and array index a structured error names.

Keys:

- `object_identifier: ObjectIdentifier`
- `property_identifier: PropertyIdentifier`
- `property_array_index: int | None`
<!-- generated:end -->
