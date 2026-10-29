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
