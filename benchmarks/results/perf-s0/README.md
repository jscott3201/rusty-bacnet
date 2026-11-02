# Counting harness: first baseline and noise floors (S0)

Part of [#1978](https://github.com/jscott3201/rusty-bacnet/issues/1978) and
[#1979](https://github.com/jscott3201/rusty-bacnet/issues/1979). This is the
first run of the counting harness that
[Benchmarks.md section 10.4](../../../Benchmarks.md#104-counting-harness)
describes: two clean builds of one commit (A and A'), measured against each
other, to find how much the counts move when nothing changed. It changes no
production code.

All instruction figures are callgrind instruction reads (Ir) per request, or
per iteration for the codec workloads. "Calls" are allocator calls per
request and "bytes" the bytes those calls requested.

## What was measured

- **Commit:** dev at `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097`, with this
  branch's `benchmarks/counts/` copied into both trees.
- **Image:** `scripts/perf/Dockerfile` (SHA-256 `f1b101ba92d6...`), built as
  `rb-perf-counts:04ede3d38473`: Ubuntu 24.04 arm64, Rust 1.99.0, Valgrind
  3.22.0. Docker 29.9.2 on an Apple M5 laptop.
- **Builds:** each tree in a fresh volume, at the same paths, `--locked`,
  `CARGO_INCREMENTAL=0`. A and A' produced byte-identical binaries in all
  three runs (`counts` SHA-256 `c33f99118823c920...`, `codec-counts`
  `32775c9c26224381...`).
- **Order:** A B B A, three times, for each workload and runtime: six values
  per build, twelve in all. Round trips ran 1,000 and 10,000 requests after
  200 warm-up requests; codec workloads 100,000 and 1,000,000 iterations.
- **Runs:** three invocations of `scripts/perf/perf_counts.py run --noise`
  (`noise-rp/`, `noise-wp-rpm/`, `noise-codec/`); 312 callgrind runs, none
  failed or repeated.

Each directory holds the verdict table (`summary.md`, `summary.json`), every
run's totals (`runs.tsv`), per-function and per-line slopes
(`profiles/`) and, for round trips, the #1978 item table (`items.md`).
`noise-floors.json` collects the floors for `--noise-floors`. The raw
callgrind files (about 560 MB) are not committed.

## Baseline and noise floor

The median and the noise floor (the range of all twelve values) per
workload and runtime. `ct` is Tokio's current-thread runtime, `mt` its
multi-thread runtime with two workers.

| Workload | Runtime | Instructions per request | Noise floor | Floor as a share | Calls | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| ReadProperty (`rp`) | ct | 40,809.7 | 15.5 | 0.04% | 22.0002 | 2,196.02 |
| ReadProperty (`rp`) | mt | 41,687.0 | 57.8 | 0.14% | 22.0003 | 2,196.03 |
| WriteProperty (`wp`) | ct | 49,890.3 | 94.0 | 0.19% | 27.0002 | 8,011.02 |
| WriteProperty (`wp`) | mt | 50,630.5 | 144.5 | 0.29% | 27.0003 | 8,011.03 |
| Ten-object ReadPropertyMultiple (`rpm`) | ct | 115,438.4 | 978.4 | 0.85% | 141.0005 | 18,797.05 |
| Ten-object ReadPropertyMultiple (`rpm`) | mt | 117,977.1 | 664.9 | 0.56% | 141.0006 | 18,797.06 |

| Codec workload | Instructions per iteration | Noise floor | Calls | Bytes |
| --- | --- | --- | --- | --- |
| `encode_read_property_request` | 747 | 0 | 1 | 64 |
| `decode_read_property_request` | 169 | 0 | 0 | 0 |
| `encode_npdu_100b_payload` | 488 | 0 | 1 | 128 |
| `decode_npdu_100b_payload` | 219 | 0 | 0 | 0 |
| `encode_apdu_confirmed_request` | 564 | 0 | 1 | 64 |
| `decode_apdu_confirmed_request` | 239 | 0 | 0 | 0 |
| `full_stack_encode_rp` | 2,367 | 0 | 5 | 242 |

The codec counts are exact: every run gave the same totals. Allocation
calls and bytes were identical in every round-trip run too; the fractions
above 22, 27 and 141 are the background tasks' allocations spread over the
loop.

**What the floors mean for the gates.** A change aimed at ReadProperty must
save at least 100 instructions per request and more than three floors: the
bar is 100 on the current-thread runtime (three floors are 46.5) and 173.4 on
the multi-thread runtime. WriteProperty and ReadPropertyMultiple may not rise
by more than their floors.

**Where the round-trip noise comes from.** The ReadPropertyMultiple values
differ mostly inside glibc's allocator: between its lowest and highest
current-thread values, `_int_malloc` alone accounts for 620 of the 978
instructions, and the next four allocator functions for most of the rest.
Which free chunks the allocator finds depends on how the client's and
server's tasks interleave, so it varies from run to run even on one thread.
WriteProperty's current-thread floor comes from one high value (49,975.6;
the other eleven lie within 17 of each other). The multi-thread runtime adds
the two workers' interleaving.

**Compare only within one matrix.** A later A vs B check the same day (dev
against this branch, identical binaries, one A B B A block) measured
ReadProperty at 40,815 instructions per request on the current-thread
runtime and 41,777 on the multi-thread runtime, with B within the floors of
A. The multi-thread value is about 90 above this run's median, more than its
floor: some work, such as the timer wheel's turns and clock reads, follows
wall time, which machine load stretches under Valgrind. Counts are far less
sensitive to load than wall-clock time, but not immune to it, so a verdict
uses only values from one interleaved matrix, as the script does.

## The #1978 items in the profile

Self instructions per ReadProperty request on the current-thread runtime
(build A; A' matches), by function name and, as a cross-check, by the
source file the instructions come from. The by-file figure sees code inlined
into other functions but misses the function's own calls into shared code,
so the two bracket the cost differently; they don't add.

| Rank | Item | By function | By source file | Main functions |
| --- | --- | --- | --- | --- |
| 1 | 2a: Tokio locks (Mutex, RwLock, semaphore) | 1,652.8 | 874.5 | `Semaphore::add_permits_locked` 698.2 (14 calls), `Acquire::poll` 506.3 (20 calls), the `SegmentAckRoute` map's `Mutex::lock` 186.0, `Semaphore::release` 125.1 |
| 2 | 6: clock reads | 1,627.9 | 1,051.8 | `Timespec::now` 804.4 (17 calls), `Timespec::sub_timespec` 384.2, `Instant::duration_since` 171.1, `clock_gettime` 170.1 (17 calls) |
| 3 | 8: client dispatch loop `select!` | 830.6 | 300.7 | the loop's `poll_fn` 172.7 and body 131.0 (2 wakes per request), the cleanup channel's `recv` and `pop` 204.8, the network-control channel's 194.6, `Interval::poll_tick` 77.5 |

The ranking is the same on the multi-thread runtime (1,653.2, 1,569.5 and
828.0) and for WriteProperty, where the locks cost more (2,712.9 against
1,741.1 and 830.9). For ReadPropertyMultiple, clocks and locks swap places
(1,629.0 against 1,577.7), within 4% of each other.

Caveats:

- **Locks (2a)** cover every Tokio lock in the process, client and server.
  The client's transaction-table mutex and the server's database lock (2b)
  share the semaphore functions, so this profile doesn't split them. The
  atomic helpers the lock code calls (`__aarch64_*`) aren't included.
- **Clocks (6):** under Valgrind, glibc's `clock_gettime` makes a system call,
  whose kernel work callgrind doesn't count, so a native clock read likely
  costs more than counted here. The robust figure is the count: 17 clock
  reads per request.
- **Item 8** includes the whole dispatch loop's own code, part of which is
  needed for every reply, and the network-control channel, which the
  server's loop also polls. The idle branches alone (cleanup channel,
  network control, purge interval) are about 480 instructions per request.

As a check on the method, the `SmallVec<[u8; 6]>` byte-copy frame of item 1
costs 688.0 instructions in 8 calls per request, exactly what the frozen
[#2012 profile](../1978-macaddr/README.md) attributed to it.

## Reproduce

```bash
python3 scripts/perf/perf_counts.py run --noise 64bb7eb27 --workloads rp --out /tmp/noise-rp
python3 scripts/perf/perf_counts.py run --noise 64bb7eb27 --workloads wp,rpm --out /tmp/noise-wp-rpm
python3 scripts/perf/perf_counts.py run --noise 64bb7eb27 --workloads codec --out /tmp/noise-codec
```

Run it from a checkout that has `benchmarks/counts/` (64bb7eb27 itself
doesn't, so the script copies the checkout's harness into both trees). On
the laptop, the three runs took about 3, 6 and 6 minutes, builds included.
