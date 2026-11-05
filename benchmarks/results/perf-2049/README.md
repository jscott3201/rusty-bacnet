# A cheap policy's trip to the blocking pool (#2049)

[#2049](https://github.com/jscott3201/rusty-bacnet/issues/2049), a
follow-up to [#2032](https://github.com/jscott3201/rusty-bacnet/issues/2032),
counted the way the counting harness counts (Benchmarks.md section 10.4).
Section 10.11 there summarizes it.

All instruction figures are callgrind instruction reads per request,
without the harness's counting allocator. "Calls" are allocator calls per
request and "bytes" the bytes requested from the allocator per request.
`ct` is Tokio's current-thread runtime, `mt` its multi-thread runtime with
two workers. The run built like the release with the fixed allocator
settings, in local Docker on an Apple M5 laptop, image
`rb-perf-counts:ac28c4cf5d0a` (libc6 2.39-0ubuntu8.9, Valgrind 3.22.0,
Rust 1.99.0), from commit `7bef2e4a7`.

## The question

Since #2032 the server asks every `ServerPolicy` decision for a received
request on Tokio's blocking pool, so that a policy that blocks, such as a
Python `mutation_policy` callable, holds neither a runtime worker nor
`stop()`. A Rust policy that only compares a field pays for that trip on
every mutating request. #2049 adds `ServerPolicy::decides_inline`: a policy
that returns `true` is called on the request task instead. This measures
what the trip costs and what the inline path saves.

## The workload

`benchmarks/counts/examples/policy_counts.rs` runs the harness's `wp`
round trip (WriteProperty of 72.5 to Analog Output 1 at priority 16, over
B/IP loopback, client and server in one process) through a server with:

- `none`: no policy, as `counts wp`;
- `pool`: a policy that allows writes from the local network only (one
  field test) and keeps the default, so each request asks it on the pool;
- `inline`: the same policy, declared inline.

It is an example, not a `counts` workload, because it needs
`decides_inline`, and `perf_counts.py` builds every binary of the harness
in both trees it compares: as a binary it would stop any baseline older
than #2049 from building. So `measure.py` here runs it with the harness's
settings: the same image, lock refresh, `--locked`, `CARGO_INCREMENTAL=0`,
`GLIBC_TUNABLES`, `--network none`, the binary's hash checked before each
run, 1,000 and 10,000 requests after 200 warm-up requests, and the
counting allocator's cost subtracted. It builds once and runs the order
none, pool, inline, inline, pool, none three times, so each row has six
values; with one build there is no A' floor, and each row's range stands
in for one. Two clean builds gave byte-identical binaries
(`b0c2f53d5f52...`), and an earlier run of the same matrix agreed with
this one within each row's range.

## Counts

| Policy | Runtime | Instructions | Range of 6 | Calls | Bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| none | ct | 39,656.3 | 13.5 | 25.0002 | 7,944.02 |
| pool | ct | 45,368.6 | 286.8 | 30.0008 | 8,406.04 |
| inline | ct | 41,416.3 | 95.4 | 29.0002 | 8,150.02 |
| none | mt | 40,830.6 | 46.5 | 25.0002 | 7,944.02 |
| pool | mt | 47,465.2 | 323.1 | 30.0003 | 8,406.04 |
| inline | mt | 42,624.8 | 122.4 | 29.0002 | 8,150.02 |

- **The trip costs about 4,000 to 4,800 instructions per request.** Pool
  minus inline is 3,952.3 on the current-thread runtime and 4,840.5 on the
  multi-thread one, each pair of runs 3,805 to 4,097 and 4,677 to 5,000
  apart, so every inline run is lower than its pool pair by more than 12
  times the widest range. It is also one allocator call (the blocking
  task) and 256 bytes per request.
- **The inline path removes it.** What a policy still costs inline, 1,760.0
  and 1,794.2 instructions and four allocator calls over no policy, is the
  work both paths share: decoding the request for the policy, building its
  context, and the vectors that carry the decisions.
- **`none` is the harness's `wp`.** 39,656.3 and 40,830.6 are within
  0.15% of the official count of `wp` on dev after #2373 (39,657.1 and
  40,890.4; on the multi-thread runtime 59.8 lower, inside that count's
  floor of 141.5), so the example measures the same round trip. A server without a policy doesn't
  reach the new branch.

What the pool path adds, in calls per request (`summary.md` lists the top
25 for each runtime): the blocking task itself (four more drops of its
stage), two more polls of the request task, which waits for the answer,
two condition-variable waits, two to four raw `syscall` calls (how the
futex waits and wakes appear), five clock reads on the current-thread
runtime and ten on the multi-thread one (mostly timed waits and parks), and
about 75 to 85 more outlined atomic operations. Callgrind counts only the
program's own instructions, so the kernel's side of those thread wake-ups
isn't in these figures.

## Wall time

The same binary without Valgrind, 20,000 requests per run, five blocks of
the same order (ten runs per row), on the same laptop while other builds
were running (load average about 6 to 9). Treat these as a sense of scale,
not as figures to compare with other sections.

| Policy | ct, median | ct, middle half | mt, median | mt, middle half |
| --- | ---: | ---: | ---: | ---: |
| none | 5,750 ns | 5,714 to 5,829 | 17,189 ns | 16,842 to 17,594 |
| pool | 18,112 ns | 17,982 to 18,980 | 38,008 ns | 35,046 to 43,153 |
| inline | 5,869 ns | 5,835 to 5,897 | 17,265 ns | 17,076 to 19,689 |

Most of the trip's cost is there: it adds about 12 microseconds to a
WriteProperty round trip on the current-thread runtime and about 21 on the
multi-thread one, waiting for a blocking thread to wake and then for the
runtime to wake again. Inline, a cheap policy costs about 0.1 microseconds
over none. An earlier wall run of three blocks under heavier load gave
the same picture (pool 19.6 and 58.5 microseconds, inline 5.8 and 17.3).

## Files

- `measure.py`: the build, the callgrind and wall matrices, the analysis,
  and removing the build's Docker volumes.
- `summary.json`: each row's six values, median, range, allocations and
  wall times; the per-function call differences, pool minus inline; the
  commit, image, binary hash and settings.
- `summary.md`: the table and the call differences.

Reproduce (takes about five minutes):

```bash
python3 benchmarks/results/perf-2049/measure.py build 7bef2e4a7 --out /tmp/perf-2049
python3 benchmarks/results/perf-2049/measure.py counts --out /tmp/perf-2049
python3 benchmarks/results/perf-2049/measure.py wall --blocks 5 --out /tmp/perf-2049
python3 benchmarks/results/perf-2049/measure.py analyze --out /tmp/perf-2049
python3 benchmarks/results/perf-2049/measure.py clean --out /tmp/perf-2049
```
