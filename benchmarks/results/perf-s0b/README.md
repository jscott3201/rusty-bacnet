# Counting harness: release-like build, fixed allocator settings (S0b)

Part of [#2314](https://github.com/jscott3201/rusty-bacnet/issues/2314), the
follow-ups to the first baseline in [perf-s0](../perf-s0/README.md). The
harness now builds like the published binaries and runs with fixed glibc
allocator settings; this directory records why, and the new baseline and
noise floors. It changes no production code.

All instruction figures are callgrind instruction reads per request (per
iteration for the codec workloads), without the harness's counting
allocator. "Calls" are allocator calls per request. `ct` is Tokio's
current-thread runtime, `mt` its multi-thread runtime with two workers. A
floor is the range of the twelve values of two clean builds of one commit
(A and A'). Everything ran in local Docker on an Apple M5 laptop, image
`rb-perf-counts:ac28c4cf5d0a` (Ubuntu snapshot 20261024T000000Z: libc6
2.39-0ubuntu8.9, Valgrind 3.22.0, gcc 13.2.0; Rust 1.99.0), while other
agents' builds shared the machine.

## 1. Built like the release

`benchmarks/counts` builds with fat LTO and one codegen unit, like the
root workspace's `dist` profile, keeping line tables. The counting
allocator's methods are kept out of line so the analysis can still subtract
them by name; under fat LTO they were inlined and the missing-share check
stopped the codec runs.

**The codegen rise vanishes.** GitHub run 41519435646 compared 64bb7eb27 with
3a2bd4b73 under the default profile and found ReadProperty +167 (ct) and +121
(mt), WriteProperty +152 and +128, all from inlining changes. Built like the
release (`profile-check/lto-compare.md`, `--aim none`, no allocator
settings):

| Workload | ct: B - A (floor) | mt: B - A (floor) |
| --- | --- | --- |
| ReadProperty | -1.5 (13.3) | -4.2 (84.7) |
| WriteProperty | +0.2 (23.8) | -3.6 (243.7) |
| ReadPropertyMultiple | +565.8 (1,404.5) | +11.8 (627.5) |

Accepted by the no-regression check: nothing rose above its floor.

**Floors don't rise.** The release-like noise run on dev 9764c9db8, before
the allocator settings (`profile-check/lto-noise.md`), against S0's default
profile: ReadProperty ct 10.6 (S0 15.5), WriteProperty ct 107.7 (94.0),
ReadPropertyMultiple ct 820.1 (978.4). The multi-thread floors move between
sessions with machine load (ReadProperty 130.8 here, 84.7 in the
comparison), so the two profiles were run back to back for those
(`profile-check/mt-default-1.md`, `mt-lto-1.md`): ReadProperty 104.5 with
the default profile and 88.8 built like the release; WriteProperty 158.0
and 172.8. Neither profile is noisier.

## 2. Fixed allocator settings

With glibc's defaults, which free chunk a request finds depends on how the
client's and server's tasks interleaved, and ReadPropertyMultiple's counts
spread over hundreds of instructions even on one thread. Each setting below
ran six times on one built binary (1,000 and 10,000 requests each,
current-thread):

| `GLIBC_TUNABLES` | ReadPropertyMultiple | Its range | ReadProperty | Its range |
| --- | --- | --- | --- | --- |
| (glibc's defaults) | 99,025.3 | 511.6 | 33,827.2 | 14.6 |
| `tcache_count=0` | 119,716.0 | 67.1 | | |
| `tcache_count=0:mxfast=0` | 127,791.1 | 5.9 | | |
| fixed mmap, trim and top-pad thresholds (1 MiB) | 99,101.3 | 295.3 | 33,825.0 | 8.4 |
| `tcache_count=65535` | 91,364.8 | 46.2 | 33,696.7 | 6.9 |
| `tcache_count=16` + fixed thresholds | 93,387.3 | 295.2 | 33,827.4 | 4.6 |
| **`tcache_count=32` + fixed thresholds** | **92,864.5** | **31.4** | **33,828.0** | **6.5** |

Turning the per-thread cache off steadies the counts but adds 20,000 to
29,000 instructions of allocator work per request; a very large cache moves
ReadProperty by 130. A cache of 32 chunks per size with fixed thresholds
keeps ReadProperty where it was and brings ReadPropertyMultiple's spread
from hundreds to about 30. It is now the default
(`glibc.malloc.tcache_count=32:glibc.malloc.mmap_threshold=1048576:glibc.malloc.trim_threshold=1048576:glibc.malloc.top_pad=1048576`;
`--glibc-tunables` overrides it). ReadPropertyMultiple itself costs about
6,200 fewer instructions with it, because fewer requests reach the
allocator's slower paths, so its absolute numbers aren't comparable with
S0's.

## 3. New baseline and noise floors

Dev at 9764c9db8, release-like build, fixed allocator settings, 312
callgrind runs (`noise/`). A and A' gave byte-identical binaries.

| Workload | Runtime | Instructions per request | With harness | Noise floor | S0 floor | Calls |
| --- | --- | --- | --- | --- | --- | --- |
| ReadProperty | ct | 33,827.3 | 34,754.3 | 11.4 | 15.5 | 22.0001 |
| ReadProperty | mt | 35,133.9 | 36,055.9 | 14.8 | 57.8 | 22.0002 |
| WriteProperty | ct | 42,649.7 | 43,784.7 | 9.9 | 94.0 | 27.0002 |
| WriteProperty | mt | 43,877.2 | 45,007.2 | 146.0 | 146.9 | 27.0002 |
| Ten-object ReadPropertyMultiple | ct | 92,863.5 | 98,744.6 | 41.4 | 978.4 | 141.0004 |
| Ten-object ReadPropertyMultiple | mt | 95,180.7 | 101,061.7 | 641.0 | 664.7 | 141.0004 |

| Codec workload | Instructions per iteration | Calls |
| --- | --- | --- |
| `encode_read_property_request` | 306 | 1 |
| `decode_read_property_request` | 180 | 0 |
| `encode_npdu_100b_payload` | 384 | 1 |
| `decode_npdu_100b_payload` | 172 | 0 |
| `encode_apdu_confirmed_request` | 312 | 1 |
| `decode_apdu_confirmed_request` | 222 | 0 |
| `full_stack_encode_rp` | 1,412 | 5 |

The codec floors are below 0.0001 per iteration (a few instructions per
whole program). On the current-thread runtime every round-trip floor is now
under 42, so the ReadProperty bar is the 100-instruction minimum. On two
workers, WriteProperty and ReadPropertyMultiple still vary with how the
workers interleave; ReadProperty's floor came out at 14.8 in this session.

## 4. Lock cost by caller

`noise/callers.md` comes from one extra pair of build A runs per round trip
with callgrind's `--separate-callers=8`. The Tokio semaphore's own
instructions per request, by the nearest caller that names a group:

| Workload | Client locks (2a) | Server locks (2b) | Bounded channel permits |
| --- | --- | --- | --- |
| ReadProperty, ct and mt | 757.0 | 228.5 / 228.6 | 144.0 |
| WriteProperty, ct and mt | 757.0 | 1,112.6 / 1,112.7 | 144.0 |
| ReadPropertyMultiple, ct and mt | 757.0 / 757.2 | 230.2 / 232.0 | 144.0 |

Every semaphore instruction found a group within eight callers. The bounded
channel's receive is inlined into the network layer's APDU receiver, so its
permit returns (about 1.9 per request) appear with that receiver as their
nearest caller and no channel frame; the split counts the receiver as the
channel. The client's share is the same for every service; the server's
grows for WriteProperty, which takes the database lock for writing. S0's
estimate of about 190 for the channel share was close. Item 2a's
by-function total in `noise/items.md` (1,488.6 for ReadProperty, ct) also
counts the Mutex and RwLock wrappers' own code, which isn't semaphore code.

## Reproduce

```bash
python3 scripts/perf/perf_counts.py run --noise 9764c9db8 --workloads all --callers --out /tmp/s0b
python3 scripts/perf/perf_counts.py run --baseline 64bb7eb27 --candidate 3a2bd4b73 \
  --aim none --glibc-tunables '' --harness benchmarks/counts --out /tmp/lto-compare
```

Run from a checkout with this harness: older commits' own
`benchmarks/counts/` builds with the default profile, so pass `--harness
benchmarks/counts` when the candidate predates it. The noise run took about
40 minutes on the shared laptop, most of it the fat-LTO builds and the
codec runs.
