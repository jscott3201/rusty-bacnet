# NPDU owned-decode attribution for #1979

The historical child decoder's borrowed-view/owned-reconstruction strategy costs
about 23.4% more than its parent on this steady shared-`Bytes`, 100-byte-payload
workload. Adapting the parent's direct parser to the child's representation and
error contract removes that observed increase. This is an evidence-only result
for one compiler, target and input; it does not establish a production fix.
It accompanies [Benchmarks.md section 10.2](../../../Benchmarks.md#102-npdu-owned-decode-attribution).
The earlier [#1979 comparison](../1979/README.md) remains unchanged.

## Controlled comparison

| Label | Historical revision | Scratch overlay |
| --- | --- | --- |
| P | `eab0b4e727fad0dc27ed75aecd3f8458eebbb191` | Parent decoder plus common diagnostics/oracles |
| Q | `a5192b89a37771790fb236f1d349b23e23ee8e69` | Child view adapter plus the same diagnostics/oracles |
| R | `a5192b89a37771790fb236f1d349b23e23ee8e69` | Q with parent's parsing algorithm adapted to Q types/errors |

Q/R differ in exactly `crates/bacnet-encoding/src/npdu.rs` among their 2,829
frozen source files. R keeps Q's public owned/error types, warning placement,
priority/message conversion, validation order, overlong-address source recovery,
address ownership and `data.slice(offset..)` payload ownership. It replaces the
borrowed-view call and reconstruction with direct parsing. R is an experimental
control; its patch is never applied to the current production source.

P/Q also differ in representation and error ABI. All owned results are 128 bytes
with alignment 8, but P retains an independent network-message boolean and its
error is 64 bytes versus Q/R's 48. Equal result size does not make P/Q an ABI
control. Q/R fixes those differences while comparing compiled parsing strategies;
it does not isolate a single instruction or layout effect.

All three use original locks with only the existing local `bacnet-npdu` edge
added to the benchmark package for diagnostics. Registry package records are
identical. All 217 compiled units' versions, features, editions and profiles match
across P/Q/R, preserving separate host/target profiles. Exact original/overlay
lock hashes and compiler commands are in `provenance.json` and
`build-inspection.json`; the original locks are recoverable from the revisions.

The machine was an Apple M5 (10 cores, 16 GiB), macOS 27.2 (26B5112a),
`aarch64-apple-darwin`, Rust/Cargo 1.99.0 (rustc `b940084d7`, LLVM 23.1.1).
Builds were serial, locked, with private targets and `CARGO_INCREMENTAL=0`.
The selected rustc commands use optimization level 3, no embedded bitcode and
stripped debug information; no injected Rust flags or compiler wrappers were set.
The fixed measurement ran on AC power after builds stopped, with disk/load/power
observations recorded per invocation. There is no CPU affinity or frequency lock.

## Timing boundary and correctness

The original owned timed expression remains
`black_box(decode_npdu(data.clone()).unwrap())`, including result destruction.
Setup retains a `Bytes` containing `01 00` followed by 100 zero bytes. An untimed
common oracle checks every header/default field, payload bytes, offset 2 and
payload pointer sharing against that independent literal, for both owned and
borrowed decoders. Its clone promotes the retained allocation to shared ownership
before timing on every variant. This measures steady shared input, not first
promotion, fresh allocation or a uniquely owned input path.

Two separately labelled diagnostics measure borrowed-core decode and shared-buffer
clone/drop. Their work remains inside the loops in the measured disassembly.
They neither measure cached-view reconstruction nor form additive components of
the owned workload; subtracting their times would not isolate reconstruction.

Before timing, each variant passed 688 existing codec tests and 21 doctests with
zero skipped tests. Independent guards passed 4 tests on P and 7 on each Q/R.
The Q/R guards cover routed proprietary values, exact rejection ordering,
truncated prefixes and overlong-address recovery. These are bounded correctness
checks, not exhaustive equivalence over all packets. The first preparation build
failed because a copied integration test lacked a crate-level doc comment. The
failure and diagnostic history are retained in provenance; adding a documented
wrapper around the unchanged guard resolved it. Timed sources did not change,
and P's measured binary stayed byte-identical. No failed timing run was discarded.

These are historical experiment validations. The delivery branch changes only
this evidence directory, report and changelog; it does not claim a new current-head
Cargo validation or use historical tests as current production build evidence.

## Results and inference

The predetermined order was `P Q R R Q P` repeated three times: six complete
invocations per variant. Each invocation ran all three workloads, each with 100
Criterion samples, three seconds of warm-up and five seconds of measurement.
All 18 invocations succeeded, giving 54 estimates and 5,400 samples. Frozen source
and executable hashes matched before/after the experiment and each executable
was checked before its invocation. All planned observations are included.

Values are median [minimum–maximum] of the six invocation medians, in ns:

| Workload | P | Q | R |
| --- | --- | --- | --- |
| Owned decode | 11.899 [11.839–11.940] | 14.662 [14.651–14.725] | 11.925 [11.725–12.008] |
| Borrowed decode diagnostic | 4.513 [4.470–4.516] | 4.511 [4.506–4.513] | 4.513 [4.510–4.516] |
| Shared clone/drop diagnostic | 3.530 [3.518–3.533] | 3.531 [3.528–3.542] | 3.539 [3.528–3.549] |

Pair equal per-variant invocation ordinals, which lie in the same forward/reverse
three-run block. Owned Q/P ratios have median 1.234104 and range
1.228783–1.238081; R/Q has median 0.812784 and range 0.800241–0.817901;
R/P has median 0.999837 and range 0.988674–1.010062. These are ratios of matched
invocation medians, not a ratio of pooled samples or a confidence interval.
Borrowed and clone/drop diagnostic paired medians remain near one; their full
ranges, including the first P borrowed observation, remain in `summary.json`.

The exact measured Q binary calls the borrowed decoder out of line, materializes
its 88-byte result and then reconstructs the owned result. R parses directly into
the same Q owned/error representation. Owned function stack reservations are
336/368/304 bytes for P/Q/R. Q/R's successful caller instructions match after
relocation normalization; the original clone, result handling and drop remain.
Both retain slice-related ownership bookkeeping. The separate untimed layout
probe used the exact build rlibs; assembly extracts come from the measured
executables themselves, not a rebuilt probe. Function-wide instruction counts
include other branches and are not executed-path costs.

Together the controlled Q/R difference and generated code support attributing
the historical increase to the compiled view-adapter strategy under Q's ABI.
They do not prove that the extra call, result size, reconstruction or stack frame
alone causes a particular number of nanoseconds. R also changes code placement;
no single-instruction intervention or production inlining experiment was run.

The earlier current-stack observation was 16.061 ns versus v0.13's 11.854 ns.
This historical Q is 14.662 ns in a separate experiment; it cannot account for
that entire later gap or be subtracted across datasets to quantify a residual.
Later-current-head decode changes, full ReadProperty encoding and admission
refusal frequency remain open under #1979. No production tuning, unique-input
claim, physical-network measurement or cross-platform conclusion is made here.

## Evidence files

- `encoding.csv`, `summary.json`: every invocation's estimates and all paired
  summaries. The primary statistic is Criterion's median point estimate.
- `criterion.jsonl`: all 54 original estimates/sample file byte strings. The
  unmodified bytes hash to `raw-sha256.json`; every sample is distributed here.
- `runs.json`: exact order, arguments, timestamps, status, output hashes and host
  observations. `raw-sha256.json` also records local stdout/stderr and manifest
  hashes; those logs and executable files are local artifacts, not public downloads.
- `provenance.json`: immutable revisions, source/lock/binary hashes, exact build/test
  commands, environment, preparation failures and local evidence locations/hashes.
  Full source manifests remain in the recorded local archive; immutable revisions
  plus the distributed overlays reproduce the measured source. Delivery base
  reconciliation changed only Python sample coverage, website prose and changelog.
- `build-inspection.json`: every compiled unit's features/profile and selected
  exact rustc argument vectors. P's cached final build is linked to its first
  build's actual commands by equal executable hashes.
- `codegen/`, `assembly-inspection.json`, `layout-inspection.json`: exact measured
  function extracts, their hashes/calls/stack adjustments, and untimed layout data.
- `overlays/`: tracked patches plus added common guard/test sources and expected
  hashes. They apply only to scratch historical checkouts, never production.

Run `python3 benchmarks/results/1979-npdu/summarize.py` with Python 3.10+.
It checks distributed hashes, every raw observation against its original hash,
run identity/order, all 100 positive finite samples per observation, exact CSV
reconstruction and every summary/paired ratio. No timing run is repeated.

## Reproduction on macOS

Create three isolated scratch checkouts at the exact revisions above, with private
target directories. Set absolute paths `P`, `Q`, `R`, `EVIDENCE` (this directory)
and `FROZEN` (a new capture directory). Prepare each once:

```sh
python3 "$EVIDENCE/prepare.py" P "$P"
python3 "$EVIDENCE/prepare.py" Q "$Q"
python3 "$EVIDENCE/prepare.py" R "$R"
```

For each label/root in P, Q, R order, run the following with `LABEL` and `ROOT`
set accordingly. Keep command logs outside the source checkout. Capture the
binary immediately after building, before another Cargo command can replace it:

```sh
cd "$ROOT"
export CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$ROOT/target"
cargo +1.99.0 bench --locked -p bacnet-benchmarks --bench encoding \
  --no-run -vv --message-format=json > "/tmp/$LABEL-npdu-build.json" \
  2> "/tmp/$LABEL-npdu-build.err"
python3 "$EVIDENCE/capture.py" "$LABEL" "$ROOT" \
  "/tmp/$LABEL-npdu-build.json" "$FROZEN"
cargo +1.99.0 nextest run --cargo-profile release --locked \
  -p bacnet-npdu -p bacnet-encoding
cargo +1.99.0 nextest run --cargo-profile release --locked \
  -p bacnet-benchmarks --test encoding_guard --test npdu_oracle
# Q/R additionally:
cargo +1.99.0 nextest run --cargo-profile release --locked \
  -p bacnet-benchmarks --test npdu_child_oracle
cargo +1.99.0 test --release --locked --doc -p bacnet-npdu -p bacnet-encoding
xcrun llvm-objdump --demangle --disassemble --no-show-raw-insn \
  "$FROZEN/$LABEL-encoding" > "/tmp/$LABEL-npdu-encoding.asm"
```

Use cargo-nextest 0.9.145 or newer. Stop if any build/test fails; preserve its logs
before repairs. Check generated code and feature/profile equality using the
recorded evidence as the comparison contract. Rebuilding at different paths may
change binary hashes or layout; verify and freeze the newly built artifacts rather
than expecting identity with this machine's binaries. `capture.py` checks overlay
identity, records full source hashes and refuses to overwrite an existing label.

With all three captures qualified, no competing build/benchmark, AC power and at
least 50 GiB free, run one matrix into a new output directory:

```sh
python3 "$EVIDENCE/run_matrix.py" "$FROZEN/inputs.json" /new/npdu-output
```

The frozen runner is byte-identical to the one used here. It checks inputs before
and after, refuses reused output/baseline names, records all failures and stops on
the first failure without retry. The preparation/capture helpers are packaging
utilities; the measured overlays, binaries and runner were frozen before timing.
