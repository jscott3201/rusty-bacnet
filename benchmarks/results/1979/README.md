# #1979 repeated measurement evidence

This directory records a same-machine comparison on 2026-10-21. It accompanies
[Benchmarks.md section 10.1](../../../Benchmarks.md#101-repeated-comparison-for-1979).
It is part of #1979; causal attribution and #1978 optimization remain open.

| Label | Production revision | Benchmark adapter |
| --- | --- | --- |
| A | `dca94f7fa5acce74423606bf526de56d1f4360cf` (v0.13.0) | `compat/v013_adapter.rs` |
| B | `f056aba22cff95ec98f77511238711a93ce82da4` | current adapter |
| C | `5b6f541322cac7b346e8fdb749689b0c5c4b2239` | current adapter |

C's measured benchmark overlay is pinned to
`4d5ccc2121297e6abe0d3442ccdd564f95d8b34d`, with source hashes recorded before
measurement. A/B include only a generated normalized-clients entrypoint, with
absolute includes of that
same overlay and the selected adapter. Production files and all three lockfiles
are unchanged. A root comparison of the benchmark's transitive inputs confirmed
that the intervening TUI change from research base `7d77d81c` to C did not affect
them. The normalized B/IP driver and original encoding workload sources remain
unchanged. A later B/IPv6 setup-diagnostic correction changes clients.rs and adds
a private helper/regression; it does not run in those timed paths. The package
library can still compile clients.rs, so this does not claim an identical final
compilation closure or byte-identical rebuilt executables. Original source hashes
and frozen binaries are preserved; the final tree is not the measured snapshot.

## Files and interpretation

- `encoding.csv`: one row per workload per invocation, including the six A/C
  primary runs and three A/C constituent runs. `median_ns` is Criterion's
  `estimates.json` median point estimate, not its fitted slope or a pooled sample.
- `clients.csv`: every sanity-curve step and every 100-client repeat, with error
  counts, successful-only quantiles, completion duration and diagnostic counters.
  `failure_elapsed_secs` sums elapsed failed calls across clients and can exceed
  wall-clock duration. `failure_max_secs` is the longest failed call in the step.
- `runs.json`: all 51 invocations in order, exact arguments, UTC timestamps,
  load/space observations, status and stdout/stderr hashes.
- `provenance.json`: compiler, environment, original locks, artifact hashes,
  measured-source hashes and feature/profile controls with residuals. Its
  derived_csv_sha256 values verify the distributed LF bytes; original generated
  CRLF files/hashes are separately retained in the local raw archive.
- `raw-sha256.json`: hashes for the unedited raw manifests, stdout/stderr and
  Criterion estimates/samples. The local archive path is in provenance; those
  large local-only files and binaries are not a public artifact download.
- `summarize.py`: recomputes median/range and paired comparisons from these CSVs.
  Run with Python 3.10 or later. It checks distributed CSV byte hashes, row
  counts, client accounting, error denominators and the observed admission correspondence before summarizing.

This is an issue-specific dataset, not a change to the existing result-row JSON
schema. Run `python3 benchmarks/results/1979/summarize.py` to inspect its summary.
Each complete invocation is the comparison unit. Pair equal per-revision run
ordinals: the fixed order puts those runs next to one another. The curves are
sanity checks, excluded from the ten-repeat statistics. No failed or zero-error
repeat was discarded; all planned invocations exited successfully and every
stress/curve step was valid.

## Reproduction

Prepare isolated A/B checkouts at the production revisions above. For exact
measured-source reproduction, check out C at the immutable measured overlay
commit `4d5ccc2121297e6abe0d3442ccdd564f95d8b34d`; its production inputs are
those of `5b6f5413`. Use that overlay's source files and compatibility generator,
not the later repaired final tree. Keep a private target directory per checkout.
Use [the compatibility instructions](../../compat/README.md) to generate A/B wrappers.
Qualify all common-driver tests and independent encoding/value guards first.
A's original benchmark library has no tests; its original nextest no-tests exit
is retained as unavailable evidence, not a passing test suite.

Build serially with Rust/Cargo 1.99.0, `CARGO_INCREMENTAL=0` and `--locked`:

```sh
# In each A/B/C checkout, after generating the selected wrapper:
cargo +1.99.0 build --locked --release -p bacnet-benchmarks \
  --bin normalized-clients --message-format=json
# In A only, align the already-existing Tokio edge with C's bench dev feature:
cargo +1.99.0 bench --locked -p bacnet-benchmarks --bench encoding \
  --features tokio/test-util --no-run --message-format=json
# In C:
cargo +1.99.0 bench --locked -p bacnet-benchmarks --bench encoding \
  --no-run --message-format=json
```

Resolve executables from Cargo's JSON compiler-artifact records and immediately
copy/hash them into immutable paths. A later Cargo invocation can overwrite a
`target/release` alias. Create a local inputs JSON with `roots` (A/B/C paths),
`artifacts` (A/C-encoding and A/B/C-normalized, each path and sha256), and
`source_hashes` (absolute path to sha256); retain revisions and lock hashes too.
The archived original inputs file is separately hashed in provenance.

With no competing builds or benchmarks, AC power and at least 50 GiB free:

```sh
python3 benchmarks/results/1979/run_matrix.py /absolute/inputs.json /new/output
# Only after interpreting that matrix and approving the constituent follow-up:
python3 benchmarks/results/1979/run_constituents.py /absolute/inputs.json /new/constituents
```

The controllers refuse an existing output directory or Criterion baseline and
stop on the first failed/invalid invocation. They verify source and executable
hashes before/after the experiment and executable hashes before each invocation.
The primary order is ACCA three times, then one A/B/C curve, then ABCCBA five
times. The approved constituent order is AC, CA, AC. All Criterion invocations
pass `--bench`, use 100 samples, a 3-second warm-up and 5-second measurement per
workload. Stress uses a 10-second submission deadline, five-second timeout and
three retries, with the completion tail included. Client setup, live independent
value preflight, counter serialization and shutdown occur outside the clock.

## Limits

The comparison preserves original dependency graphs rather than claiming whole
binary equivalence. Critical inspected runtime versions/features/profiles match;
additional TLS/parser/host features and libc extra_traits remain disclosed in
provenance. Feature reachability review found no use in the selected timed
codec/plain-BIP operations, but code layout and other indirect effects are not
excluded. The original historical stress harness used different timing and
histogram ownership, so its reported percentages are not normalized controls.

All terminal request failures in the fresh dataset were Abort reason 9
(`OUT_OF_RESOURCES`), with exact per-step admission-counter correspondence.
Clause 18.10 of ASHRAE 135-2020 distinguishes that reason from transaction timer
expiry; the code maps it in `AbortReason` and emits it on capacity refusal.
No terminal Timeout or TSM_TIMEOUT result was observed. Internal retry deadlines
remain possible before an eventual success or Abort. Fallback responses were
dropped, failed calls sometimes exceeded five seconds, and complete steps ran
as long as fifteen seconds. Zero observed user-space B/IP receive counters does
not establish zero kernel loss. The size-refusal counter `npdu_too_long` is not
a general socket-send error counter. Active counter endpoints are not peaks.

No OS UDP sampling, Linux repetition, causal commit bracket or production tuning
was performed. The NPDU decode attribution, exact composite-encode cause and
B-to-C admission-refusal rate cause remain open. Standalone component timings
cannot be added to explain a composite with different buffer sizes and copies.
