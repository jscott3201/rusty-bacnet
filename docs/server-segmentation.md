# Full-server segmentation timing

`DeviceIdentity::with_apdu_segment_timeout_ms(...)` supplies one segment timeout
(Tseg) in milliseconds. Its default is 5000. `DeviceIdentity::new` segments
both ways by default (#2617); `with_segmentation(...)` picks another mode. The
generic, B/IP and SC builders accept that identity through `.device(...)`. A
full server uses Tseg when waiting for a response SegmentACK and uses
`4 * Tseg` for incoming request inactivity.
`APDU_Timeout` and retry counts keep their separate meanings.

A selected Device must declare the same `Segmentation_Supported` as the server.
For `SEGMENTED_TRANSMIT`, `SEGMENTED_RECEIVE` or `SEGMENTED_BOTH`, its `DeviceConfig.identity` segment timeout
must also match the resolved server identity. The Device exposes the configured
value as read-only `APDU_Segment_Timeout`; its property list, metadata and PICS
include the property only when segmentation is supported. `NO_SEGMENTATION` omits it.
Unknown server modes, active zero timeouts, mismatches, multiplication overflow
and unrepresentable deadlines fail before transport startup or SC dialing.
An existing empty database remains supported.

| Mode | Receive segmented requests | Send segmented responses | Segment timeout property |
| --- | --- | --- | --- |
| `NO_SEGMENTATION` | No | No | Absent |
| `SEGMENTED_TRANSMIT` | No | Yes | Present |
| `SEGMENTED_RECEIVE` | Yes | No | Present |
| `SEGMENTED_BOTH` | Yes | Yes | Present |

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

The full server reassembles with `bacnet_endpoint_core::segmented_request`,
the receiver its clients use too (#2742). It acknowledges segment zero at
once, with the window the requester proposed, so a requester that sends
segment zero alone and waits for that answer (Clause 5.4.4.1) goes on at
once. After that it acknowledges each full window, counted from its last
acknowledgment, positive or negative (Clause 5.4.5.2), and the last segment.
A requester that sends its whole first window without waiting still
completes: the segments it sends again after segment zero's acknowledgment
are duplicates and are discarded. While the whole request still executes, a
segment of it sent again by the same requester, in the same trust context
and by a route of the same origin, is acknowledged again with the last
sequence number and window instead (Clause 5.4.5.3, #2751). That note lives
as long as the request is pending in the duplicate-request tracker, so a
request the tracker doesn't follow (over 64 KiB, past 256 pending, or a
LifeSafetyOperation) has none; once the request is answered, a segment sent
again draws the invalid-state Abort above. This workspace's client sends segment zero alone and
waits for its answer, so it sends nothing twice to a server that answers at
once; a receiver that never answers segment zero alone costs it one segment
timeout, once (see the client's segmented requests in
[the Rust API](rust-api.md#segmented-requests)). A requester that ignores
any acknowledgment short of its window's end, for the record, now sees the
segments it sends after segment zero's acknowledgment dropped as duplicates
and then asked for again with a negative acknowledgment: about two segment
timeouts for each request that spans more than one window.

`DeviceIdentity` is shared by full servers and endpoints, with a 5000 ms
segment timeout. `with_apdu_segment_timeout_ms(...)` changes both. An
`EndpointSession` takes any of the four modes (#2617): its full server follows
the table above, and its client follows the same identity, accepting segmented
answers only when the Device receives segments, sending segmented requests only
when it transmits them, and taking the identity's Tseg as its segment timeout.
A selected Device database must state the identity's mode. The client's APDU
timeout and retries stay the session's. When the Device receives segments, the
session's client also reassembles a segmented COV notification of either form
(#2646) with the full server's receiver, acknowledging as the full server
does, all with the server bit set, and answers the whole notification as one
that came in one APDU. A standalone client, or one on a session with no
server role, reassembles every segmented confirmed request the same way,
unless its `ClientConfig::segmentation` receives no segments: it executes a
notification and rejects any other service as unrecognized, as it does an
unsegmented one. A requester that stops midway meets the first of two clocks:
with a segment timeout under 4 s the transfer ends silently after four of
them; from 4 s up (the Device default is 5 s) the 16 s no-progress limit comes
first, and the requester gets an Abort (OTHER).

The Python full server accepts keyword-only `segmentation_supported` (default
`Segmentation.SEGMENTED_BOTH`, as the Rust server's, since #2617) and
`apdu_segment_timeout_ms` (default 5000). It constructs
a matching Device and server configuration and validates the enum and timeout
before startup. See the [Python constructor](python-api.md#constructor).

The protocol boundaries follow ASHRAE 135-2020 Clauses 5.3, 5.4.5.2 and 12.11;
the 16-second cap and other resource limits are local policy. This change adds
focused timing, property, startup and wire evidence; it is not a claim of full
BACnet conformance or hardware qualification.
