# Locks and clocks (S4): the client's async mutexes and segment-expiry clock reads

Part of [#1978](https://github.com/jscott3201/rusty-bacnet/issues/1978):
item 2a (the client's `tokio::sync::Mutex`es) and item 6 (clock reads),
each counted against its own baseline with the counting harness
(Benchmarks.md section 10.4). Section 10.8 there summarizes both.

All instruction figures are callgrind instruction reads per request,
without the harness's counting allocator. "Calls" are allocator calls per
request. `ct` is Tokio's current-thread runtime, `mt` its multi-thread
runtime with two workers. The floor is the range of the twelve A and A'
values (two clean builds of the baseline) from the same matrix; A and A'
were byte-identical in both runs. Each run built like the release with the
fixed allocator settings, in local Docker on an Apple M5 laptop, image
`rb-perf-counts:ac28c4cf5d0a` (libc6 2.39-0ubuntu8.9, Valgrind 3.22.0,
Rust 1.99.0), in the order A B A' A' B A three times.

## Item 2a: blocking client locks (`9b78cb492` against dev `40eeea7d9`)

**Where the cost was.** A noise run on dev with `--callers` put 773
instructions per request of Tokio semaphore work on the client
(`item2a/callers-baseline.md`), the same on every round trip. Call edges
showed four tokio mutex locks per ReadProperty, plus their releases:
- three on the transaction table (TSM): registration, the timer settings
  read, and the terminal completion;
- one on the device table, the peer's Max_APDU lookup.

Under fat LTO, all tokio mutex lock futures fold into one symbol, which the
profile names after the SegmentACK route table.

**The change.** The TSM, the device table and the SegmentACK route table
are now `SyncMutex` (`bacnet-client/src/sync_mutex.rs`): a
`std::sync::Mutex` whose lock ignores poisoning, as the tokio mutex had no
poisoning. No guard of these three is held across an await;
`clippy::await_holding_lock` now enforces that. Four sites held a guard
where an await followed, and were restructured so the guard ends first,
with the same steps still under one lock:
- `requests.rs`, the request timer: the expiry and, for a retry with
  nothing to resend, the cancel happen in one locked block. The two later
  expiry checks become single locked calls.
- `response_admission.rs`, the terminal-response loop: it decides and
  completes under the lock, and only the wait for the final segment's
  send runs without it.
- `segmentation.rs`, segment-zero admission: it checks the owner, saves
  the segment and enters SEGMENTED_CONF under the lock, then sends any
  Abort without it.
- `dispatch.rs`, the SegmentACK route lookup: TSM then routes, in one
  block.

The two `Drop` impls keep `try_lock`, so a drop never waits.

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| ReadProperty | ct | 33,243.5 | 32,400.7 | -842.8 | 15.6 | 6 of 6 | 22.0001 / 22.0001 | PASS |
| ReadProperty | mt | 34,556.8 | 33,709.2 | -847.6 | 80.9 | 6 of 6 | 22.0002 / 22.0002 | PASS |
| WriteProperty | ct | 41,452.0 | 40,625.4 | -826.6 | 12.4 | 6 of 6 | 27.0003 / 27.0003 | PASS |
| WriteProperty | mt | 42,775.0 | 41,915.0 | -859.9 | 234.3 | 6 of 6 | 27.0007 / 27.0004 | PASS |
| ReadPropertyMultiple | ct | 92,119.3 | 91,458.6 | -660.6 | 227.0 | 6 of 6 | 141.0005 / 141.0004 | PASS |
| ReadPropertyMultiple | mt | 94,708.0 | 94,213.2 | -494.8 | 762.5 | 5 of 6 | 141.0007 / 141.0006 | PASS |

**Verdict: accepted.** ReadProperty falls by about 845 instructions per
request: 54.1 times the floor on the current-thread runtime and 10.5 times on the
multi-thread one, with all pairs lower. WriteProperty and
ReadPropertyMultiple fall too. Allocations and requested bytes are
unchanged. In `item2a/profiles/rp-ct.md`:
- the tokio mutex lock future goes, from 4 calls to 0;
- semaphore acquire polls fall from 409 to 81 instructions per request;
- permit returns fall from 686 to 281.

What's left of both is the server's database lock and bounded channels.

## Item 6: clock reads (`6a8bb8a85` against `9b78cb492`)

**Where the reads were.** About 17 clock reads per ReadProperty round trip
(16.6 calls to `Timespec::now`). By caller, in the dev profile:
- about 8 are Tokio's own: the I/O and timer drivers and the request
  timer's `sleep`;
- 2 are the B/IP node core's, one per call into it (`bip/node.rs`);
- about 3 are on the harness's and the client's caller task, inlined
  into the runtime;
- 2 are the server receive loop's segment expiry
  (`server/lifecycle.rs`).

The client's transaction state machine reads the clock only through its
request timer, which needs it. `dispatch.rs`'s read serves only
unconfirmed requests.

**The change.** The server's receive loop read the clock twice per APDU to
expire segmented requests: once before waiting, once after the APDU
arrived. It did this even with no reassembly open, when nothing can expire
and the instant was used for nothing else. It now reads the clock, and
runs the expiry, only while a reassembly is open. With one open, both
reads stay where they were, so no timeout, deadline or order changes.

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| ReadProperty | ct | 32,406.1 | 32,130.5 | -275.6 | 37.6 | 6 of 6 | 22.0002 / 22.0001 | PASS |
| ReadProperty | mt | 33,714.5 | 33,432.1 | -282.4 | 60.2 | 6 of 6 | 22.0002 / 22.0002 | PASS |
| WriteProperty | ct | 40,624.7 | 40,348.8 | -275.9 | 13.3 | 6 of 6 | 27.0002 / 27.0002 | PASS |
| WriteProperty | mt | 41,839.5 | 41,601.0 | -238.5 | 96.7 | 6 of 6 | 27.0003 / 27.0003 | PASS |
| ReadPropertyMultiple | ct | 91,451.6 | 91,181.4 | -270.2 | 33.4 | 6 of 6 | 141.0004 / 141.0004 | PASS |
| ReadPropertyMultiple | mt | 93,683.8 | 93,749.4 | +65.6 | 1,192.4 | 4 of 6 | 141.0005 / 141.0005 | PASS |

**Verdict: accepted.** ReadProperty falls by about 280 instructions per
request: 7.3 times the floor on the current-thread runtime and 4.7 times on the
multi-thread one, with all pairs lower. Two clock reads go (17.0 to 15.0
`clock_gettime` calls), and with them the two expiry futures the loop
built and dropped each turn (`item6/attribution-rp-ct.md`). The
ReadPropertyMultiple multi-thread row is +65.6 against a floor of 1,192.4.

## Files

- `summary.json`: both comparisons' rows (medians, deltas, floors, pairs,
  allocation calls and bytes, verdicts) and commits.
- `item2a/`, `item6/`: each comparison's `summary.md` and `summary.json`
  from `perf_counts.py`, the #1978 item shares (`items.md`) and per-function
  and per-line slopes (`profiles/`). `item2a/callers-baseline.md` is the
  semaphore cost by caller on dev, and `item6/attribution-rp-ct.md` the
  `attribute` output.

Reproduce:

```bash
python3 scripts/perf/perf_counts.py run --baseline 40eeea7d9 --candidate 9b78cb492 \
  --workloads rp,wp,rpm --callers --out /tmp/item2a
python3 scripts/perf/perf_counts.py run --baseline 9b78cb492 --candidate 6a8bb8a85 \
  --workloads rp,wp,rpm --out /tmp/item6
```
