# Full-server segmentation timing

`ServerConfig::apdu_segment_timeout_ms` supplies one segment timeout (Tseg) in
milliseconds. Its default is 5000. The generic, B/IP and SC builders expose
`apdu_segment_timeout_ms(...)`. A full server uses Tseg when waiting for a
response SegmentACK and uses `4 * Tseg` for incoming request inactivity.
`APDU_Timeout` and retry counts keep their separate meanings.

A selected Device must declare the same `Segmentation_Supported` as the server.
For `TRANSMIT`, `RECEIVE` or `BOTH`, its `DeviceConfig::apdu_segment_timeout`
(default 5000) must also match the server. The Device exposes the configured
value as read-only `APDU_Segment_Timeout`; its property list, metadata and PICS
include the property only when segmentation is supported. `NONE` omits it.
Unknown server modes, active zero timeouts, mismatches, multiplication overflow
and unrepresentable deadlines fail before transport startup or SC dialing.
An existing empty database remains supported.

| Mode | Receive segmented requests | Send segmented responses | Segment timeout property |
| --- | --- | --- | --- |
| `NONE` | No | No | Absent |
| `TRANSMIT` | No | Yes | Present |
| `RECEIVE` | Yes | No | Present |
| `BOTH` | Yes | Yes | Present |

Receive protocol expiry is silent and occurs only after elapsed inactivity is
**greater than** `4 * Tseg`. Equality remains live. Valid duplicate/gap traffic
refreshes protocol activity; only saving a new in-order segment refreshes the
independent local progress timer. The dispatch loop wakes for the earliest
live deadline even with no new input, and recomputes deadlines after refresh,
completion and removal. Timer-wheel resolution can delay observing expiry.

The existing local 16-second no-progress cap remains a resource policy. When
it expires, the server removes the transfer and sends a server `Abort` with
reason `OTHER` using the original next hop, routed destination and response
capability. All selected expired payload and quota ownership is released before
any Abort send awaits. A failed send does not restore state or prevent the
remaining selected Aborts. Dispatch owns these sends, so explicit stop cancels
and joins them. Original direct-SC capabilities cannot fall back to another
socket or a normal MAC send after retirement.

If both conditions are overdue when dispatch observes them, protocol expiry
wins and stays silent. At an exact shared `4 * Tseg` / 16-second boundary,
protocol expiry is not yet due, so the local cap sends `OTHER`. A noninitial
segment received after removal still gets the normal invalid-state Abort;
segment zero can start a fresh transfer. The existing 128 global / 16 per-peer
slots, 4 MiB saved-payload budget and 256-segment limit remain in force.

`DeviceIdentity` derives a matching full-server config and Device declaration
with a 6000 ms segment timeout. `with_apdu_segment_timeout_ms(...)` changes both.
The narrow `EndpointSession` roles require `NONE` in both an explicit identity
and the selected Device database, and reject unsupported declarations before
ingress. Client timers remain caller-owned; this does not change their coupling
or add segmentation to endpoint roles.

The Python full server accepts keyword-only `segmentation_supported` (default
`Segmentation.NONE`) and `apdu_segment_timeout_ms` (default 5000). It constructs
a matching Device and server configuration and validates the enum and timeout
before startup. See the [Python constructor](python-api.md#constructor).

The protocol boundaries follow ASHRAE 135-2020 Clauses 5.3, 5.4.5.2 and 12.11;
the 16-second cap and other resource limits are local policy. This change adds
focused timing, property, startup and wire evidence; it is not a claim of full
BACnet conformance or hardware qualification.
