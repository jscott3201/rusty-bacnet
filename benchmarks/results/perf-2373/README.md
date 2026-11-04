# B/IP receive without a clock read (#2373)

Part of [#1978](https://github.com/jscott3201/rusty-bacnet/issues/1978):
[#2373](https://github.com/jscott3201/rusty-bacnet/issues/2373), a
follow-up to S4's item 6 ([perf-s4](../perf-s4/README.md)), counted with
the counting harness (Benchmarks.md section 10.4). Section 10.10 there
summarizes it.

All instruction figures are callgrind instruction reads per request,
without the harness's counting allocator. "Calls" are allocator calls per
request and "bytes" the bytes requested from the allocator per request.
`ct` is Tokio's current-thread runtime, `mt` its multi-thread runtime with
two workers. The floor is the range of the twelve A and A' values (two
clean builds of the baseline) from the same matrix; A and A' were
byte-identical. The run built like the release with the fixed allocator
settings, in local Docker on an Apple M5 laptop, image
`rb-perf-counts:ac28c4cf5d0a` (libc6 2.39-0ubuntu8.9, Valgrind 3.22.0,
Rust 1.99.0), in the order A B A' A' B A three times.

**Baseline.** Dev `dd786122c` (after S5, #2386) against `d6de4282e`, which
is that dev with this change and nothing else.

## Where the cost was

The B/IP receive loop (`bacnet-transport/src/bip/io.rs`, `receive`) read
the node's clock for every datagram and passed the time to
`Link::receive`. The link used it for one thing: to ask
`Policy::admit_management` about one of the four rate-limited management
requests. No policy used it. The core's default ignored it, and the
transport's `HostPolicy` asks the management rate limiter, which reads a
clock of its own. So an ordinary NPDU paid for a clock read it never used:
one per received datagram, two per round trip.

## The change

- `bacnet-bip-core` (`node/receive.rs`): `Link::receive` and
  `Policy::admit_management` take no time. A policy that needs the time
  reads its own clock, so only a management request pays for the read.
  `Node::receive` and the ports' `receive` keep their `now`, which their
  timers and tables use.
- `bacnet-transport` (`bip/io.rs`, `bip/node.rs`): the receive loop no
  longer reads the clock, and `NodeHandle::clock`, which only that read
  used, is gone.

It is a breaking change to `bacnet-bip-core`'s Rust API; the changelog
fragment has the migration note. No other caller read the clock just for
this: the firmware images, the router and the tests call `Node::receive`
or a port's `receive`, or passed `Instant::ZERO`.

## Counts

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Bytes A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- | --- |
| ReadProperty | ct | 31,253.9 | 31,011.0 | -242.9 | 14.2 | 6 of 6 | 18.0001 / 18.0001 | 2,127.01 / 2,127.02 | PASS |
| ReadProperty | mt | 32,554.6 | 32,300.9 | -253.7 | 101.5 | 6 of 6 | 18.0003 / 18.0003 | 2,127.03 / 2,127.03 | FAIL |
| WriteProperty | ct | 39,881.7 | 39,641.6 | -240.1 | 98.8 | 6 of 6 | 25.0002 / 25.0002 | 7,944.02 / 7,944.02 | PASS |
| WriteProperty | mt | 41,076.5 | 40,836.6 | -239.9 | 186.3 | 6 of 6 | 25.0003 / 25.0003 | 7,944.03 / 7,944.03 | PASS |
| ReadPropertyMultiple | ct | 91,106.5 | 90,864.6 | -241.9 | 42.0 | 6 of 6 | 139.0004 / 139.0004 | 18,496.04 / 18,496.04 | PASS |
| ReadPropertyMultiple | mt | 93,102.9 | 92,948.1 | -154.8 | 985.4 | 5 of 6 | 139.0005 / 139.0004 | 18,496.06 / 18,496.04 | PASS |

**Verdict: not accepted by this local count.** ReadProperty falls by
242.9 instructions per request on the current-thread runtime, 17.1 times
its floor, with all pairs lower. On the multi-thread runtime it falls by
253.7, with all six pairs lower (by 205.7 to 268.4), but that is 2.5
times its floor of 101.5, and the rule asks for more than 3. One value
sets that floor: the first baseline run, at 32,629.7 instructions per
request; the other eleven A and A' values lie within 42.6 of each other.
WriteProperty and ten-object ReadPropertyMultiple fall by about 240 on
the current-thread runtime with every pair lower; on the multi-thread
runtime WriteProperty falls by 239.9 with every pair lower, and
ReadPropertyMultiple by 154.8, inside its floor of 985.4. No row rises,
and allocator calls and bytes are unchanged on every row.

## What the profile shows

`count/attribution-rp-ct.md` and `count/attribution-rp-mt.md`, from
`perf_counts.py attribute`:

- **Calls.** Per ReadProperty round trip, on both runtimes, exactly two
  calls fewer each of `HostClock::now`, the transport's Tokio clock
  closure, `Timespec::now`, `Timespec::sub_timespec` and `clock_gettime`:
  the two clock reads, one per received datagram. No other call count
  moves.
- **Instructions.** `Timespec::now`, `sub_timespec`, `clock_gettime` and
  the clock closure fall by 210.2 instructions per request on the
  current-thread runtime and 213.6 on the multi-thread one.
  `HostClock::now` falls by 234.0, and the receive loop's closure
  (`Listeners::recv`) rises by 204.0. In A, callgrind lists the receive
  path's inlined code under the `HostClock::now` symbol: of its 234.0
  instructions per request, 96 come from `bacnet-bip-core`'s
  `node/receive.rs`, 40 from its `node/addressing.rs`, 20 from the BVLL
  codec and 38 from the lock the link is read under, and only 18 from
  `bacnet-time`. In B that code sits in the receive closure. So the two
  rows are mostly the same instructions under another name, and the net,
  241.0 and 249.1 instructions per request over the functions that moved,
  is the clock reads.

That is about 120 instructions per received datagram, more than the
issue's estimate of about 90, as the transport's clock goes through
Tokio's clock and the conversion to the core's time.

## Firmware

`check-no-std.sh` and `check-firmware-link.sh` pass on `d6de4282e`. The
B/IP images are the same size before (dev `dd786122c`) and after, to the
byte, on every target. Their policy is the core's permissive one, which
never read the time it was handed, so the optimized build had already
left it out. In bytes used against the budget, both before and after:

| Image | Target | Flash | RAM |
| --- | --- | ---: | ---: |
| bip | thumbv7m | 10,940 of 12,256 | 132 of 160 |
| bip | thumbv6m | 11,640 of 13,040 | 132 of 160 |
| bip | riscv32imc | 13,366 of 14,976 | 132 of 160 |
| bip | thumbv7em | 10,872 of 12,176 | 132 of 160 |
| node | thumbv7m | 16,636 of 18,000 | 240 of 272 |
| node | thumbv6m | 17,608 of 19,088 | 240 of 272 |
| node | riscv32imc | 18,838 of 20,096 | 240 of 272 |
| node | thumbv7em | 16,580 of 17,936 | 240 of 272 |
| bbmd | thumbv7m | 12,072 of 12,832 | 240 of 272 |
| bbmd | thumbv6m | 13,312 of 13,872 | 240 of 272 |
| bbmd | riscv32imc | 14,266 of 15,072 | 240 of 272 |
| bbmd | thumbv7em | 12,020 of 12,752 | 240 of 272 |
| router | thumbv7m | 43,272 of 47,024 | 15,632 of 17,376 |
| router | thumbv6m | 45,836 of 49,056 | 15,632 of 17,376 |
| router | riscv32imc | 49,266 of 52,960 | 15,632 of 17,376 |
| router | thumbv7em | 43,168 of 46,848 | 15,632 of 17,376 |

## Files

- `summary.json`: the comparison's rows (medians, deltas, floors, each
  pair's change, allocation calls and bytes, verdicts) and commits.
- `count/`: `summary.md` and `summary.json` from `perf_counts.py`, the
  #1978 item shares (`items.md`), per-function and per-line slopes
  (`profiles/`) and the `attribute` output for ReadProperty
  (`attribution-rp-ct.md`, `attribution-rp-mt.md`).

Reproduce:

```bash
python3 scripts/perf/perf_counts.py run --baseline dd786122c --candidate d6de4282e \
  --workloads rp,wp,rpm --out /tmp/perf-2373
python3 scripts/perf/perf_counts.py attribute /tmp/perf-2373 --workload rp --runtime mt
```
