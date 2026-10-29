---
title: "Python module functions"
description: "The functions at the top of the rusty_bacnet module, generated from the type stub."
---

:::note[On `dev` after the release]
This page is generated from the type stub on the `dev` branch and tracks the next release, so it can list classes, methods and keywords merged after v0.15.0 that the released wheel doesn't have. The [v0.15.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) and the [changes waiting for the next release](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d) say what changed.
:::

Functions at the top of the `rusty_bacnet` module. The [Python reference](/rusty-bacnet/reference/python/) lists every group. Each page opens with its main classes, then the other classes, the dictionaries (`TypedDict`) and the type aliases, in stub order.

<!-- generated:start by website/scripts/reference-python.py; do not edit this page -->

## enum_name

```python
def enum_name(property: PropertyIdentifier, value: int) -> str | None
```

The name of enumerated `value` as read from `property`
(`enum_name(PropertyIdentifier.EVENT_STATE, 3) == "HIGH_LIMIT"`), or
None when the property's enumeration isn't known from the property alone
(Present\_Value, vendor properties) or the number has no name in it.

## decode_log_records

```python
def decode_log_records(result: ReadRangeResult) -> list[dict[str, Any]]
```

Decode the records of a `read_range` result of a log's Log\_Buffer
(#1534).

Each record is a dict with `timestamp` as a `(date, time)` pair and
`datum`, a dict whose `kind` names the choice and whose key of the
same name holds its value: `log_status` (a `BitString` naming
`LOG_DISABLED`, `BUFFER_PURGED`, `LOG_INTERRUPTED`), `boolean`,
`real`, `enumerated`, `unsigned`, `signed`, `bitstring` (a
`BitString` of no known type), `null` (no value), `failure` (`(ErrorClass, ErrorCode)` members), `time_change` (seconds), `any` (the value's encoding),
`values` (a Trend Log Multiple record's list of such dicts),
`notification` (an Event Log record's event notification, its
`event_values` None or a dict whose `kind` names the event type in
snake case, `out_of_range`, with each value under its member's name;
see docs/python-api.md) or `audit_notification` (an Audit
Log record, as `audit_log_query` shows it). A Trend Log record adds
`status_flags` (a `BitString` naming `IN_ALARM`, `FAULT`,
`OVERRIDDEN`, `OUT_OF_SERVICE`), or None.

Raises ValueError for a result that isn't a log's Log\_Buffer, or whose
item data doesn't decode as `item_count` records, naming the first
record that fails by index and offset.

## list_serial_ports

```python
def list_serial_ports() -> list[str]
```

List the serial ports the operating system reports, as names to pass as
`serial_port=` for `transport="mstp"` (`/dev/ttyUSB0`,
`/dev/cu.usbserial-1410`, `COM3`).

macOS lists them through IOKit, Windows through SetupAPI and the registry,
and Linux from sysfs, so a port another program has open is listed too.
An empty list means the system reports none. Raises OSError (or the
subclass for the failure's kind) if the operating system can't be asked.

## refresh_logging

```python
def refresh_logging() -> None
```

Read the `logging` levels again, so that a level lowered since the
last read forwards the stack's events it now allows.

The stack's log events go to the logger `rusty_bacnet.<module path>`,
`rusty_bacnet.bacnet_transport.bip` for `bacnet_transport::bip`; only
the crate-level names (`rusty_bacnet.bacnet_transport` and so on) are
stable. TRACE goes out at level 5, which importing `rusty_bacnet` names
`TRACE`. The package logger has a `NullHandler`, so nothing prints
until the application configures logging.

The bridge decides which events to forward from a copy of the levels,
taken at import, whenever a client, server, hub or endpoint starts, and
by this function, so a disabled level never takes the GIL. Raised levels
filter at delivery as well.

Records arrive from one background thread, in order, shortly after the
event, and carry the event's fields as `record.bacnet_fields`. Past
4,096 waiting records, newer ones are dropped and a warning on
`rusty_bacnet` counts them. At exit, records already queued are
delivered first (waiting up to two seconds).

## inspect_sc_pem

```python
def inspect_sc_pem(
    identity_pem: bytes | bytearray,
    *,
    peer_trust_pem: bytes | bytearray | None = None,
    private_key_pem: bytes | bytearray | None = None,
    inspected_at_unix: int | None = None,
) -> ScPemInspection
```

Inspect bounded PEM bytes synchronously. Time is a signed 64-bit integer second.

ValueError: malformed/empty/oversized supplied PEM. TypeError: wrong input type,
including bool time. OverflowError: time outside the signed 64-bit range.
DER defects and key/date observations are returned in the report.

## inspect_sc_pem_files

```python
def inspect_sc_pem_files(
    identity_path: str | os.PathLike[str],
    *,
    peer_trust_path: str | os.PathLike[str] | None = None,
    private_key_path: str | os.PathLike[str] | None = None,
    inspected_at_unix: int | None = None,
) -> ScPemInspection
```

Read selected files once and inspect them without network I/O; OSError retains errno.

None time samples the clock once. A later connection may read changed files.
Same interpretation and input limits as inspect\_sc\_pem; no chmod or startup gate.
<!-- generated:end -->
