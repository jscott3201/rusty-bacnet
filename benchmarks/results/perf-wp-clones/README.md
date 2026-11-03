# WriteProperty address copies (#2342)

Follow-up to [perf-s3](../perf-s3/README.md) for
[#2342](https://github.com/jscott3201/rusty-bacnet/issues/2342), counted with
the counting harness (Benchmarks.md section 10.4). After S3, WriteProperty
still made four byte-by-byte `SmallVec<[u8; 6]>` clones per request: copies
of the writer's `BACnetAddress`, made while the commanded object tracks its
Value_Source.

Figures are callgrind instruction reads per request without the harness's
counting allocator. "Calls" are allocator calls per request. `ct` is
Tokio's current-thread runtime, `mt` its multi-thread runtime with two
workers. The floor is the range of the twelve A and A' values from the same
matrix; A and A' were byte-identical in both runs. Each run built like the
release with the fixed allocator settings, in local Docker on an Apple M5
laptop, image `rb-perf-counts:ac28c4cf5d0a` (libc6 2.39-0ubuntu8.9,
Valgrind 3.22.0, Rust 1.99.0), with the order A B A' A' B A three times and
`--aim wp`. The verdict run compares dev `03fb3bbbf` (with S3) against this
branch merged with it, `a2ec0f8e0`; two earlier runs on dev `211f1b616`,
before S3, are kept below as history.

## Where the four copies were

On each WriteProperty from a network writer, `ValueSourceTracking::command`
(`bacnet-objects`, `command_source/tracking.rs`) cloned the writer's
address four times:
- twice for `effect`, before and after the command, to compare the
  published source;
- twice in `record`, for the published source and the retained
  `CommandOrigin`.

The profile names `LifeSafetyCovSnapshots::capture_write_property` as the
caller of one of the four. That function copies no address for an Analog
Output: it returns before reading anything. After the fix the same label
sits on the one out-of-line `BACnetAddress::clone` call. Under fat LTO,
callgrind's caller names on this path are unreliable; it also names
Tokio's semaphore and a mutex as callers of `SmallVec::clone`. So the label
is not a fifth site, and `life_safety_cov.rs` is unchanged.

## The changes

1. `0ea3c14db`, `bacnet-types`: `BACnetAddress` clones by hand with
   `MacAddr::from_slice`. That fixes every clone of the type in one
   place, including the derived clones of `BACnetValueSource` and
   `CommandOrigin`.
2. `5602a5b7e`, `bacnet-objects`: `command` no longer copies the
   published source to compare it. Of the sources, only the commanded
   slot's changes, so the published source changes exactly when that
   slot is active afterwards and the source it replaced differs. The
   replaced source comes out by `mem::replace`. Lighting keeps `effect`
   and `settle`.

### The verdict run (`a2ec0f8e0` against dev `03fb3bbbf`)

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| WriteProperty | ct | 41,952.3 | 41,450.5 | -501.8 | 7.5 | 6 of 6 | 27.0002 / 27.0002 | PASS |
| WriteProperty | mt | 43,176.8 | 42,706.3 | -470.5 | 110.5 | 6 of 6 | 27.0005 / 27.0002 | PASS |
| ReadProperty | ct | 33,241.8 | 33,245.1 | +3.3 | 10.3 | 2 of 6 | 22.0001 / 22.0001 | PASS |
| ReadProperty | mt | 34,548.1 | 34,547.6 | -0.5 | 62.3 | 4 of 6 | 22.0002 / 22.0002 | PASS |
| ReadPropertyMultiple | ct | 92,218.3 | 92,205.4 | -12.9 | 56.2 | 3 of 6 | 141.0005 / 141.0005 | PASS |
| ReadPropertyMultiple | mt | 94,782.4 | 94,802.4 | +19.9 | 1,059.7 | 3 of 6 | 141.0006 / 141.0012 | PASS |

**Verdict: accepted.** WriteProperty falls by about 500 instructions per
request: 66.8 floors on the current-thread runtime and 4.3 on the
multi-thread one, with all twelve pairs lower. ReadProperty and
ReadPropertyMultiple aren't higher beyond their floors. Allocation calls
don't rise by more than 0.01, and requested bytes are unchanged. The
WriteProperty profile now has no `SmallVec<[u8; 6]>::clone` calls (4 in A,
0 in B; `merged/profiles/wp-ct.md`).

`merged/attribution-wp-ct.md` and `attribution-wp-mt.md` list the
per-function changes. The saving: -521 from the clones and -348 from
`effect` and `current`. The cost: +175 for the new `replace` (two
`from_slice` copies, a `mem::replace` and the drop of the replaced source),
+132 for the inlined comparison in `command`, and +48 for `memcpy`.

### History: the runs before S3 (dev `211f1b616`)

Change 1 alone (`0ea3c14db`) removed the four clones but saved only -275.9
(ct, floor 107.1) and -312.8 (mt, floor 206.4) on WriteProperty, because
`from_slice` still costs about 60 instructions per copy. That failed the
more-than-3-floors test, so change 2 followed. Both changes (`5602a5b7e`)
saved -505.4 (ct, floor 198.7, failed that test) and -499.3 (mt, floor
104.1, passed), with all pairs lower. Those numbers are in `clone-only/`
and `combined/`. The verdict run above replaces them.

`tracking_differential_tests.rs` runs `command` beside the implementation
it replaced (still built from `effect`, `record` and `settle`) over a
seeded random walk of 60,000 steps. It requires the same slots, sources,
owners and Last_Command_Time after every step.

## The optional `NpduAddress` clone: left out

A hand-written `Clone` for `NpduAddress` (`bacnet-encoding`) would only pay
off where a routed source is copied: SNET/SADR on a request or answer. The
harness's round trips are direct B/IP, so their source network is `None`.
No profile of rp, wp or rpm here or in perf-s3 has a `NpduAddress` frame,
and after S3 (perf-s3 item 1) ReadProperty makes no `SmallVec::clone` calls
at all. That is zero copies per request to save. So it was not built,
`bacnet-encoding` is unchanged, and check-no-std and check-firmware-link
weren't needed for it. A routed-path workload would be needed to measure it.

## Files

- `merged/` (the verdict), `clone-only/`, `combined/`: each comparison's `summary.md` and
  `summary.json` from `perf_counts.py`, the #1978 item shares (`items.md`)
  and per-function and per-line slopes (`profiles/`);
  `*/attribution-wp-*.md` is the `attribute` output.

Reproduce:

```bash
python3 scripts/perf/perf_counts.py run --baseline 03fb3bbbf --candidate a2ec0f8e0 \
  --workloads rp,wp,rpm --aim wp --out /tmp/merged
python3 scripts/perf/perf_counts.py run --baseline 211f1b616 --candidate 0ea3c14db \
  --workloads rp,wp,rpm --aim wp --out /tmp/clone-only
python3 scripts/perf/perf_counts.py run --baseline 211f1b616 --candidate 5602a5b7e \
  --workloads rp,wp,rpm --aim wp --out /tmp/combined
```
