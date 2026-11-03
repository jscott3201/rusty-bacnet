# Request-path copies (S3): MAC address clones and the response route

Part of [#1978](https://github.com/jscott3201/rusty-bacnet/issues/1978),
items 1 and 7, counted one at a time with the counting harness
(Benchmarks.md section 10.4). Item 1 changes how MAC addresses are copied
on the B/IP request path. Item 7 builds the server's response route once
per request instead of twice.

All instruction figures are callgrind instruction reads per request,
without the harness's counting allocator. "Calls" are allocator calls per
request. `ct` is Tokio's current-thread runtime, `mt` its multi-thread
runtime with two workers. The floor is the range of the twelve A and A'
values (two clean builds of the baseline) from the same matrix, and the
A and A' binaries were byte-identical in both runs. Each run built like the
release (fat LTO, one codegen unit) with the fixed allocator settings, in
local Docker on an Apple M5 laptop, image `rb-perf-counts:ac28c4cf5d0a`
(libc6 2.39-0ubuntu8.9, Valgrind 3.22.0, Rust 1.99.0), with the order
A B A' A' B A three times.

## Item 1: MAC address clones (`123414b3b` against dev `a1d54afb8`)

`MacAddr` is `SmallVec<[u8; 6]>`. Its `Clone` goes through the generic
slice conversion and copies one byte at a time; `MacAddr::from_slice`
copies the bytes at once. The baseline profile shows eight calls to
`SmallVec::clone` per ReadProperty round trip, 947 instructions in all.
The change copies with `from_slice` (or borrows) where the request path
cloned:

- client: `TransactionPeer::of`, the TSM registration and its guard, the
  coordinated completion lookups; the dispatch arms move the TSM key into
  their reassembly key and borrow it back instead of cloning it;
- `bacnet-endpoint-core`: `CanonicalPeer` clones by hand (each admitted
  answer clones its lease's peer);
- server: the receive loop's source MAC copy, and `CanonicalRequester`
  clones by hand (request admission clones it per confirmed request).

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| ReadProperty | ct | 33,826.1 | 33,312.4 | -513.8 | 11.8 | 6 of 6 | 22.0001 / 22.0001 | PASS |
| ReadProperty | mt | 35,138.7 | 34,618.1 | -520.6 | 40.4 | 6 of 6 | 22.0002 / 22.0002 | PASS |
| WriteProperty | ct | 42,653.3 | 42,137.4 | -515.8 | 114.0 | 6 of 6 | 27.0003 / 27.0003 | PASS |
| WriteProperty | mt | 43,888.9 | 43,402.5 | -486.4 | 148.6 | 6 of 6 | 27.0003 / 27.0003 | PASS |
| ReadPropertyMultiple | ct | 92,861.4 | 92,350.3 | -511.0 | 39.0 | 6 of 6 | 141.0004 / 141.0004 | PASS |
| ReadPropertyMultiple | mt | 95,296.1 | 94,685.2 | -610.9 | 983.6 | 6 of 6 | 141.0007 / 141.0007 | PASS |

**Verdict: accepted.** ReadProperty falls by about 515 instructions per
request on both runtimes: over 100, 43.6 floors (ct) and 12.9 floors (mt),
with all six pairs lower. WriteProperty and ReadPropertyMultiple fall by the
same amount. Allocation calls and requested bytes are unchanged. All eight
`SmallVec::clone` calls are gone from the ReadProperty and
ReadPropertyMultiple profiles (`item1/profiles/`). This matches the 520
measured on `2d82e9a9` in [1978-macaddr](../1978-macaddr/README.md).

WriteProperty still makes four `SmallVec::clone` calls per request (521
instructions), from `bacnet-objects` value-source tracking
(`ValueSourceTracking::current` and `record`) and the server's
`LifeSafetyCovSnapshots::capture_write_property`. They are outside this
slice's scope.

## Item 7: the response route built twice (`8706ea6b8` against `123414b3b`)

The server's receive loop took a response route, a provenance snapshot and
the source network for every APDU, and `dispatch` then took its own route
from the same envelope. Only segment reassembly reads the loop's copies, so
they are now built in that branch. The loop passes the envelope straight to
`dispatch`; the `Option` wrapper, the `handled` flag and the empty fallback
envelope are gone. A reassembled request moves its saved route in instead of
cloning it.

Judged with the no-regression check (`--aim none`), since the expected
saving is under 100:

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| ReadProperty | ct | 33,311.5 | 33,230.6 | -80.9 | 13.1 | 6 of 6 | 22.0001 / 22.0001 | PASS |
| ReadProperty | mt | 34,623.9 | 34,543.1 | -80.9 | 36.4 | 6 of 6 | 22.0002 / 22.0002 | PASS |
| WriteProperty | ct | 42,135.2 | 42,048.8 | -86.4 | 209.5 | 6 of 6 | 27.0002 / 27.0002 | PASS |
| WriteProperty | mt | 43,376.8 | 43,290.8 | -85.9 | 145.6 | 6 of 6 | 27.0004 / 27.0003 | PASS |
| ReadPropertyMultiple | ct | 92,349.7 | 92,305.8 | -43.9 | 11.0 | 3 of 6 | 141.0005 / 141.0005 | PASS |
| ReadPropertyMultiple | mt | 94,658.2 | 94,628.0 | -30.1 | 507.5 | 2 of 6 | 141.0004 / 141.0004 | PASS |

**Verdict: kept.** No row rises and no allocation calls are added. The
saving, 81 instructions per ReadProperty, is below the 100-instruction
bar for an optimization, as the plan expected. It is kept because it is
also a simplification of the receive loop (net 44 fewer lines). Most of the
saving is in the receive loop itself (-67 self instructions per
ReadProperty, `item7/profiles/rp-ct.md`); the rest is reference-count work
on the route's capabilities.

## Files

- `summary.json`: both comparisons' rows (medians, deltas, floors, pairs,
  allocation calls and bytes, verdicts) and commits.
- `item1/`, `item7/`: each comparison's `summary.md` and `summary.json`
  as written by `perf_counts.py`, the #1978 item shares (`items.md`) and
  per-function and per-line slopes (`profiles/`); `item1/callers.md` is the
  semaphore split by caller.

Reproduce:

```bash
python3 scripts/perf/perf_counts.py run --baseline a1d54afb8 --candidate 123414b3b \
  --workloads rp,wp,rpm --callers --out /tmp/item1
python3 scripts/perf/perf_counts.py run --baseline 123414b3b --candidate 8706ea6b8 \
  --workloads rp,wp,rpm --aim none --out /tmp/item7
```
