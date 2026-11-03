# #1979(a): NPDU decode and ReadProperty encode, by counts (S1)

Part of [#1979](https://github.com/jscott3201/rusty-bacnet/issues/1979),
slice S1 of the counts-first plan. Section 10.1 of Benchmarks.md timed two
codec workloads slower on dev than on v0.13.0: decoding a 100-byte NPDU
(11.854 to 16.061 ns) and encoding a whole ReadProperty request (109.4 to
113.0 ns). This directory attributes both with the counting harness
(Benchmarks.md section 10.4): callgrind instructions and allocator calls per
iteration, per function and per source line, and per merge between the two
releases. It changes no production code.

All figures are instructions per iteration (callgrind instruction reads).
Codec counts are exact: two clean builds of one commit gave byte-identical
binaries and the same count to the instruction, so the noise floor is zero
and any change counts. Everything ran in local Docker on an Apple M5 laptop,
image `rb-perf-counts:ac28c4cf5d0a` (libc6 2.39-0ubuntu8.9, Valgrind 3.22.0,
Rust 1.99.0), with the harness's fixed allocator settings.

## 1. v0.13.0 against dev, two build profiles

A is v0.13.0 (`dca94f7f`), B is dev `a1d54afb8`. v0.13.0 builds the harness
with `benchmarks/counts/compat/v013_setup.rs` in place of `src/setup.rs`
(`perf_counts.py run --baseline-compat v013`), in A and A' only: only the
round-trip setup's constructors moved, and the codec binary doesn't use
them. The compat setup is compile-only, so the script accepts it with codec
workloads alone. Refreshing the harness lock for v0.13.0 dropped four
packages that tree no longer uses (autocfg, cfg_aliases, memoffset, nix;
the manifest lists them per build as `lock_dropped`, next to
`setup_compat`); every shared package kept its version.

The harness builds like the published binaries (fat LTO, one codegen unit:
"release-like"). The timings in section 10.1 came from the Criterion suite,
which builds with the default release profile (no LTO, 16 codegen units), so
the same matrix also ran with that profile (`default-profile/`, the harness
with only its `[profile.release]` changed). Without the counting allocator:

| Workload | Release-like A | B | B - A | Default profile A | B | B - A |
| --- | --- | --- | --- | --- | --- | --- |
| Decode NPDU, 100-byte payload | 219 | 172 | **-47** | 222 | 219 | **-3** |
| Encode NPDU, 100-byte payload | 414 | 384 | -30 | 408 | 446 | +38 |
| Encode ReadProperty request | 453 | 306 | -147 | 503 | 705 | +202 |
| Encode confirmed-request APDU | 551 | 312 | -239 | 532 | 522 | -10 |
| Whole ReadProperty request (NPDU, APDU, service) | 1,838 | 1,412 | **-426** | 1,935 | 2,157 | **+222** |
| Decode ReadProperty request | 344 | 180 | -164 | 465 | 169 | -296 |
| Decode confirmed-request APDU | 241 | 222 | -19 | 246 | 239 | -7 |

Allocator calls are unchanged in every row (0, 1 or 5 per iteration).
`release-like/` and `default-profile/` hold each matrix's `summary.md`,
`runs.tsv` and per-function and per-line slopes (`profiles/`). Their
"not acceptable as run" lines only say that a codec-only run has no round
trips to judge; these runs are evidence, not a gate.

## 2. Every change, merge by merge

`bisect/` built a codec-only copy of the harness (`bisect/codec-harness-Cargo.toml`:
no round-trip setup, so it builds at every commit) at v0.13.0, at each of
the 32 first-parent merges between v0.13.0 and dev that touch
`bacnet-encoding`, `bacnet-npdu`, ReadProperty or the primitive types, and at
dev, in both profiles, and counted 2,000 and 12,000 iterations of five
workloads (`bisect/counts-*.txt`; these include the counting allocator,
which is the same at every commit, so the changes are unaffected). Each tree
is touched before its build: `git archive` stamps files with the commit
time, and a shared target directory would otherwise reuse the previous
commit's crates. The sweep started from the harness's `Cargo.lock` (not
committed with `codec-harness-Cargo.toml`), refreshed the path packages at
each commit and logged any third-party package that would be added; none
was, but unlike `perf_counts.py` the sweep didn't refuse one. The totals
telescope either way: consecutive counts are differences of the same
builds. The merges that moved a count:

Default profile (what section 10.1 timed):

| Merge | PR | NPDU decode | NPDU encode | ReadProperty request | Whole ReadProperty |
| --- | --- | --- | --- | --- | --- |
| d5a08d625 | #1757 tag writer | | | -17 | -17 |
| 69f75f83d | #1760 scalar encoder sinks | | | -27 | -27 |
| fca238953 | #1780 allocation-free profile | +3 | -1 | **+267** | **+257** |
| 62a5baf1d | #1788 bacnet-npdu crate | **+51** | +39 | | +30 |
| 28e63162c | #1798 static codec errors | | | -21 | -21 |
| 4a098fccb | #1821 (#1713) per-request cuts | **-55** | | | |
| 28262cb77 | #1832 static encoder errors | | | -73 | -73 |
| dbe342f94 | #1843 network layer on router-core | -2 | | | |
| 06722c58e | #1990 edition 2024 | | | +73 | +73 |
| **Total** | | **-3** | **+38** | **+202** | **+222** |

Release-like:

| Merge | PR | NPDU decode | NPDU encode | ReadProperty request | Whole ReadProperty |
| --- | --- | --- | --- | --- | --- |
| 0853ea403 | #1751 decode performance | +1 | +1 | | +1 |
| d5a08d625 | #1757 tag writer | | -121 | **-205** | **-557** |
| 69f75f83d | #1760 scalar encoder sinks | | | +60 | +64 |
| fca238953 | #1780 allocation-free profile | -7 | -1 | +4 | -6 |
| 62a5baf1d | #1788 bacnet-npdu crate | -2 | +87 | | +76 |
| 28e63162c | #1798 static codec errors | | +4 | -6 | -4 |
| 4a098fccb | #1821 (#1713) per-request cuts | **-40** | | | |
| dbe342f94 | #1843 network layer on router-core | +1 | | | |
| **Total** | | **-47** | **-30** | **-147** | **-426** |

The totals equal section 1's differences exactly. `bisect/steps.md` has the
per-function changes and call counts at the steps that matter:

- **#1780, ReadProperty encode, default profile, +267.** The request now
  encodes through the sink API: `encode_reference` writes into a stack
  slice and `staged` copies that into the `BytesMut`. Without LTO the
  slice sink's `write` (160) and `write_tagged::<&mut [u8]>` (102) stay out
  of line, replacing `encode_ctx_unsigned` and `encode_ctx_object_id`
  (-149), with two more `memcpy` calls. Release-like, the same code costs +4.
- **#1788, NPDU decode, default profile, +51.** `decode_npdu` became an
  adapter over `bacnet_npdu::Npdu::decode`, which is now a call of its own
  (+46) instead of inline parsing. Release-like, LTO inlines it (-2).
- **#1788, NPDU encode, +39 / +87.** `encode_npdu` sizes the buffer with
  `encoded_len`, zero-fills it with `BytesMut::resize` (a `memset`) and
  writes with `encode_into`, in place of one `put_slice` per field.
- **#1821, NPDU decode, -55 / -40.** The payload is advanced, not sliced:
  one `Bytes` clone and its drop are gone (two atomic read-modify-writes).
- **#1757, release-like, -557.** The tag writer's small writes inline;
  three `put_slice` calls and four `memcpy` calls per ReadProperty request
  disappear.
- **#1832 -73 and #1990 +73 (default profile)** are one codegen flip: the
  compiler outlines one more `write_tagged` call after #1990 and one fewer
  after #1832. Calls of real work and allocations are unchanged.

Those are the steps tied to functions (`steps.md`): #1780, #1788, #1821,
#1832 and #1990 in the default profile, and #1757 and #1788 release-like
(#1821 release-like moves the same two `Bytes` calls but wasn't profiled
apart). The rest are tied only to their merge: #1757 in the default profile
(-17), #1760 (-27; release-like +60 and +64), #1798 (-21; release-like -6
and -4), #1780 release-like (-7 to +4), #1788's -2 release-like decode,
#1843 (-2 and +1) and #1751 (+1).

## 3. A direct NPDU parser (variant R)

`direct-npdu-decode.patch` is a scratch change to dev (never merged): an
accepted frame is parsed straight into the owned `Npdu`, and any fault falls
back to the view so the error contract stays the same. It was checked only
by the harness's decode oracle, not by the codec test suite. Against dev,
decode only (`direct-npdu-*/`):

| Profile | dev | R | R - dev |
| --- | --- | --- | --- |
| Release-like | 172 | 154 | -18 |
| Default | 219 | 158 | -61 |

In the default profile R saves 61 instructions, more than the +51 the view
adapter added in #1788. That compares R with one gross step, not with the
net change from v0.13.0 (-3), which is a saving and leaves nothing to
recover. Release-like, #1788 added nothing either; R still saves 18 on the
parse.

## 4. Verdict

The declared rule (plan section 3(a)) asks that listed deltas cover at least
90% of the net change per iteration, each tied to a commit, and that for NPDU
decode a direct parser recover at least 80% of the delta.

- **Both shifts are attributed.** The merge tables cover 100% of every net
  change in both profiles, but that holds by construction: a sweep over every
  merge telescopes. The evidence is the per-function ties in `steps.md`,
  which cover the large steps (section 2 lists which steps have them and
  which are tied only to a merge).
- **The 80% clause is vacuous.** On the net change it asks about, NPDU decode
  got cheaper in both profiles (-3 and -47), so there is nothing to recover.
  R's 61 is measured against #1788's gross +51, one step of the sweep.
- **Release-like builds (what ships) got cheaper.** Dev runs fewer
  instructions than v0.13.0 in all seven codec workloads, 426 fewer for a
  whole ReadProperty request and 47 fewer per NPDU decode.
- **ReadProperty encode, as timed, is #1780.** In the default profile the
  whole request costs 222 more instructions (+11%), in the same direction
  as the timed +3%; #1780's out-of-line sink path is +257 of it.
- **NPDU decode's timed rise is not explained by instruction counts.** In
  the profile that was timed, dev's count is 3 lower: #1788's adapter (+51)
  and #1821's removed clone (-55) cancel. What makes the same number of
  instructions take 35% longer is not established here: no cache or branch
  simulation ran, #1997's explanation (the 88-byte view and 128-byte owned
  copies through the stack) is untested by these counts, and #1821 removed
  two atomic read-modify-writes, which on their own would predict a faster
  decode, not a slower one.

No production change is in this slice. A direct `decode_npdu` and inlining
the slice sink would help builds without LTO (61 per decode, up to about 250
per ReadProperty request encode) but only 18 per decode release-like: about
36 per B/IP ReadProperty round trip (one decode at each end), under the
100-instruction bar of #1978's acceptance rule.

## Reproduce

From a checkout with this harness (the default-profile runs copy
`benchmarks/counts/` to a scratch directory and delete `lto` and
`codegen-units` from its `[profile.release]`):

```bash
python3 scripts/perf/perf_counts.py run --baseline dca94f7f --candidate a1d54afb8 \
  --baseline-compat v013 --harness benchmarks/counts --workloads codec --aim none --out /tmp/s1
python3 scripts/perf/perf_counts.py attribute /tmp/s1 --workload full_stack_encode_rp --runtime none
```

The release-like matrix took about 25 minutes, the default-profile one about
20, and the two-profile merge sweep about 25.
