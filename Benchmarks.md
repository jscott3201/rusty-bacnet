# Rusty BACnet benchmarks

This page reports what rusty-bacnet does on one machine: the 0.14.0
development branch after the round-trip performance pass of
[#1713](https://github.com/jscott3201/rusty-bacnet/issues/1713), measured on
2026-10-21, with an A/B comparison against 0.13.0 on the same machine
([section 1.1](#11-what-changed-since-0130)). It covers how fast it encodes
and decodes, how long a request takes on each
transport, how many requests a server answers, what routing costs, how close
MS/TP gets to the wire, and how small the embedded builds are. It also says
how each number was measured, so you can repeat it, and where the numbers are
weak.

## Contents

1. [Summary](#1-summary)
2. [Test environment](#2-test-environment)
3. [Encoding and the portable codecs](#3-encoding-and-the-portable-codecs)
4. [Transports](#4-transports)
5. [Routing](#5-routing)
6. [Scale and stress](#6-scale-and-stress)
7. [Docker cross-network results](#7-docker-cross-network-results)
8. [Embedded footprint](#8-embedded-footprint)
9. [Python bindings](#9-python-bindings)
10. [Guidance for future work](#10-guidance-for-future-work)
11. [How to reproduce](#11-how-to-reproduce)

## 1. Summary

These are the figures most projects ask about. Every one comes from a single
run on an Apple M5 laptop over the loopback interface, so treat them as what
the stack itself costs, not as what a building network will show: a real
network adds its own delay, and a real field bus is far slower than the
host.

- **Request latency on IP transports.** One ReadProperty round trip, client
  to server and back, takes a median of 29 microseconds over BACnet/IP
  (B/IP, BACnet over UDP and IPv4), 25 microseconds over BACnet/IPv6
  (B/IPv6), and 48 microseconds over BACnet Secure Connect (BACnet/SC,
  BACnet over Transport Layer Security (TLS) WebSockets through a hub)
  with mutual TLS
  ([section 4](#4-transports)).
- **Server throughput.** One B/IP server answered about 82,000
  ReadProperty requests per second from 10 concurrent clients, and 65,000
  to 71,000 requests per second from 25 to 50 clients. Over B/IPv6 it
  reached 101,000, and over BACnet/SC 93,000 with 50 clients. A weighted
  mix of reads, writes, ReadPropertyMultiple, change-of-value (COV)
  subscriptions and Who-Is reached 85,400 operations per second with no
  errors ([section 6](#6-scale-and-stress)).
- **Database size does not slow reads.** ReadProperty latency stayed at a
  median of 24 microseconds from 100 to 10,000 objects; resident memory
  grew by about 1.3 kilobytes per Analog Input object
  ([section 6.2](#62-object-database-size)).
- **Routing.** A request routed through `BACnetRouter` from one B/IP
  network to another took a median of 43 microseconds against 22 direct,
  so the router hop adds about 21 microseconds on loopback. Under load the
  routed path delivered 57,000 requests per second, 88% of the direct
  path's rate at 50 clients ([section 5](#5-routing)).
- **MS/TP efficiency.** Over a simulated RS-485 line, MS/TP (BACnet's
  Master-Slave/Token-Passing serial link) reached 90% to 100% of the wire
  limit for large replies at every baud rate from 9,600 to 115,200. For a
  small reply the fixed host overhead of about 7 milliseconds per request
  matters more: 93% of the wire limit at 9,600 baud, 52% at 115,200
  ([section 4.4](#44-mstp)).
- **Embedded footprint.** Without an allocator, the MS/TP master node links
  into 17.9 kilobytes of flash and 5.5 kilobytes of static RAM on a
  Cortex-M3, the B/IP codec into 10.9 kilobytes of flash, and a two-port
  B/IP-to-MS/TP router into 43.0 kilobytes of flash and 15.6 kilobytes of
  RAM ([section 8](#8-embedded-footprint)).
- **Python.** A request through the Python package takes 70 to 100
  microseconds, and one Python client reaches about 34,000 requests per
  second with 10 in flight and 36,000 with 16 ([section 9](#9-python-bindings)).
- **Resource limits by design.** A server accepts 64 COV subscriptions per
  subscriber by default, 16 confirmed requests in flight per client, a
  ReadPropertyMultiple answer of at most 256 results, and a BBMD (the B/IP
  broadcast relay) admits 16 registrations per source address per second.
  Some stress steps go past these limits on purpose; each place is noted
  where it shows.

### 1.1 What changed since 0.13.0

Since 0.13.0 the performance pass of
[#1713](https://github.com/jscott3201/rusty-bacnet/issues/1713) cut the cost
of a request round trip in several steps. The last of them, measured here,
did three things:

- The server's per-request handler state shrank from 3,248 to 976 bytes,
  small enough for the memory allocator's fast path.
- A received request carries its reply authority in one pointer each
  instead of 88 bytes inline.
- The network layer no longer runs a task between the transport and the
  application: the receiver decodes each packet as it is read.

Counted with callgrind (an instruction-counting profiler) on Linux, a B/IP
ReadProperty round trip with client and server in one process went from
47,610 to 41,454 instructions on one thread (13% fewer) and from 48,711 to
42,775 on the multi-threaded runtime, with 22 heap allocations instead of 23
and 2,316 bytes allocated instead of 4,588. Those counts compare the
development branch just before this last step with it applied.

**A/B against 0.13.0.** Each pair below ran back to back on this machine,
alternating between the two builds. The 0.13.0 build ran with the
benchmark harness of its own report (commit `deaeb745`); the new build ran
with the current harness, which connects every client before a step's
clock starts and keeps one latency record per client
([#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714)). Both
count one client's requests the same way.

| Measure | 0.13.0 | This edition | Change |
| --- | --- | --- | --- |
| One stress client, ReadProperty (requests per second, two runs) | 29,289 and 29,432 | 40,294 and 39,666 | about 36% more |
| One stress client, median latency (microseconds) | 33 | 22 to 23 | about a third less |
| Routed B/IP to B/IP, 1 client (requests per second) | 17,693 | 22,755 | 29% more |
| Routed B/IP to B/IP, 5 clients (requests per second) | 44,866 | 46,457 | 4% more |
| Routed B/IP to B/IP, 10 clients (requests per second) | 50,004 | 52,928 | 6% more |
| Routed B/IP to B/IP, 25 clients (requests per second) | 56,110 | 57,317 | 2% more |
| Routed B/IP to B/IP, 50 clients (requests per second) | 54,812 | 57,605 | 5% more |
| Routed B/IP to MS/TP at 115,200 baud, 1 client (requests per second) | 66 | 66 | the same: the serial line sets the rate |
| Routed B/IP to MS/TP at 115,200 baud, 5 clients (requests per second) | 67 | 66 | the same |

Against the 0.13.0 edition of this page, measured on 2026-10-13 on the same
machine, the Criterion suites show the same direction: a B/IP ReadProperty
round trip from 35.5 to 28.7 microseconds, B/IPv6 from 31.1 to 24.6, and
BACnet/SC from 54.0 to 48.2; sequential B/IP ReadProperty from 29,300 to
38,600 requests per second. Decoding a ReadProperty request fell from 19.9
to 5.0 nanoseconds ([#1729](https://github.com/jscott3201/rusty-bacnet/issues/1729)).
The router hop gained least: its own ports still hand packets between
tasks, which this pass did not change.

## 2. Test environment

| Item | Value |
| --- | --- |
| Machine | MacBook Pro (Mac17,2), Apple M5, 10 cores (4 performance, 6 efficiency), 16 gigabytes of memory |
| Operating system | macOS 27.2 (build 26B5112a) |
| Rust | rustc 1.99.0 (b940084d7 2026-09-28), release builds; Criterion suites use the `bench` profile |
| TLS provider | rustls with aws-lc-rs |
| Commit | Branch `perf/round-trip-cuts`: `dev` at `f056aba2` with #1713's last cuts (commits `c5c780cb` to `85ee9cea`) and this report's harness fixes. The 0.13.0 A/B: tag `v0.13.0` with the harness of commit `deaeb745` |
| Docker | Not re-run for this edition: [section 7](#7-docker-cross-network-results) is the 0.13.0 run, on Docker Desktop 29.8.2 |
| Python | CPython 3.13.15, the `rusty-bacnet` package built with `maturin develop --release` |
| Date | 2026-10-21 (sections 6.6 and 7: 2026-10-13) |

How the numbers were taken:

- Every suite ran by itself, one after another, on an otherwise idle
  machine. Running two suites at once makes both numbers wrong.
- All network traffic stays on the loopback interface (127.0.0.1 or ::1).
  Loopback has no wire delay and almost no loss, so it flatters every
  network figure: it shows the cost of the stack, not of a network.
- Criterion figures are the median of Criterion's estimate over its default
  100 samples, after its warm-up. The stress figures are a single run of
  each step, usually 10 seconds long; they vary by a few percent between
  runs.
- A laptop changes its clock speed with temperature and load. The
  efficiency cores also take some threads. Expect a server-class machine to
  give steadier numbers, not necessarily faster ones.
- The 0.13.0 edition of this page was measured on this machine on
  2026-10-13 with the same compiler and macOS 27.2 (build 26B5091g);
  [section 1.1](#11-what-changed-since-0130) compares the two. The edition
  before it measured 0.8.0 on a machine of the same model under macOS 26.4
  and Rust 1.94, which is not comparable.

## 3. Encoding and the portable codecs

These benchmarks run no input or output: they measure the processor time
of one operation. The times are on the M5; a microcontroller runs the same
code tens to hundreds of times slower.

### 3.1 Application and network layer encoding

Suite `encoding`. The NPDU is the network protocol data unit, the APDU the
application protocol data unit.

| Operation | Median time (nanoseconds) |
| --- | --- |
| Encode a ReadProperty request (service parameters only) | 24.1 |
| Decode a ReadProperty request | 5.0 |
| Encode an NPDU carrying 100 bytes | 16.5 |
| Decode an NPDU carrying 100 bytes | 15.6 |
| Encode a confirmed-request APDU header | 18.4 |
| Decode a confirmed-request APDU header | 10.3 |
| Encode a whole ReadProperty request: service, APDU and NPDU | 82.1 |

A complete request takes about 80 nanoseconds to build, so encoding is a
fraction of a percent of the 29-microsecond round trip in
[section 4](#4-transports). Decoding a ReadProperty request is four times
faster than in 0.13.0 (19.9 nanoseconds then). Decoding an NPDU (11.2
nanoseconds then) and building a whole request (68.7 then) are slower;
[section 10](#10-guidance-for-future-work) lists them.

### 3.2 Portable `no_std` codecs and the router core

Suite `codecs`. These crates build for
microcontrollers without an allocator; the same code runs here on the host.
CRC stands for cyclic redundancy check. A routed NPDU names its destination
network and address (DNET and DADR); a router adds the source network and
address (SNET and SADR) when it forwards one.

| Codec | Operation | Median time (nanoseconds) |
| --- | --- | --- |
| B/IP BACnet Virtual Link Layer (BVLL) | Encode an Original-Unicast-NPDU of 13 bytes | 4.5 |
| B/IP BVLL | Decode it | 3.8 |
| B/IP BVLL | Encode a Forwarded-NPDU of 1,497 bytes | 15.7 |
| B/IP BVLL | Decode it (the NPDU is borrowed, not copied) | 3.9 |
| MS/TP frame | Encode a Token frame | 3.7 |
| MS/TP frame | Encode a standard data frame of 501 bytes | 883 |
| MS/TP frame | Decode it | 858 |
| MS/TP frame | Encode an extended data frame of 1,497 bytes | 6,203 |
| MS/TP frame | Decode it (in place) | 5,210 |
| MS/TP frame | CRC-16 over 501 bytes (the standard frame's data check) | 871 |
| MS/TP frame | CRC-32K over 1,497 bytes (the extended frame's data check) | 4,963 |
| Router core | Parse a routed ReadProperty NPDU (DNET, 6-byte DADR) | 7.1 |
| Router core | Parse it and write the final-hop rewrite (DNET dropped, SNET and SADR added) | 9.2 |
| `bacnet-encoding` | Decode the same routed NPDU into its owned form, for comparison | 20.7 |

What the table shows:

- The B/IP codec costs a few nanoseconds whatever the size, because decoding
  only checks the header and borrows the payload.
- The MS/TP frame cost is almost all checksum: the CRC-16 runs at about
  1.7 nanoseconds per byte and the CRC-32K at about 3.3. The default
  CRC-32K uses a 16-entry table to save flash; the `crc32k-table` feature
  trades about 1 kilobyte more flash for half the table lookups. Even the
  slowest case is far below the frame's time on the wire: a 1,497-byte
  extended frame takes 130 milliseconds to send at 115,200 baud.
- The router core's borrowed NPDU view parses a routed NPDU in about a
  third of the time `bacnet-encoding`'s owned decoder takes, and rewrites it
  for the next hop in 9 nanoseconds.

### 3.3 ReadRange of a full Trend Log

Suite `read_range`. The server's cost to answer one ReadRange for 100
records from the middle of a full Trend Log, by request kind and log size.
The page is the same size at every log size, so growth with the log is the
cost of finding the window.

| Request kind | Median time, 10,000-record log (microseconds) | Median time, 100,000-record log (microseconds) |
| --- | --- | --- |
| By position | 7.6 | 7.5 |
| By sequence number | 7.7 | 7.6 |
| By time | 7.7 | 7.7 |
| By time, in a log whose clock stepped back once | 30.9 | 152.7 |

Position, sequence and time lookups cost the same at 10 times the log
size. A log whose clock stepped back can't be searched by halving, so the
time lookup scans it, and grows with the log: still under a fifth of a
millisecond at 100,000 records.

## 4. Transports

### 4.1 How these were measured

Suites `bip_latency`, `bip_throughput`, `bip6_latency`, `bip6_throughput`,
`sc_mtls_latency` and `sc_mtls_throughput`. One client and one server in the
same process, over loopback. The server holds a Device, 10 Analog Inputs,
an Analog Output and a Binary Value.

- **Latency** is one request and its answer: ReadProperty of an Analog
  Input's Present_Value, WriteProperty of an Analog Output's
  Present_Value, and ReadPropertyMultiple of 10 Present_Values.
- **Throughput** sends 10, 100 or 1,000 requests one after another, each
  waiting for its answer, and reports the batch's time. It is sequential,
  so it is close to the inverse of latency; for concurrent clients, see
  [section 6.1](#61-concurrent-clients).
- BACnet/SC runs through a hub in the same process, with mutual TLS: the
  hub and the node each check the other's certificate. The older
  server-authentication-only mode has been removed, so there is no
  separate figure for it.

### 4.2 Latency

| Transport | ReadProperty median (microseconds) | WriteProperty median (microseconds) | ReadPropertyMultiple of 10 objects, median (microseconds) |
| --- | --- | --- | --- |
| B/IP | 28.7 | 30.9 | 31.1 |
| B/IPv6 | 24.6 | 29.5 | 32.7 |
| BACnet/SC with mutual TLS | 48.2 | 49.1 | 54.9 |

ReadPropertyMultiple of 10 objects costs only 2 to 8 microseconds more
than one ReadProperty: the round trip, not the property reads, is the cost.
In 0.13.0 the three ReadProperty figures were 35.5, 31.1 and 54.0
microseconds.

### 4.3 Sequential throughput

The rate is the batch size divided by the batch's median time.

| Transport | ReadProperty, 1,000 in a row (requests per second) | WriteProperty, 1,000 in a row (requests per second) | Median time for 1,000 ReadProperty requests (milliseconds) |
| --- | --- | --- | --- |
| B/IP | 38,600 | 31,600 | 25.9 |
| B/IPv6 | 40,500 | 35,700 | 24.7 |
| BACnet/SC with mutual TLS | 20,800 | 20,400 | 48.1 |

Batches of 10 and 100 ran at the same rates within 5%.

### 4.4 MS/TP

MS/TP runs over RS-485 serial lines at low baud rates, where the wire, not
the processor, limits throughput. The `stress-test mstp` scenario
measures how close the stack gets to that limit.

**Method.** A client station (MAC address 0) and a server station (MAC
address 1), the only two masters on the line, run the real MS/TP transport
over a simulated RS-485 line (`SimWire` in `benchmarks/src/stress/mstp.rs`).
The line hands each frame's bytes to the other station no sooner than a
serial port's UART (its transmitter hardware) sending them at the baud
rate would, in chunks of about 4
milliseconds, and counts every byte. The client sends one ReadProperty
after another for 10 seconds (20 for the largest reply) at each baud rate.
Three reply sizes:

- **Small:** an Analog Input's Present_Value. Request and reply frames
  together are 52 bytes.
- **Standard:** the Device's Object_List of 92 objects. Request and reply
  frames together are 507 bytes, near the most a standard frame carries.
- **Extended:** the Device's Object_List of 289 objects, over extended
  frames. Request and reply frames together are 1,496 bytes: the reply's
  APDU is about 1,460 bytes, near the 1,476-byte limit.

**The wire limit** is the rate the line could carry the same frames at if
nothing else took time: per request, the request and reply data frames,
one Token frame each way (8 bytes each), and the turnaround of 40 bit times
before each of the four frames, all at 10 bits per byte (start bit, 8 data
bits, stop bit). Every request in every run took exactly those four frames,
so the stack sent no extra frames; the gap between the measured rate and
the limit is time the stations spent between frames.

| Baud rate | Reply | Requests per second | Wire limit (requests per second) | Share of the wire limit | Median latency (milliseconds) | 99th percentile latency (milliseconds) |
| --- | --- | --- | --- | --- | --- | --- |
| 9,600 | Small | 10.7 | 11.4 | 93% | 93.6 | 96.6 |
| 38,400 | Small | 35.8 | 45.7 | 78% | 27.7 | 29.9 |
| 76,800 | Small | 61.6 | 91.4 | 67% | 16.0 | 18.3 |
| 115,200 | Small | 71.8 | 137.1 | 52% | 14.0 | 15.2 |
| 9,600 | Standard | 1.8 | 1.8 | 99% | 568.3 | 570.4 |
| 38,400 | Standard | 6.8 | 7.1 | 96% | 146.3 | 148.1 |
| 76,800 | Standard | 13.1 | 14.2 | 92% | 76.2 | 78.1 |
| 115,200 | Standard | 19.1 | 21.4 | 90% | 52.4 | 54.4 |
| 9,600 | Extended | 0.6 | 0.6 | 100% | 1,598.5 | 1,599.5 |
| 38,400 | Extended | 2.5 | 2.5 | 99% | 404.2 | 405.2 |
| 76,800 | Extended | 4.9 | 5.0 | 97% | 205.2 | 206.8 |
| 115,200 | Extended | 7.2 | 7.5 | 95% | 139.4 | 141.7 |

What it shows:

- Each request spends a roughly fixed 5 to 9 milliseconds beyond its wire
  time, about 1.5 to 2 milliseconds per frame. That is mostly the host's
  timer resolution: Tokio's timers wake on whole milliseconds, and the
  stations wait on a timer before every frame they send. For large replies
  the overhead is small next to the wire time, so the stack reaches 90% to
  100% of the limit. For small replies at high baud rates the wire time
  shrinks to 7 milliseconds and the overhead is half the total.
- One client gets no more from the line by sending concurrently: with one
  information frame per token (the default `max_info_frames` of 1), the
  client sends one request per token rotation
  ([section 5](#5-routing) shows the same through the router).
- The simulated line has no noise, no adapter latency and no other
  stations. A USB serial adapter adds several milliseconds of receive
  latency per frame (16 milliseconds by default on FTDI parts), so real
  hardware will sit further below the limit than this.

### 4.5 Comparison

| Transport | ReadProperty median latency (microseconds) | Sequential ReadProperty requests per second | Most requests per second measured with concurrent clients |
| --- | --- | --- | --- |
| B/IP | 28.7 | 38,600 | 82,200 (10 clients) |
| B/IPv6 | 24.6 | 40,500 | 101,400 (10 clients) |
| BACnet/SC with mutual TLS | 48.2 | 20,800 | 92,900 (50 clients) |
| MS/TP at 115,200 baud, small reply | 14,000 | 71.8 | 71.8 (one line, see above) |
| MS/TP at 38,400 baud, small reply | 27,700 | 35.8 | 35.8 |

BACnet/SC costs about 70% more per request than B/IP when one request is
in flight: the TLS record layer and the hub's extra hop. With many clients
the hub keeps up, and BACnet/SC passed B/IP from 25 clients on
([section 6.1](#61-concurrent-clients)). An MS/TP request takes several
hundred times as long as one over IP because the serial line is that much
slower; the stack's own share of an MS/TP request is a few milliseconds.

## 5. Routing

The `router` scenario of `stress-test` now routes real traffic through
`BACnetRouter` (`--path bip` and `--path mstp`); `--path direct` keeps the
same-network baseline. Every routed request names network 2 and the
server's MAC address in its NPDU, so it takes the router's forwarding path
both ways: the router adds the source network and address on the way in
and strips the destination on the final hop.

**B/IP to B/IP.** Clients on B/IP network 1, the server on B/IP network 2,
the router between them, all on loopback. Each client sends ReadProperty
requests one after another for 10 seconds at each step.

| Concurrent clients | Direct: requests per second | Direct: median latency (microseconds) | Direct: 99th percentile latency (microseconds) | Routed: requests per second | Routed: median latency (microseconds) | Routed: 99th percentile latency (microseconds) | Routed rate as a share of direct |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 42,831 | 22 | 34 | 22,053 | 43 | 61 | 51% |
| 5 | 82,480 | 58 | 107 | 47,318 | 103 | 158 | 57% |
| 10 | 84,271 | 115 | 222 | 53,307 | 184 | 293 | 63% |
| 25 | 71,557 | 349 | 582 | 57,296 | 426 | 719 | 80% |
| 50 | 65,381 | 778 | 1,270 | 57,212 | 836 | 1,484 | 88% |

The router hop adds about 21 microseconds to an idle request: one more UDP
datagram each way through the loopback interface (four per request instead
of two), and the router's receive, table lookup, rewrite and send. Under load the router is a
separate set of tasks working in parallel with the server, so the gap
narrows to 12% at 50 clients. No request failed on either path. The direct
path gained more from #1713 than the routed one, so one idle client's
routed rate is now about half of its direct rate (63% in 0.13.0).

**B/IP to MS/TP.** Clients on B/IP network 1, the server on an MS/TP
network at 115,200 baud behind the router (the router's MS/TP port is
station 0, the server station 1, over the simulated line of
[section 4.4](#44-mstp)).

| Concurrent clients | Requests per second | Median latency (milliseconds) | 99th percentile latency (milliseconds) |
| --- | --- | --- | --- |
| 1 | 67 | 15.4 | 17.6 |
| 5 | 67 | 75.0 | 80.1 |

One client through the router got 67 requests per second against 71.8 for
an MS/TP client on the line itself, so the B/IP side and the router add
about 1.4 milliseconds to a request that the serial line already holds for
14. More clients only queue: the line carries one request per token
rotation, and each client's latency grows with the queue.

## 6. Scale and stress

The `stress-test` binary runs these. All are on B/IP over loopback, in one
process, with clients doing requests one after another as fast as answers
come back; `stress-test clients` can also run over B/IPv6 or BACnet/SC.
Resident memory is the whole process's, read from the operating system, so
it includes the clients.

Each scenario prints one line per step, then a table of every step and a
summary of all of them: the requests that succeeded and failed in all, the
rate over all the steps' measured time, and the fastest step. Its JSON
output holds the same, with each step's counts and measured time in the
degradation curve. Before
[#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714) the
summary came from the last step alone, and its count of successful
requests was that step's rate times 10 seconds, whatever the duration; the
per-step figures below were not affected.

### 6.1 Concurrent clients

`stress-test clients`: N clients reading one Analog Input's Present_Value
from a server with 100 Analog Inputs, 10 seconds per step. Every client
connects before a step's clock starts and keeps its own latency record.
`--transport bip6` runs the same over B/IPv6 on the IPv6 loopback address,
and `--transport sc` over BACnet/SC with mutual TLS, every client and the
server connected to a hub in the same process.

| Concurrent clients | B/IP: requests per second | B/IP: median latency (microseconds) | B/IP: 99th percentile latency (microseconds) | B/IPv6: requests per second | B/IPv6: median latency (microseconds) | BACnet/SC: requests per second | BACnet/SC: median latency (microseconds) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 43,594 | 21 | 34 | 39,120 | 25 | 22,344 | 43 |
| 5 | 81,012 | 59 | 109 | 93,581 | 51 | 64,610 | 75 |
| 10 | 82,229 | 117 | 227 | 101,385 | 96 | 77,422 | 126 |
| 25 | 70,966 | 352 | 589 | 91,842 | 270 | 88,105 | 281 |
| 50 | 65,182 | 779 | 1,297 | 79,413 | 637 | 92,910 | 535 |
| 100 | 52,598 | 1,077 | 2,012 | 65,660 | 899 | 68,653 | 797 |

Throughput peaks at 5 to 10 clients on B/IP and B/IPv6 and falls by 20% to
35% from there to 100; beyond the peak, more clients only lengthen the
queue, and latency grows in proportion. BACnet/SC peaks later, at 50
clients. No request failed up to 50 clients. At 100 clients some timed
out: 3,355 of 638,181 on B/IP (0.5%), 2,153 on B/IPv6 (0.3%) and 289 on
BACnet/SC (0.03%), and each 100-client step ran about 2 seconds past its
10 seconds while the last requests waited for their timeouts. In 0.13.0,
380 of 672,778 B/IP requests (0.06%) timed out at 100 clients. The `dev`
branch before #1713's last cuts also timed out 1,700 to 3,200 requests at
that step with the current harness, so this pass did not cause it;
[section 10](#10-guidance-for-future-work) lists it. Peak resident memory
was 49 megabytes on B/IP.

### 6.2 Object database size

`stress-test objects`: one client, 5 seconds of ReadProperty of random
Analog Inputs, then ReadPropertyMultiple requests of 10 random objects (one
per 10 objects in the database, at most 100), at each database size.

| Objects | ReadProperty requests per second | ReadProperty median latency (microseconds) | ReadProperty 99th percentile latency (microseconds) | ReadPropertyMultiple of 10, median latency (microseconds) | Resident memory after the step (megabytes) |
| --- | --- | --- | --- | --- | --- |
| 100 | 38,042 | 24 | 39 | 36 | 14.2 |
| 500 | 38,459 | 24 | 38 | 33 | 15.7 |
| 1,000 | 38,527 | 24 | 38 | 31 | 16.3 |
| 2,500 | 38,043 | 24 | 39 | 30 | 18.0 |
| 5,000 | 38,351 | 24 | 38 | 31 | 22.0 |
| 10,000 | 38,152 | 24 | 38 | 30 | 26.9 |

Lookups are indexed, so latency does not change with the database size.
From 100 to 10,000 objects resident memory grew by 12.6 megabytes, about
1.3 kilobytes per Analog Input including the allocator's overhead. 10,000
objects is the server's default object limit.

### 6.3 COV subscriptions

`stress-test cov`: each step's N subscriptions to Analog Outputs come from
as many subscriber clients as the per-subscriber limit of 64 needs, each
taking Analog Outputs 1 to 64 (or fewer), so 1,024 subscriptions come from
16 subscribers. A writer client then writes their Present_Values in turn at
10 writes per second for 10 seconds, and each write notifies every
subscriber of that object. `--confirmed` asks for confirmed notifications,
which each subscriber acknowledges.

| Subscriptions | Subscribers | Accepted | Writes | Unconfirmed notifications received / expected | Confirmed notifications received / expected |
| --- | --- | --- | --- | --- | --- |
| 1 | 1 | 1 | 97 to 98 | 98 / 98 | 99 / 99 |
| 10 | 1 | 10 | 98 | 108 / 108 | 108 / 108 |
| 64 | 1 | 64 | 98 | 162 / 162 | 162 / 162 |
| 256 | 4 | 256 | 97 to 98 | 644 / 644 | 648 / 648 |
| 1,024 | 16 | 1,024 | 98 | 2,592 / 2,592 | 2,592 / 2,592 |

Expected notifications are one initial notification per accepted
subscription plus one per write for every subscriber of the written
object. Every subscription was accepted and every expected notification
arrived, confirmed or not, up to the server's total of 1,024. The server
admits at most 64 subscriptions from one subscriber
(`CovPolicy::max_subscriptions_per_peer`); a single subscriber asking for
more is refused the rest with `RESOURCES` / `NO_SPACE_TO_ADD_LIST_ELEMENT`.

### 6.4 Segmentation

`stress-test segmentation`: ReadPropertyMultiple of N Present_Values, 50
requests at each size. The server and its Device support segmentation in
both directions (the 0.13.0 edition fixed the scenario, which before never
enabled it, so a reply too large for one APDU was refused).

| Objects per request | Reply size and segments | Median latency (microseconds) | 99th percentile latency (microseconds) | Failed requests |
| --- | --- | --- | --- | --- |
| 10 | about 160 bytes, one APDU | 84 | 1,479 | 0 |
| 50 | about 800 bytes, one APDU | 99 | 1,175 | 0 |
| 100 | about 1.6 kilobytes, 2 segments | 183 | 486 | 0 |
| 250 | about 4 kilobytes, 3 segments | 342 | 1,496 | 0 |

Latency grows with the reply's size and its segments: 250 objects in 3
segments take about three and a half times as long as 50 in one APDU. With only 50
requests per size, the 99th percentile is the slowest one or two requests
(typically the first), not a steady-state figure.

A ReadPropertyMultiple of 500 objects was refused with an Abort (out of
resources): by default a server answers at most 256 results or 16,384
bytes in one ReadPropertyMultiple (`ReadPropertyMultipleBudget`).

### 6.5 Mixed workload

`stress-test mixed`: 10 clients for 30 seconds, each request picked at
random: 60% ReadProperty, 15% WriteProperty, 10% ReadPropertyMultiple of 5
objects, 5% a COV subscribe and unsubscribe, 5% Who-Is, and 5%
ReadProperty of the Device. The server holds 550 objects.

| Clients | Operations per second | Median latency (microseconds) | 99th percentile latency (microseconds) | Failed operations | Peak resident memory (megabytes) |
| --- | --- | --- | --- | --- | --- |
| 10 | 85,424 | 117 | 226 | 0 | 19.9 |

### 6.6 BBMD foreign devices

This section was not re-run for this edition: its capacity scenario needs
Linux (see below). The table below is from the scenario's earlier form: a BBMD, which relays
broadcasts for B/IP devices, with N foreign devices registered in its
Foreign Device Table (FDT), all on loopback. The foreign devices took
turns sending a broadcast Who-Is through the BBMD for 10 seconds, with a
100-microsecond pause between sends, and the next device waited up to 10
milliseconds for the forwarded copy.

| Foreign devices requested | Registered in the FDT | Broadcasts sent per second | Median send time (microseconds) | Packets forwarded | Packets held back by the fan-out limit | Failed sends |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 741 | 13 | 0 | 0 | 0 |
| 5 | 5 | 533 | 14 | 5,120 | 16,208 | 0 |
| 10 | 10 | 532 | 36 | 5,120 | 42,805 | 0 |
| 25 | 16 | 174 | 38 | 5,120 | 13,376 | 270 |

The scenario's own pacing sets its send rate, so the rate is not a
capacity figure. What it shows is that the BBMD forwards broadcasts to
every registered foreign device and that its fan-out limits engage: the
scenario configures at most 512 forwarded packets per second from one
origin, so the BBMD holds the rest back rather than flooding the network.

Only 16 of 25 foreign devices registered: the BBMD admits at most 16
management requests per source address per second, a fixed guard against
floods, and all 25 devices share the address 127.0.0.1 and register within
the same second. On a real network each foreign device has its own
address.

`stress-test bbmd` now measures capacity. Every foreign device sends
broadcasts through the BBMD as fast as its socket takes them, with no
pause and no waiting, for the step's duration, and the BBMD's fan-out
limits are set far above what loopback carries. The step's rate is the
packets the BBMD forwarded per second; it also reports the broadcasts
offered, the copies delivered to the foreign devices, and the forwarding
jobs dropped when the BBMD's queue was full, which is where its capacity
runs out. On Linux each foreign device binds its own loopback address
(127.1.0.1, 127.1.0.2 and so on), so the registration guard no longer caps
the count; on macOS they still share 127.0.0.1. Run it in the Docker
orchestrator container ([section 7.3](#73-reproducing-the-docker-run)) for
the published figure, and replace this table at the next refresh. A short
trial run on macOS (2 seconds a step, on a loaded machine) forwarded about
30,000 packets per second with 5 foreign devices; in the Linux container
all 100 of 100 foreign devices registered. The 1-device step always
forwards nothing: a BBMD doesn't send a broadcast back to the foreign
device it came from, and there is no other.

### 6.7 Device scan

`stress-test device-scan` (named `whois` before
[#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714)): N
servers on loopback, each its own device; one client reads every Device's
Object_Name in turn, as a supervisory tool does once it knows the devices.
It does not broadcast a Who-Is: on loopback each server has its own UDP
port, so a broadcast would reach only one of them. The Docker
orchestrator's discovery scenario
([section 7.1](#71-cross-network-orchestrator)) measures Who-Is discovery.

| Devices | Devices read | Time to read all (milliseconds) | Median read time (microseconds) | 99th percentile read time (microseconds) |
| --- | --- | --- | --- | --- |
| 5 | 5 | 0.6 | 90 | 281 |
| 10 | 10 | 1.9 | 66 | 1,250 |
| 25 | 25 | 3.1 | 90 | 1,132 |
| 50 | 50 | 5.7 | 92 | 1,211 |

Every device answered. The first request to each new server is slower than
the steady-state requests of [section 6.1](#61-concurrent-clients), and the
99th percentile is the slowest one or two of them. The scenario's rate,
devices read per second, now divides by the scan's measured time at full
resolution; it used to divide by whole milliseconds, which meant nothing
for a scan of under a millisecond. It has no duration option: each step
is one pass over the devices.

## 7. Docker cross-network results

This section is the 0.13.0 run of 2026-10-13, not re-run for this
edition; its macOS columns are that run's.

`examples/docker` runs the stack as separate Linux containers on Docker
bridge networks, so packets cross real (virtual) network interfaces instead
of loopback. Docker Desktop runs the containers in a Linux virtual machine
with 4 processors on the same laptop. The images are Alpine Linux with
statically linked release binaries. The BACnet/SC pair needs credentials:
this run used the development-only recipe in `examples/docker/README.md`,
with the files kept outside the checkout and deleted afterwards.

The topology: B/IP network 1 (172.20.0.0/24) with `server-a` (1,000
objects) and `bbmd-a`; B/IP network 2 (172.20.1.0/24) with `server-b`
(1,000 objects) and `bbmd-b`; the `router` service (`BACnetRouter`) joining
networks 1 and 2; a BACnet/SC hub and device; a foreign-device network
(172.20.4.0/24), which the `ip-router` service joins to network 1 as a
site's IP router would; and a discovery network (172.20.5.0/24) holding
the `fleet` service, 20 copies of one device by default, each with its own
address and Device instance. The orchestrator container sits on networks
1 and 2, the BACnet/SC network and the discovery network, and drives the
tests; the `foreign-bench` container sits on the foreign network alone and
runs the foreign-device scenario. It is in the compose file's `foreign`
profile, so a plain `docker compose up` does not run it alongside the
orchestrator. Docker does not route IP between its
bridge networks, so the hosts on the foreign device's path add routes
through `ip-router` when they start, which needs the `NET_ADMIN`
capability.

### 7.1 Cross-network orchestrator

`stress-orchestrator --duration 10`: one client doing ReadProperty requests
one after another for 10 seconds. Its scenarios are `same-subnet`,
`router`, `discovery` and `foreign`; `--only` and `--skip` pick them, and
without `--only` every one but `foreign` runs. The summary covers every
scenario it ran; it used to report the requests as each path's rate times
the duration, and none as failed.

| Path | Requests per second | Median latency (microseconds) | 99th percentile latency (microseconds) | Failed requests |
| --- | --- | --- | --- | --- |
| Same network: orchestrator to `server-a` | 27,944 | 30 | 89 | 0 |
| Routed: orchestrator on network 1, through `router`, to `server-b` on network 2 | 20,916 | 43 | 120 | 0 |

The router hop added 13 microseconds to the median, close to the 18
measured on macOS loopback in [section 5](#5-routing). An earlier run of
the same-network path, before the fix below, gave 34,109 requests per
second at a median of 27 microseconds: the virtual machine's numbers vary
by about 20% between runs.

Two fixes were needed for this run, both in the benchmark harness, not the
stack:

- The orchestrator's "router hop" sent its requests straight to
  `server-b`'s IP address and relied on IP routing between Docker bridge
  networks, which Docker blocks, so every request timed out and no BACnet
  routing was involved. It now sends them to the `router` service with
  the destination network and address in the NPDU.
- The `bacnet-router` binary bound each port's B/IP transport to the
  wildcard address. Both ports use UDP port 47808, so on Linux one socket
  received every port's unicast and the router could not tell the networks
  apart. It now binds each port to its own address
  (`BipTransport::set_share_port_by_address`).

This run had no foreign-device or discovery figure. The foreign-device
scenario then never registered with a BBMD and, like the old router hop,
depended on IP routing between bridge networks, so every request timed
out. Since [#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714)
two scenarios cover them; measure both at the next refresh:

- **`foreign`**, run by the `foreign-bench` service from the foreign
  network: a client registers as a foreign device with `bbmd-a`, finds
  `server-a` with a Who-Is that `bbmd-a` distributes on network 1, then
  reads from `server-a` for the duration. Only the Who-Is goes through
  the BBMD: by default a server sends its I-Am straight back to the
  device that asked (`DiscoveryPolicy::prefer_directed_responses`), so
  the I-Am and the reads are unicast over `ip-router`. It reports the
  time to register and to discover, and the reads' rate and latency.
- **`discovery`**: the orchestrator broadcasts a Who-Is on the discovery
  network and times how long until every device of the fleet has
  answered with an I-Am, round after round for the duration. A round in
  which some device does not answer within 2 seconds counts as failed.
  `--fleet` tells it how many devices to expect (the fleet's replicas).
  Rounds start 250 milliseconds apart, off the clock: by default a server
  ignores a repeat of the same Who-Is from one source within 200
  milliseconds (`DiscoveryPolicy::coalesce_window`).

A short trial of both (2 seconds each, on a heavily loaded machine) found
`server-a` through the BBMD 5 milliseconds after registering, and read
from it with no failures; and all 20 devices of the fleet answered in
every one of 9 discovery rounds.

### 7.2 Stress scenarios inside a container

The same `stress-test` scenarios as [section 6](#6-scale-and-stress), run
inside one orchestrator container, over the container's loopback interface.
This compares the Linux network stack with macOS's on the same hardware.

| Scenario | Linux container: requests per second | Linux container: median latency (microseconds) | macOS host: requests per second | macOS host: median latency (microseconds) |
| --- | --- | --- | --- | --- |
| 1 client | 45,373 | 21 | 30,502 | 32 |
| 5 clients | 117,006 | 40 | 70,884 | 68 |
| 10 clients | 88,937 | 102 | 73,750 | 132 |
| 25 clients | 104,914 | 230 | 69,895 | 353 |
| 50 clients | 110,152 | 416 | 65,016 | 749 |
| Mixed workload, 10 clients (10 seconds in the container, 30 on the host) | 99,202 | 99 | 76,769 | 130 |
| Routed B/IP to B/IP, 1 client | 38,526 | 24 | 19,366 | 49 |
| Routed B/IP to B/IP, 25 clients | 81,119 | 302 | 55,874 | 435 |

| MS/TP small reply, baud rate | Linux container: requests per second | Linux container: share of the wire limit | macOS host: requests per second | macOS host: share of the wire limit |
| --- | --- | --- | --- | --- |
| 9,600 | 10.2 | 89% | 10.4 | 91% |
| 38,400 | 32.4 | 71% | 34.4 | 75% |
| 76,800 | 47.1 | 51% | 53.2 | 58% |
| 115,200 | 63.8 | 47% | 72.7 | 53% |

The Linux container answered 20% to 70% more requests than macOS on the
same machine with 4 of its 10 processors (about 50% in most rows), and the
routed path gained most, up to twice the rate:
the per-datagram cost on macOS is higher than on Linux. A Linux server is
the better guide to production. MS/TP, which is bound by timers rather than
datagrams, ran a little slower in the virtual machine. Peak resident memory
in the container was 9 to 10 megabytes per scenario.

### 7.3 Reproducing the Docker run

Provision the BACnet/SC credentials first, as `examples/docker/README.md`
describes, then, from `examples/docker`:

```bash
export SC_CERT_DIR=/absolute/path/outside/the/checkout
export COMPOSE_PROJECT_NAME=rusty-bench
# --profile foreign includes foreign-bench, which a plain build or up leaves out.
docker compose --profile foreign build
# Everything but the two runners, whose own commands would run alongside yours.
docker compose up -d server-a bbmd-a server-b bbmd-b router sc-hub sc-server \
  ip-router fleet
docker compose run --rm -T --no-deps orchestrator \
  stress-orchestrator --wait-for-ready=20 --duration 10 --fleet 20
docker compose run --rm -T --no-deps foreign-bench \
  stress-orchestrator --wait-for-ready=5 --duration 10 --only foreign
docker compose run --rm -T --no-deps orchestrator \
  stress-test clients --steps 1,5,10,25,50 --duration 10
# BBMD capacity, one loopback address per foreign device.
docker compose run --rm -T --no-deps orchestrator \
  stress-test bbmd --steps 1,5,10,25,50,100 --duration 10
docker compose down -v --rmi local
```

For a larger fleet, start it with `--scale fleet=50` and pass the
orchestrator `--fleet 50`. `stress-test clients --transport bip6` needs an
IPv6 loopback address, which Docker containers lack unless IPv6 is enabled
for them, so run it on the host. Without BACnet/SC credentials, leave `sc-hub`
and `sc-server` out of the list.

## 8. Embedded footprint

The portable crates build for microcontrollers with no operating system.
`firmware/link` links one small image per stack for four targets, with the
size-optimized `firmware` profile, and `scripts/ci/check-firmware-link.sh`
checks each against `firmware/link/size-budget.toml`. These sizes were
measured for this report with that script (raw output:
`firmware-link.txt`); all images were within their budgets.

The images:

- `mstp`: an MS/TP master node (the frame codec and the Clause 9 master
  core) with standard frames.
- `mstp-extended`: the same with extended frames, for NPDUs up to 1,497
  bytes.
- `bip`: the B/IP and B/IPv6 BVLL codecs.
- `node`: a B/IP foreign-device node.
- `bbmd`: a BBMD's broadcast and foreign device tables.
- `router`: a two-port router, a BBMD's B/IP port to an MS/TP port with
  extended frames.
- `apdu`: the application layer encoder and decoders, with a 1-kilobyte
  heap; the only image with an allocator.
- `device`: a whole MS/TP device with no heap: the data link, the
  network layer and an application layer that answers Who-Is and
  ReadProperty.

Flash is code, read-only data and the vector table. RAM is static RAM
(initialized and zeroed data, the heap arena included), not the stack.
Sizes in bytes.

| Image | Cortex-M0+ (`thumbv6m-none-eabi`) flash | Cortex-M3 (`thumbv7m-none-eabi`) flash | Cortex-M4F (`thumbv7em-none-eabihf`) flash | RISC-V (`riscv32imc-unknown-none-elf`) flash | Static RAM, all targets |
| --- | --- | --- | --- | --- | --- |
| `mstp` | 18,388 | 17,928 | 17,812 | 19,450 | 5,528 |
| `mstp-extended` | 17,220 | 16,880 | 16,792 | 18,302 | 7,696 |
| `bip` | 11,640 | 10,940 | 10,872 | 13,366 | 132 |
| `node` | 15,268 | 14,576 | 14,564 | 16,118 | 240 |
| `bbmd` | 13,288 | 12,056 | 12,000 | 14,274 | 240 |
| `router` | 45,576 | 43,020 | 42,812 | 49,078 | 15,624 |
| `device` | 30,872 | 30,140 | 30,096 | 33,822 | 3,496 |
| `apdu` | 43,620 | 42,724 | 42,504 | 50,038 | 1,056 |

The deepest stack each image reached, measured under QEMU emulation by the
manual check `scripts/emu/emulate-firmware.sh` (Rust 1.99.0, QEMU 10.0.13;
not re-run for this report; see `docs/ci.md`, "Emulation (manual)", and
`firmware/link/stack-budget.toml`). The `node`, `bbmd` and `router`
columns were measured again after their ports changed (#1774). Sizes in
bytes.

| Board (target) | `mstp` | `mstp-extended` | `bip` | `node` | `bbmd` | `router` | `apdu` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| micro:bit (`thumbv6m-none-eabi`) | 2,040 | 3,020 | 2,220 | 5,444 | 2,380 | does not fit | 2,768 |
| LM3S6965 (`thumbv7m-none-eabi`) | 1,824 | 2,868 | 2,100 | 4,972 | 2,260 | 4,456 | 2,536 |
| Netduino Plus 2 (`thumbv7em-none-eabihf`) | 1,832 | 2,868 | 2,100 | 4,972 | 2,260 | 4,456 | 2,536 |
| SiFive E (`riscv32imc-unknown-none-elf`) | 1,712 | 2,732 | 2,020 | 4,808 | 2,132 | does not fit | 2,464 |

The `device` image's deepest stack, measured when it joined (#1771) with
the same toolchain and QEMU, was 2,652 bytes on the micro:bit board and
2,284 on the LM3S6965.

What an embedded project can expect:

- An MS/TP device needs about 18 kilobytes of flash and 7.6 kilobytes of
  RAM (5.5 static, 2 of stack) for the data link; with extended frames,
  about 10.7 kilobytes of RAM.
- The application layer image (`apdu`) is about 43 kilobytes of flash on
  its own, plus its heap.
- A whole MS/TP device that answers Who-Is and ReadProperty without a
  heap (`device`) needs about 30 kilobytes of flash and 6.2 kilobytes of
  RAM (3.5 static, 2.7 of stack). Without the heap, the application layer
  links no formatting code, and the device is smaller in flash than the
  `apdu` image alone.
- The router needs about 20 kilobytes of RAM (15.5 static, 4.5 of stack),
  so it does not fit the 16-kilobyte boards; most of its static RAM is the
  extended-frame buffers an MS/TP router port requires.
- Each image links its own test scenario as well as the stack, so compare
  images with care: `mstp-extended` is smaller in flash than `mstp`
  because their scenarios differ.

## 9. Python bindings

The `rusty-bacnet` package wraps the same Rust client and server for
Python's asyncio. `benchmarks/python` holds four pytest suites, run here on
CPython 3.13.15 against a release build of the package (built with
`maturin develop --release`), over loopback. Each measures 100 or 200
requests, one after another, after a warm-up; the concurrency tests run
50 requests per concurrent task, with that many in flight at once from one
client. These are single runs of small samples.

The two suites are `bench_bip.py`, with client and server both from
Python over B/IP, and `bench_sc.py`, over BACnet/SC with mutual TLS through
a hub. Their concurrency tests keep each client at or under the server's
limit of 16 confirmed requests in flight from one peer: `bench_bip.py` runs
1, 5, 10, 16, 32 and 64 requests in flight, spread over 2 and 4 clients for
the last two, and `bench_sc.py` runs 1, 5, 10 and 16 from one client.

| Suite | ReadProperty median (microseconds) | WriteProperty median (microseconds) | ReadPropertyMultiple of 3 objects and 2 properties, median (microseconds) | COV subscribe and cancel, median (microseconds) |
| --- | --- | --- | --- | --- |
| `bench_bip.py` (B/IP) | 98 | 89 | 71 | 144 |
| `bench_sc.py` (BACnet/SC with mutual TLS, through a hub) | 155 | 91 | 87 | 179 |

| Requests in flight | `bench_bip.py`: requests per second | `bench_sc.py`: requests per second |
| --- | --- | --- |
| 1 | 5,586 | 9,378 |
| 5 | 17,940 | 27,927 |
| 10 | 33,951 | 29,512 |
| 16 | 36,446 | 37,145 |
| 32, over 2 clients | 37,288 | not run |
| 64, over 4 clients | 38,016 | not run |

What it shows:

- A request from Python costs 70 to 100 microseconds, against 29 from Rust
  ([section 4](#4-transports)): the asyncio event loop and the crossing
  between Python and Rust add 40 to 70 microseconds per call.
- Python reaches 34,000 requests per second with 10 requests in flight on
  B/IP and 36,000 to 38,000 from 16 on, about half of what Rust clients
  reach on the same machine. In 0.13.0 the B/IP suites reached 23,000 to
  24,000 with 10 in flight.
- No request failed or stalled: every test kept within the server's limit
  of 16 confirmed requests in flight per peer, and BACnet/SC with 10 or 16
  in flight ran at 30,000 to 37,000 requests per second, its slowest
  request under 1.5 milliseconds. Before
  [#1712](https://github.com/jscott3201/rusty-bacnet/issues/1712), a
  BACnet/SC node's receive queue admitted at most 4 messages from one peer
  and silently dropped the rest of a burst, so 10 requests in flight from
  one Python client stalled on timeouts; the quota is now 32.
- The suites' Who-Is test waits a fixed 300 milliseconds for answers, so
  its 302-millisecond time measures the wait, not the stack; the test is
  named `test_bip_whois_fixed_wait` to say so.

To run them:

```bash
uv venv .venv --python 3.13 && source .venv/bin/activate
uv pip install pytest pytest-benchmark pytest-asyncio psutil maturin
maturin develop --release -m crates/rusty-bacnet/Cargo.toml
cd benchmarks/python
python -m pytest bench_bip.py -s -v   # B/IP, client and server from Python
python -m pytest bench_sc.py -s -v    # BACnet/SC through a hub
```

## 10. Guidance for future work

Where the time goes, and what to measure next.

**Bottlenecks seen in this run.**

- **The IP round trip is the stack, not the encoding.** Encoding and
  decoding a request take under 0.1 microseconds of a 25- to
  29-microsecond round trip. The profile of
  [#1713](https://github.com/jscott3201/rusty-bacnet/issues/1713) puts most
  of the rest in the allocator, Tokio's scheduler and I/O driver, the
  operating system's UDP path, and locks; its remaining items, in order of
  cost, are filed for 0.15.0: copying MAC addresses, Tokio lock
  round trips on the client's transaction table and the server's database,
  encoding the ReadProperty answer in place, the second box around the
  other services' handlers, a buffer per B/IP send, and clock reads.
- **macOS's datagram path is slow; Linux is the better guide.** The same
  binaries in a Linux container on 4 of the laptop's processors answered
  20% to 70% more requests than on macOS in the 0.13.0 run, and up to twice
  as many routed ([section 7.2](#72-stress-scenarios-inside-a-container)).
  Performance work on the IP transports should be measured on Linux.
- **The IP round trip is still slower than in the 0.8.0 report.** That
  edition measured a B/IP ReadProperty at 20.9 microseconds on a machine of
  the same model, under an older operating system and compiler; this run
  measured 28.7. An A/B of 0.8.0 against the current branch on one machine
  would show how much of the gap is left.
- **Two encoding figures grew since 0.13.0.** Decoding an NPDU of 100
  bytes went from 11.2 to 15.6 nanoseconds and building a whole
  ReadProperty request from 68.7 to 82.1. Both are small next to the round
  trip, but a bisect would find the change.
- **Timeouts at 100 clients.** At 100 concurrent B/IP clients 0.5% of the
  requests timed out, against 0.06% in 0.13.0, and the branch did the same
  before #1713's last cuts. Under the same load B/IPv6 lost 0.3% and
  BACnet/SC 0.03%. Profiling that step would show whether the server's
  admission, the transport queue or the clients' timers are the limit.
- **Concurrency peaks early.** One server peaks at 5 to 10 clients over
  B/IP and B/IPv6 and loses up to a third beyond that. Profiling the
  server's dispatch under 25 to 100 clients would show whether the limit is
  a lock, the request admission control, or the single UDP socket.
- **MS/TP's host overhead is per frame.** About 1.5 to 2 milliseconds per
  frame of timer and scheduling delay halves small-request throughput at
  115,200 baud. The MS/TP transport's `DedicatedThread` execution mode,
  or finer timers, would be the next thing to measure (see also
  [#501](https://github.com/jscott3201/rusty-bacnet/issues/501), a
  deterministic Linux execution mode).
- **The router hop costs about 21 microseconds**, now nearly as much as a
  direct request. Most of it is two more datagrams; the router's own ports
  still hand each packet between tasks, which a profile of its per-packet
  path would weigh.

**What to measure next.**

- The MS/TP scenario on real RS-485 hardware with a USB adapter, to compare
  with the simulated line.
- Stack peaks re-measured under emulation after the next toolchain bump.

**Not re-run for this edition.** The Docker run
([section 7](#7-docker-cross-network-results)), with the `foreign` and
`discovery` scenarios of
[#1714](https://github.com/jscott3201/rusty-bacnet/issues/1714) still to
add, and the BBMD capacity scenario of
[section 6.6](#66-bbmd-foreign-devices), which needs the Linux container's
one address per foreign device.

## 11. How to reproduce

Run one suite at a time on an idle machine. Each Criterion suite takes 1 to
2 minutes, each stress scenario 10 seconds per step. For an A/B against
0.13.0, build tag `v0.13.0` with the `benchmarks/` directory and
`Cargo.lock` of commit `deaeb745` (its report's harness) in a separate
checkout with its own target directory, and alternate runs of the two
`stress-test` binaries.

```bash
# Criterion suites
cargo bench -p bacnet-benchmarks --bench encoding
cargo bench -p bacnet-benchmarks --bench codecs
cargo bench -p bacnet-benchmarks --bench read_range
cargo bench -p bacnet-benchmarks --bench bip_latency
cargo bench -p bacnet-benchmarks --bench bip_throughput
cargo bench -p bacnet-benchmarks --bench bip6_latency
cargo bench -p bacnet-benchmarks --bench bip6_throughput
cargo bench -p bacnet-benchmarks --bench sc_mtls_latency
cargo bench -p bacnet-benchmarks --bench sc_mtls_throughput

# Stress scenarios (build once, then run the binary)
cargo build --release -p bacnet-benchmarks --bins
ST=./target/release/stress-test
$ST clients --steps 1,5,10,25,50,100 --duration 10
$ST clients --transport bip6 --steps 1,5,10,25,50,100 --duration 10
$ST clients --transport sc --steps 1,5,10,25,50,100 --duration 10
$ST objects --steps 100,500,1000,2500,5000,10000 --duration 5
$ST cov --steps 1,10,64,256,1024 --duration 10
$ST cov --steps 1,10,64,256,1024 --duration 10 --confirmed
$ST segmentation --steps 10,50,100,250
$ST mixed --clients 10 --duration 30
$ST router --path direct --steps 1,5,10,25,50 --duration 10
$ST router --path bip --steps 1,5,10,25,50 --duration 10
$ST router --path mstp --baud 115200 --steps 1,5 --duration 10
$ST mstp --reply small --duration 10
$ST mstp --reply standard --duration 10
$ST mstp --reply extended --duration 20
$ST bbmd --steps 1,5,10,25,50,100 --duration 10   # on Linux; see section 7.3
$ST device-scan --steps 5,10,25,50

# Embedded image sizes (needs the four targets: rustup target add ...)
bash scripts/ci/check-firmware-link.sh
```

The Docker and Python steps are in their own sections above.
`scripts/bench-local.sh` and `scripts/bench-ab.sh` wrap these suites and
compare two revisions; their output goes to `bench-output/`, which git
ignores. Results promoted into a tracked history should follow
`benchmarks/schema/benchmark-result.schema.json`.
