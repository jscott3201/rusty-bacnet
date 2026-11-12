# The client's dispatch loop and two write-path clock reads (#1978 items 8 and 6)

Part of [#1978](https://github.com/jscott3201/rusty-bacnet/issues/1978):
item 8 (the client dispatch loop's `select!`), kept, and what was left of
item 6 (clock reads), measured and dropped, each counted against its own
baseline with the counting
harness (Benchmarks.md section 10.4). Section 10.13 there summarizes both.

All instruction figures are callgrind instruction reads per request,
without the harness's counting allocator. "Calls" are allocator calls per
request and "bytes" the bytes requested from the allocator per request.
`ct` is Tokio's current-thread runtime, `mt` its multi-thread runtime with
two workers. The floor is the range of the twelve A and A' values (two
clean builds of the baseline) from the same matrix; A and A' were
byte-identical in every run. Each run built like the release with the
fixed allocator settings, in local Docker on an Apple M5 laptop, image
`rb-perf-counts:ac28c4cf5d0a` (libc6 2.39-0ubuntu8.9, Valgrind 3.22.0,
Rust 1.99.0), in the order A B A' A' B A three times. Two other builds
were running on the laptop throughout; instruction counts don't depend on
that, wall-clock time would.

## What was left

`noise/` is a noise run of dev `41200dd6d`, the base of this work. Its
per-request counts: ReadProperty 30,996.6 and 32,282.5 instructions
(`ct` and `mt`), WriteProperty 39,934.0 and 41,140.9, ten-object
ReadPropertyMultiple 90,458.2 and 93,156.4; 18, 25 and 139 allocator
calls. Its item shares put item 8 at about 795 instructions per
ReadProperty.

- **Item 8.** The client's dispatch loop was a `select!` over four
  sources: the APDU receiver, the network-control queue, the cleanup queue
  of requests given up on, and the five-minute device-purge interval. It
  polled every one on every wake, from a random start, and each source it
  found idle registered the task's waker again: a waker clone, a drop and
  two compare-and-swaps. A ReadProperty round trip wakes the loop twice,
  once for the answer and once to find nothing left.
- **Item 6.** A ReadProperty round trip reads the clock about 12 times.
  One read is the stack's own, the client's request timer, which is the
  deadline itself; there is no other read to share it with. The rest are
  Tokio's: the time driver around each park, the I/O driver's turn and the
  scheduler's busy-time metrics. A WriteProperty made two more reads on the
  server, each used for nothing when its table is empty.

## Item 8: the loop's turn order (`192bc03a9` against dev `41200dd6d`)

The run counted `40044f639`. The branch's `192bc03a9` differs from it in
one doc comment's link, which changes no instruction.

**The change.** No `select!` is left in the loop
(`bacnet-client/src/client/dispatch_turn.rs`, `lifecycle.rs`):
- **Controls first, with no waker.** With `NetworkLayer::start`, a network
  control reaches its queue only while the APDU receiver is polled, which
  only this task does. A non-waiting take before and after that poll finds
  every control there is. The layer's ingress sequence advances for each
  control it admits, so while it stands where the queue was last found
  empty, the loop doesn't read the queue at all.
- **Then APDUs, then cleanups.** A turn that finds an APDU doesn't poll
  the cleanup queue, except that after 16 APDUs in a row the next turn
  polls it first, so a steady stream of APDUs can't hold a cleanup back. A
  turn that finds no APDU polls the cleanup queue, so a turn that returns
  `Pending` has registered with both receivers.
- **The device purge is a task of its own,** aborted on drop and joined by
  `stop()` with the others. It sends its Lost events under the device
  table's lock.

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| ReadProperty | ct | 30,998.6 | 30,229.9 | -768.7 | 9.0 | 6 of 6 | 18.0001 / 18.0001 | PASS |
| ReadProperty | mt | 32,277.3 | 31,510.4 | -766.9 | 14.3 | 6 of 6 | 18.0002 / 18.0002 | PASS |
| WriteProperty | ct | 39,928.5 | 39,167.4 | -761.1 | 97.7 | 6 of 6 | 25.0002 / 25.0002 | PASS |
| WriteProperty | mt | 41,108.4 | 40,339.3 | -769.1 | 53.5 | 6 of 6 | 25.0002 / 25.0002 | PASS |
| ReadPropertyMultiple | ct | 90,443.9 | 90,234.7 | -209.3 | 124.2 | 6 of 6 | 139.0004 / 139.0004 | PASS |
| ReadPropertyMultiple | mt | 93,069.9 | 92,167.3 | -902.6 | 484.0 | 6 of 6 | 139.0004 / 139.0004 | PASS |

**Verdict: accepted.** ReadProperty falls by 768.7 and 766.9 instructions
per request, 85 and 54 times its floors, with every pair lower; every
other row falls too, with every pair lower. Allocator calls and bytes are
unchanged on every row.

**What the profile shows** (`item8/attribution-rp-ct.md`, `-mt.md`). Per
ReadProperty round trip, on both runtimes, 3.5 waker registrations fewer,
each with its waker clone and drop; the purge interval's polls are gone
(3.5 timer polls fewer); the network-control queue's 2.5 receives become 3
reads of an atomic counter; the cleanup queue is popped 2 times instead of
3. The loop's own code falls from 399 to 224 instructions per request.

**ReadPropertyMultiple on the current-thread runtime** falls by only
209.3. The same loop savings, about 740 instructions, are there, but glibc's
allocator spends about 530 more (`_int_malloc`, `_int_free`,
`unlink_chunk`) with the same 139 calls: which free chunk each allocation
finds moved, as one more task now exists from start. That is heap layout,
not more work.

`item8-steps/` splits the change. `order/` is the fixed turn order and the
purge task, with controls taken by a non-waiting receive on every check
(`d16ef2022` against dev): ReadProperty -621.5 and -613.6, WriteProperty
-625.6 and -646.2, ReadPropertyMultiple -64.0 and -331.4, every
ReadProperty pair lower. `sequence/` adds the ingress-sequence check on top
(`ae89b50ed` against `d16ef2022`): ReadProperty -144.7 and -147.0, 23.9
and 5.4 times its floors, WriteProperty -145.5 and -166.2, ten-object
ReadPropertyMultiple -144.9 and -394.9, every ReadProperty pair lower. A
non-waiting receive on an empty bounded queue cost about 59 instructions
there, three times per round trip. The two steps, with a lifecycle test
and a doc comment added, became `40044f639`.

## Item 6: write-path clock reads, measured and dropped (`e721988a1` against `40044f639`)

The candidate, `e721988a1`, is on no branch: the change was dropped, and
the source keeps both reads. It was item 8's `40044f639` plus two guards
in `bacnet-server`:
- `server/device_bindings.rs`, `source_binding`: the instant resolves each
  learned device binding. With none, the loop doesn't run and the answer is
  Unknown, so it now returns that before reading the clock.
- `cov/store_capture.rs`: a timed COV capture compares its instant only
  with subscriptions. With none, no comparison happens and the capture
  keeps no instant, so the guard skipped the clock read.

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| ReadProperty | ct | 30,230.4 | 30,231.6 | 1.2 | 9.4 | 0 of 6 | 18.0001 / 18.0001 | PASS |
| ReadProperty | mt | 31,511.6 | 31,511.9 | 0.3 | 13.7 | 3 of 6 | 18.0002 / 18.0002 | PASS |
| WriteProperty | ct | 39,165.4 | 39,008.6 | -156.9 | 92.1 | 6 of 6 | 25.0002 / 25.0002 | FAIL |
| WriteProperty | mt | 40,359.3 | 40,202.4 | -156.9 | 60.8 | 6 of 6 | 25.0002 / 25.0002 | FAIL |
| ReadPropertyMultiple | ct | 90,235.4 | 90,234.8 | -0.6 | 35.9 | 3 of 6 | 139.0004 / 139.0004 | PASS |
| ReadPropertyMultiple | mt | 92,133.2 | 92,086.4 | -46.8 | 715.8 | 5 of 6 | 139.0004 / 139.0004 | PASS |

**Verdict: dropped, short of the rule.** The run aimed at WriteProperty
(`--aim wp`). WriteProperty fell by 156.9 instructions per request on both
runtimes, with 12 of 12 pairs lower, two clock reads fewer per round trip
and no other call count changed (`item6/attribution-wp-ct.md`). But that
is 1.7 and 2.6 times its floors, under the rule of more than three times
the floor that the counts-first plan of
[#1979](https://github.com/jscott3201/rusty-bacnet/issues/1979) keeps a
change by; item 4 was dropped by the same rule. ReadProperty and
ReadPropertyMultiple don't take this path and didn't move. Allocator calls
were unchanged.

## Files

- `noise/`: the base's noise run (`summary.md`, `summary.json`).
- `item8/`: `summary.md` and `summary.json` from `perf_counts.py`, the
  #1978 item shares (`items.md`), per-function and per-line slopes
  (`profiles/`) and the `attribute` output for ReadProperty.
- `item8-steps/order/`, `item8-steps/sequence/`: the two steps' verdicts.
- `item6/`: the dropped change's verdict, WriteProperty profiles and
  `attribute` output.

Reproduce:

```bash
python3 scripts/perf/perf_counts.py run --noise 41200dd6d --out /tmp/noise
python3 scripts/perf/perf_counts.py run --baseline 41200dd6d --candidate 192bc03a9 --out /tmp/item8
python3 scripts/perf/perf_counts.py attribute /tmp/item8 --workload rp --runtime ct
```
