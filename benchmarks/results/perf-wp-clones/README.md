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
`--aim wp`. The baseline is dev `211f1b616`, which doesn't have S3 yet, so
the eight clones S3 removes remain on both sides.

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

### Change 1 alone (`0ea3c14db` against `211f1b616`)

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- |
| WriteProperty | ct | 42,546.3 | 42,270.4 | -275.9 | 107.1 | 6 of 6 | FAIL (needs > 321.3) |
| WriteProperty | mt | 43,804.5 | 43,491.7 | -312.8 | 206.4 | 6 of 6 | FAIL (needs > 619.2) |
| ReadProperty | ct | 33,833.1 | 33,835.5 | +2.4 | 11.7 | 1 of 6 | PASS |
| ReadProperty | mt | 35,141.4 | 35,146.6 | +5.1 | 28.0 | 3 of 6 | PASS |
| ReadPropertyMultiple | ct | 92,877.0 | 92,877.5 | +0.6 | 475.1 | 3 of 6 | PASS |
| ReadPropertyMultiple | mt | 95,389.5 | 95,255.5 | -134.0 | 682.7 | 5 of 6 | PASS |

All four clones went (-521 instructions), but `from_slice` costs about 60
instructions per copy, mostly a `memcpy` call, so the net saving was about
290.

### Changes 1 and 2 (`5602a5b7e` against `211f1b616`)

| Workload | Runtime | A | B | B - A | Floor | Pairs lower | Calls A / B | Verdict |
| --- | --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| WriteProperty | ct | 42,549.8 | 42,044.5 | -505.4 | 198.7 | 6 of 6 | 27.0002 / 27.0002 | FAIL (needs > 596.1) |
| WriteProperty | mt | 43,777.3 | 43,277.9 | -499.3 | 104.1 | 6 of 6 | 27.0002 / 27.0003 | PASS |
| ReadProperty | ct | 33,835.3 | 33,835.5 | +0.2 | 5.6 | 3 of 6 | 22.0002 / 22.0002 | PASS |
| ReadProperty | mt | 35,141.0 | 35,141.1 | +0.1 | 22.9 | 3 of 6 | 22.0002 / 22.0002 | PASS |
| ReadPropertyMultiple | ct | 92,885.8 | 92,784.3 | -101.5 | 289.6 | 5 of 6 | 141.0004 / 141.0004 | PASS |
| ReadPropertyMultiple | mt | 95,171.1 | 95,380.4 | +209.3 | 635.8 | 2 of 6 | 141.0005 / 141.0004 | PASS |

**Verdict: not accepted under the fixed rule, on one row.** WriteProperty
falls by about 500 instructions per request on both runtimes, with all
twelve pairs lower and allocations unchanged. The four clones are gone from
the WriteProperty profile. Its eight remaining `SmallVec::clone` calls are
S3's sites, the same as on ReadProperty. ReadProperty and
ReadPropertyMultiple aren't higher beyond their floors. The multi-thread
row passes at 4.8 floors. The current-thread row fails the more-than-3-floors
test only because this matrix's WriteProperty current-thread floor was
198.7. The other recent matrices measured that floor at 107.1 (change 1's)
and 114.0 (S3's item 1). The run was not repeated to get a lower floor;
whether to accept the change is the maintainer's call.

`attribution-wp-ct.md` and `attribution-wp-mt.md` list the per-function
changes. The saving: -521 from the clones and -348 from `effect` and
`current`. The cost: +175 for the new `replace` (two `from_slice` copies, a
`mem::replace` and the drop of the replaced source), +132 for the inlined
comparison in `command`, and +48 for `memcpy`.

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

- `clone-only/`, `combined/`: each comparison's `summary.md` and
  `summary.json` from `perf_counts.py`, the #1978 item shares (`items.md`)
  and per-function and per-line slopes (`profiles/`);
  `combined/attribution-wp-*.md` is the `attribute` output.

Reproduce:

```bash
python3 scripts/perf/perf_counts.py run --baseline 211f1b616 --candidate 0ea3c14db \
  --workloads rp,wp,rpm --aim wp --out /tmp/clone-only
python3 scripts/perf/perf_counts.py run --baseline 211f1b616 --candidate 5602a5b7e \
  --workloads rp,wp,rpm --aim wp --out /tmp/combined
```
