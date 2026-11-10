# Time synchronization policy

A server sets its Device clock from every valid TimeSynchronization and
UTCTimeSynchronization request it receives, as Clauses 16.7 and 16.8 expect of
a receiver. `TimeSyncPolicy` is a **local hardening option** on top of that:
it limits which claimed sources may set the clock, how far one request may move
it, and how often requests are taken. Every limit is opt-in and independent of
the others, and of the DCC and mutation policies. The default policy applies
every valid request, so a server that never configures one behaves as before.

Both services are unconfirmed, so the policy changes nothing on the wire. A
refused request gets no response; it leaves the clock alone and does not reach
the Rust `on_time_sync` observer. A refusal is logged at DEBUG with its
reason (`Ignoring time synchronization request`).

## Defaults

| Rust field | Python key | Default | Effect |
|---|---|---|---|
| `enabled` | `enabled` | `true` | `false` refuses every request |
| `source_restriction` | `source_restriction` | `None` | Exact claimed sources that may set the clock; an empty list refuses all |
| `max_step` | `max_step_ms` | `None` | Largest correction one request may make, forward or back |
| `per_source_rate` | `per_source_rate` | `None` | Token bucket for each source |
| `global_rate` | `global_rate` | `None` | One token bucket for all sources |
| `coalesce_window` | `coalesce_window_ms` | 0 (off) | Least time between accepted requests from one source |
| `global_coalesce_window` | `global_coalesce_window_ms` | 0 (off) | Least time between accepted requests from any source |
| `max_sources` | `max_sources` | 256 | Sources the rate and coalescing state can track |

A rate is `TimeSyncRateLimit { max_per_second, burst_capacity }` in Rust and a
`(max_per_second, burst_capacity)` tuple in Python. A bucket starts full at
`burst_capacity` tokens and refills at `max_per_second` on the monotonic clock.
The buckets and windows count local and UTC requests together.

There is no default step cap. A controller that boots with a wrong clock may
need one large correction, and a cap would refuse it, so set one only when the
device's boot and recovery can live with that.

## What is checked, in order

1. Before the policy: request admission bounds the unconfirmed work in flight.
   DeviceCommunicationControl drops nothing here: the server refuses DISABLE,
   and DISABLE_INITIATION leaves incoming requests alone.
2. The request must decode, and no field of its date or time may be left
   unspecified.
3. `enabled`.
4. `source_restriction`. A request that carries a routed source (SNET and
   SADR) matches only a routed entry with the same network and the full
   address, never the MAC of the router it came through. A request with no
   routed source matches a direct entry equal to its transport MAC. Once the
   server knows its own network's number, it also matches a routed entry
   naming that number and its MAC: network numbers are unique, so that
   entry names the same station (#1458), as the
   [DCC restriction](dcc-policy.md#optional-exact-source-restriction) reads
   it. While the number is unknown, a routed entry matches routed requests
   only, and a direct entry never matches a routed one.
5. The server must have a Device clock; a clockless server refuses.
6. Rate and coalescing, under one lock: the global coalescing window, the
   global bucket, then, when a per-source rate or window is set, the source's
   window and bucket. A source that is not tracked yet is refused while the
   table holds `max_sources` entries, unless an entry has refilled and left its
   window, which makes room; an active entry is never dropped to reset its
   budget. Once the server knows its own network's number, a station's
   direct requests and those relayed with that number as SNET and its MAC as
   SADR are one source with one budget (#1458). While sources are tracked, a
   request whose source address is not 1 to 18 octets, or whose routed network
   is not 1 to 65534, is refused.
7. `max_step`. The requested time is compared with the clock in the request's
   own basis: a UTC request against the local time shifted by `UTC_Offset` and
   daylight saving. The bound is inclusive in both directions, and 0 allows
   only an exact match. With a cap set and no readable clock, the request is
   refused.
8. The clock is set, under the same hold of its lock as the `max_step`
   comparison, so a zone change by the host (`set_clock_zone`) can't fall
   between the two. Only then does the request spend its tokens and start
   its coalescing windows, so a refused request never uses up budget meant for
   a valid one. The `on_time_sync` observer runs afterwards, outside the lock.

## Sources are claims, not identities

An entry matches the address a request claims. B/IP addresses and routed
SNET/SADR pairs can be forged, and on BACnet/SC the MAC is the sender's VMAC,
not its certificate identity. The allowlist narrows who can move the clock by
accident or by casual misuse; it is not authentication. The DCC
[source restriction](dcc-policy.md#optional-exact-source-restriction) carries
the same caveat.

## Limits checked before startup

- At most 256 allowlist entries.
- Each entry address holds 1 to 18 octets (`BACnetAddress::MAX_MAC_LEN`, the
  longest source the network layer delivers), and a routed network is 1 to
  65534.
- `max_per_second` is positive and finite, and `burst_capacity` is positive.
- `max_sources` is 1 to 65536.

In Rust, `TimeSyncSourceRestriction::new` checks the entries and
`TimeSyncPolicy::validate` the rest; the generic, B/IP and SC builders run the
validation again before any transport starts or SC dials, and each failure is
an `Error::Encoding`. In Python the constructor checks the dict before any I/O
and raises `ValueError` for these limits, `TypeError` for an unknown key or a
value of the wrong type, and `OverflowError` for a negative or oversized
integer.

## Configuration

Rust sets the policy on any builder:

```rust
use std::time::Duration;
use bacnet_server::server::{
    BACnetServer, TimeSyncPolicy, TimeSyncRateLimit, TimeSyncSource, TimeSyncSourceRestriction,
};

let policy = TimeSyncPolicy {
    source_restriction: Some(TimeSyncSourceRestriction::new(vec![
        TimeSyncSource::Direct(vec![192, 168, 1, 10, 0xBA, 0xC0]), // B/IP: IPv4 then UDP port
        TimeSyncSource::Routed { network: 7, address: vec![0x2a] },
    ])?),
    max_step: Some(Duration::from_secs(300)),
    global_rate: Some(TimeSyncRateLimit { max_per_second: 0.2, burst_capacity: 2 }),
    ..TimeSyncPolicy::default()
};
let server = BACnetServer::bip_builder()
    .database(db)
    .time_sync_policy(policy)
    .build()
    .await?;
```

Python takes the same policy as the keyword-only `time_sync_policy` dict,
typed as the `TimeSyncPolicy` TypedDict in the stub. Each key is the Rust field
name, with `_ms` on the durations, which are whole milliseconds. A key left out
keeps its default, so `None` and `{}` both give the default policy.
`source_restriction` takes the `dcc_source_restriction` shape: `None` for a
directly attached source, or the routed source network, with the address
octets.

```python
server = BACnetServer(
    1234,
    time_sync_policy={
        "source_restriction": [
            (None, bytes([192, 168, 1, 10, 0xBA, 0xC0])),
            (7, b"\x2a"),
        ],
        "max_step_ms": 300_000,
        "global_rate": (0.2, 2),
    },
)
```

The policy is copied at construction and applies to every later `start()`;
each start builds a new native server with fresh buckets, windows and source
table.

## Evidence

`crates/bacnet-server/src/server/time_sync_policy_tests.rs` and
`requests/unconfirmed_time_sync_tests.rs` cover the matching, the step cap and
the buckets. `crates/rusty-bacnet/src/server/server_methods/time_sync_policy_tests.rs`
covers the Python dict, and `crates/rusty-bacnet/tests/test_time_sync_policy.py`
sends requests to a running B/IP server from raw sockets and reads
Local_Date and Local_Time back.
