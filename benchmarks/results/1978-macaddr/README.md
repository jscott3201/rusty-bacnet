# Address-copy experiment for #1978

**The production optimization was rejected under the fixed acceptance protocol.**
Replacing eight reached ownership copies reduced Linux ReadProperty instruction
counts with unchanged allocation traffic, but the native latency observations did
not establish the required overall nonregression. The delivery change contains
only this evidence, its index entry and changelog. Production and product-test
files were restored byte-for-byte to the baseline. This is Part of #1978; its
other costs remain open.

The native result does not establish a causal slowdown. WriteProperty's aggregate
median decreased but its uncertainty exceeded the declared bound;
ReadPropertyMultiple's aggregate median increased and its interval was wider.
No observation was removed, no matrix was repeated, and no threshold was changed.

## Source and measured behavior

| Variant | Immutable commit | Meaning |
| --- | --- | --- |
| A | `2d82e9a949bb57702f4672705a46b19345bb3d59` | Unchanged baseline |
| B | `b1f6845eca586bc03b98c4445e82b54fd49fdc98` | Rejected measured candidate; tree `54d9c7c240825d2fc98b2bdeea657bf08c2ac34e` |

`candidate.patch.gz` contains the exact patch bytes in deterministic gzip form
(mtime zero). Decompress it before applying it to reconstruct all eight B files
exactly from A, including its three added identity tests and historical optimization changelog. Apply it only
to an isolated experimental checkout. Its seven Rust files are not present as
changes in the final delivery diff. The original candidate remains in branch
history; the later evidence commit explicitly restores A's production bytes.
The raw patch hash is unchanged. Compression preserves blank context markers
without introducing trailing whitespace into the tracked text artifact.

The public `MacAddr = SmallVec<[u8; 6]>` alias, dependency versions and features
were unchanged. B used the existing safe `from_slice` constructor at the reached
client request/guard, transaction-peer, ComplexACK key and server receive copies,
and manual byte-preserving `Clone` implementations for the two canonical peer
identities. No ownership was borrowed, and no admission/security/lock behavior
was changed. Tests covered lengths 0/1/3/6/7/18/23/64, inline and spilled storage,
network/variant/length/trailing-byte distinctions, map lookup and mutation
independence, including the 23-byte routed transaction key for an 18-byte address.

The separate exploratory A current-thread profile attributed 7,018,976 self
instructions to the generic `SmallVec<[u8; 6]>` byte-`Extend` frame over 10,202
requests: eight calls and 688 frame instructions per request. Source/caller
attribution reached transaction-peer construction twice, request registration and
its guard once each, server receive/admission once each, coordinator admission
once, and the client ComplexACK key once. The selected final B profile no longer
contains that frame. Other SmallVec uses are outside this attribution. The net
end-to-end instruction saving below is not an additive claim that every frame
instruction disappeared without replacement or other code-generation effects.

## Frozen protocol and environment

Both matrices used `A B B A` repeated three times, six invocations per variant.
Pairing uses equal per-variant ordinals. Owned builds and benchmark suites ran serially, with root reporting no
competing work. Pre/post process snapshots support that account; they are not
continuous proof excluding every unrecorded source of host interference.
All 60 planned invocations exited zero, but execution success
is distinct from the failed overall performance acceptance.

- Callgrind: ReadProperty on current-thread (`ct`) and two-worker multi-thread
  (`mt`) Tokio runtimes, 200 warmups, separate N=1000/10000 invocations; each
  slope is `(Ir10000 - Ir1000) / 9000`. All six B slopes must be lower than their
  A pair, and the difference of median slopes must exceed A's max-minus-min
  repeated slope drift on each runtime. Matched median allocation-call and
  requested-byte traffic at N=10000 must not increase; allocation slopes are
  also retained. This definition was encoded before timing.
- Native: the actual unmodified `bip_latency` Criterion binary, RP/WP/10-object
  RPM, `TOKIO_WORKER_THREADS=2`, sample size 100, warmup 3 seconds and measurement
  5 seconds. For each workload B's median of invocation medians must not exceed
  A's, and the paired bootstrap 95% interval's upper slowdown bound must be at
  most 2%. That bound does not permit a higher observed median.
- Bootstrap: enumerate all `6^6 = 46,656` ordered samples of six paired ordinals
  with replacement. Each statistic is `median(B) / median(A) - 1`; endpoints
  are linearly interpolated 2.5th and 97.5th percentiles. It is not a bootstrap
  of pooled Criterion samples. Every invocation's original estimate and sample
  bytes remains available, as do all paired differences and ranges. These are
  Criterion samples of repeated operations, not 3,600 individual request latencies.

The host was an Apple M5, 10 CPUs, 16 GiB RAM, macOS 27.2 (26B118),
`aarch64-apple-darwin`, Rust/Cargo 1.99.0, rustc `b940084d7`, LLVM 23.1.1.
Linux Callgrind ran in arm64 Docker 29.8.2 with four assigned CPUs and
5,155,708,928 bytes of VM memory, Rust 1.99.0/LLVM 23.1.1 and Valgrind 3.22.0.
The task derivative image ID and actual executable paths/hashes are in
`inputs.json`. Builds used separate targets, `CARGO_INCREMENTAL=0`, the identical
normalized external harness lock and manifests, and no feature override.
Native compiled features/profiles matched for all 226 units. Linux's common
release profile adds line-table debug information; per-unit Linux feature
metadata was not separately captured, and the full inherited native build
environment was not archived. `build-inspection.json` states the recorded evidence;
it is not a complete verbose compiler-invocation/environment graph.

**Power observation remains unresolved.** The pre-matrix snapshot reported
`Battery Power`, 100%. Every native invocation's header reported `AC Power`,
while its battery detail simultaneously said `discharging`, falling from 100%
to 97%. The post-matrix snapshot again reported `Battery Power`, 97%. Original
strings are preserved in `runs.json` and `provenance.json`. They are not
normalized into steady AC, asserted to cause the latency differences, or used
to exclude any run. No affinity/frequency/system-power setting was changed.

The Linux command checks its executable hash before each invocation. The native
runner checks both binaries and full source inventories before and after the
matrix, not before every invocation. The original independent archive review
verified recorded guards/hashes, not archived executables: none were in that
initial archive. After timing, four retained executables were copied and rehashed
as supplemental local evidence; those later copies are not part of that original
archive review or the distributed Git payload.

The common harness checks the actual AI1 value (21.0), response identity and
property/index before and after the counted loop. Its WP control verifies AO1
readback (72.5). These checks passed for A/B on ct/mt before final measurements.
The timed native workloads are the existing RP/WP/RPM benchmarks, unchanged.
Whole-program instruction slopes remove fixed setup/oracle work; allocation
counters cover the N-request loop across all threads, including background work.
Requested allocation traffic is not retained/peak memory, and the hosted round
trip is not allocation-free.

## Results

Instruction values are median [minimum–maximum] instructions/request:

| Runtime | A | B | Median saving | Repeated A drift | Six paired savings range | Result |
| --- | --- | --- | --- | --- | --- | --- |
| ct | 40406.768 [40403.548–40415.257] | 39886.577 [39882.238–39891.203] | 520.192 | 11.710 | 514.809–526.886 | Pass |
| mt | 41490.685 [41484.690–41494.730] | 40970.771 [40967.919–40976.715] | 519.914 | 10.040 | 514.848–522.059 | Pass |

Matched median allocation calls/request and requested bytes/request were exactly
unchanged: ct **22.0002 / 2228.0208**, mt **22.0003 / 2228.0312**, on both A/B.
All raw N=1000 and N=10000 instruction/allocator totals, allocation slopes and
ranges are distributed; fractional counts reflect division and background work.

Native values are median [minimum–maximum] of six invocation medians, in ns.
Slowdown is the ratio of aggregate medians minus one; negative means lower B.

| Workload | A | B | Aggregate slowdown | Paired bootstrap 95% interval | Observed median gate | Upper-bound gate |
| --- | --- | --- | --- | --- | --- | --- |
| RP | 26487.684 [26029.578–27527.951] | 26087.477 [25910.543–27287.388] | -1.5109% | [-2.5175%, +0.0995%] | Pass | Pass |
| WP | 29119.155 [27451.532–31099.326] | 27473.507 [27440.454–32033.241] | -5.6514% | [-9.6609%, +3.0030%] | Pass | **Fail** |
| 10-object RPM | 32317.326 [31396.139–34858.549] | 32554.345 [31202.619–35148.168] | +0.7334% | [-5.7972%, +8.8973%] | **Fail** | **Fail** |

Absolute paired B-minus-A median differences ranged from -669.324 to +176.557 ns
for RP, -2934.794 to +933.915 ns for WP, and -2053.431 to +3352.872 ns for RPM.
The instruction/allocation gates pass; RP's native gates pass; WP and RPM fail
the stated native qualification. **Overall: not accepted.** There is no claimed
platform-wide speedup, physical-network qualification or statistically established
causal RPM regression. A future experiment would require a separately chosen
scope and protocol; none was added to this dataset.

## Correctness and retained failures

Before timing, A passed 4,105 scoped tests and B passed 4,108, each across client,
server, endpoint-core and encoding with all features. Each passed 43 doctests.
B passed strict affected-crate Clippy/private rustdoc, two existing no-allocator
codec exchange tests and the existing envelope-layout budget test. Relevant
transaction cancellation/segmentation/admission and future-size tests are in the
scoped suites. These checks describe the historical A/B sources, not a new Cargo
gate for the final evidence-only delivery head. The codec subset's zero-allocation
contract does not describe the hosted round trip above.

The first Docker build failed on a sandboxed `~/.docker/buildx` activity-file
write. A task-local `BUILDX_CONFIG` allowed the authorized derivative build;
the shared image was unchanged. Initial external-harness resolution was explicitly
unlocked: it added only `rp-harness`, removed 186 unused workspace records and
changed no retained version/checksum. A subsequent `--locked` build succeeded.
The first worktree file write and an early process read were also sandbox-denied;
reviewed authorized retries succeeded before timing. No performance run failed,
was replaced or was discarded. `failures.json`, `image-build-failure.log`,
`lock-normalization.json` and `commands.json` retain classifications and evidence.

## Distributed and local evidence

- `inputs.json`: original full A/B source hashes, lock/harness/profile identities,
  image/compiler/environment records and exact native/Linux executable hashes.
- `runs.json`: all 48 Callgrind totals/counters and all 12 native invocation
  records, exact commands/statuses/timestamps, original stdout/stderr and power.
- `criterion.jsonl`: original bytes and SHA256 hashes of all 36 estimates and
  sample files, containing all 3,600 positive samples. `summary.json` contains
  every invocation median, paired difference/range and acceptance result.
- `candidate.patch.gz`, `patch-reconstruction.json`: the rejected candidate and its
  exact eight-file reconstruction proof. `common/` holds the unchanged harness,
  normalized lock, profile, protocol and derivative Dockerfile.
- `measured/`: byte-identical original runners and analysis. Corrected top-level
  reproduction tools own fresh source-bound builds, require those captures and
  execute an immutable Docker image ID. They intentionally differ from the
  historical scripts; the original measurements are unchanged.
- `profiles/`: compressed original exploratory A and selected final B ct profiles,
  with raw/compressed hashes in `provenance.json`; `attribution.json` identifies
  the reached frame. Complete original profiles and binaries remain at the local
  evidence path in provenance, approximately 70 MiB of output. They are not an
  uploaded/downloadable artifact. Root preserves that archive before cleanup.
- `sha256.json`: distributed artifact hashes. `reconstruct.py` verifies original
  bytes, source identities, run order/status, per-run counters, sample counts and
  reconstructs the complete frozen result without compiling or timing.

The isolated integrity challenge reseals outer checksums after changing one metric,
source identity or sample, so rejection exercises semantic guards. All three
corruptions were rejected, and the untouched data passed before/after.
`integrity-validation.json` retains these expected failures. The capture helper
also accepted exact A/B source and rejected a wrong revision before restoration.
That historical source-only check did not establish build/artifact association.
`test_capture.py` and `reproduction-validation.json` cover the corrected fresh-build
path: 21 isolated controls mock Git/compiler/Docker processes while exercising real
source hashing, artifact parsing, resource admission, logging and publication.
Wrong-source/stale build metadata, reused targets/volumes, changed source, failed
builds and image-tag drift are covered. No new build or timing is claimed by these
controls. The original capture misbinding and mutable-tag execution were reproduced
before repair; their expected failing evidence is retained separately.

Run from any directory with Python 3.10+:

```sh
python3 /absolute/path/to/1978-macaddr/reconstruct.py
python3 /absolute/path/to/1978-macaddr/validate_integrity.py
python3 -B /absolute/path/to/1978-macaddr/test_capture.py
```

## Opt-in reproduction of new measurements

Reconstruction above uses the published observations. The following starts a
separate experiment, not a favorable retry of this dataset. Use clean isolated
checkouts at A and B. The original B commit is retained in history;
`candidate.patch.gz` contains its exact intervention.

`capture.py` now owns the pinned, locked builds. It accepts neither a prebuilt
executable nor independently supplied Cargo JSON. For each label it reserves a
new native target and named Linux volume, verifies the complete source manifest,
runs the builds from that root, checks their successful Cargo artifact records
against the exact manifest/source paths, and rechecks source and common inputs
before publishing the variant. Existing resources and cross-label reuse are
refused, even when an earlier failed attempt reserved them. A volume ownership
label checks the create race. Independent builds with equal binary bytes are valid.

The native command is `cargo +1.99.0 bench --locked -p bacnet-benchmarks --bench
bip_latency --no-run --message-format=json`, with `CARGO_INCREMENTAL=0` and the
fresh target. Linux runs `cargo build --release --locked --message-format=json`
inside the selected image, with the frozen `common/harness` and its lock mounted
read-only at `/h`, the verified source at `/wt`, and a fresh `/target` volume.
These are opt-in commands executed by the capture tool; they are not prerequisites
for checking the published observations.

Supply an already prepared local image with Rust/Cargo 1.99.0 and Valgrind 3.22.0.
The original derivative is identified in `inputs.json`; it requires the retained
local base `sha256:0f6ec7193b18244427fba499eaf40ff35be670e08b327c5f689b1a6eb9c5f5ee`,
not a public registry download. `common/image/Dockerfile` is the frozen historical
recipe. If preparing a new derivative, pin its base by immutable ID and record
the resulting image as a new environment. Do not assume that the recipe's mutable
base tag still names the original base.

The first capture resolves `IMAGE` once to a complete immutable image ID. Both
variants use that ID for version probes, builds and executable hashing; the
Callgrind matrix executes the same ID. Rebinding the original tag later does not
change execution. The helper records actual new binary hashes and platform/tool
observations, not a claim that another machine reproduces the original bytes.

Set `EVIDENCE` to this directory, `FROZEN` to a fresh absolute capture directory,
and `A_TREE`/`B_TREE` to the clean immutable checkouts. `A_NATIVE`/`B_NATIVE` must
name absent private target paths outside the sources; `A_VOLUME`/`B_VOLUME` must
name absent, distinct Docker volumes. Use the same `IMAGE` selector for both calls.
Run them serially:

```sh
python3 -B "$EVIDENCE/capture.py" A "$A_TREE" "$A_NATIVE" "$A_VOLUME" \
  "$FROZEN" --image "$IMAGE"
python3 -B "$EVIDENCE/capture.py" B "$B_TREE" "$B_NATIVE" "$B_VOLUME" \
  "$FROZEN" --image "$IMAGE"
```

Each preparation retains its reservation, exact commands, explicit environment
overrides, stdout/stderr/statuses and successful build receipt under
`preparations/A` or `preparations/B`. A failed build never falls back to a prior
executable and never publishes its variant. Failed logs and allocated resources
remain for inspection; use a fresh output and fresh resources for another attempt.
The receipt checks bracket the builds; they are not a filesystem snapshot or a
complete capture of the inherited native build environment.

Before timing, run the recorded correctness checks in separate validation targets
and the non-timing ct/mt RP/WP harness oracles. Any later binary change invalidates
its capture. Check actual OS/toolchain/power/process/container state and at least
50 GiB free. Use no other builds or benchmark suites during the fixed serial matrix:

```sh
python3 -B "$EVIDENCE/callgrind_matrix.py" "$FROZEN"
python3 -B "$EVIDENCE/native_matrix.py" "$FROZEN"
python3 -B "$EVIDENCE/analyze.py" "$FROZEN"
```

Both runners require the corrected capture format, two distinct build resources,
unchanged receipts and unchanged reproduction tools. They refuse existing output
or baseline names, stop on execution failure, retain raw data and check frozen
source/binary identity. Linux checks the binary before each invocation and executes
only the selected image ID. Keep the resulting new data separate, including any
failures or uncertainty. No new measurements are needed to verify this report.
