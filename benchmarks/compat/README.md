# Normalized B/IP comparison for #1979

`normalized-clients` runs the same B/IP setup, preflight, request loop, accounting,
counter snapshots and cleanup as the current `stress-test clients` scenario.
All three normalized binaries compile the driver as local modules, keeping the
same crate boundary instead of comparing local code to library calls.
Its command shape is `normalized-clients clients --transport bip --objects 100
--duration 10 --steps 100`. Invalid arguments or incomplete runs produce JSON
with `clients_diagnostics.valid=false` and exit with status 1 after cleanup.
Request failures alone do not invalidate a complete measurement.

To qualify an older checkout without changing its production crates, run:

```sh
python3 benchmarks/compat/bootstrap.py /absolute/historical/checkout --adapter v013
# Use --adapter current for f056aba22cff95ec98f77511238711a93ce82da4.
```

The `--adapter current` path tracks dev's API, while `v013_adapter.rs` is
frozen against the v0.13.0 API, so leave it unchanged when dev's API moves.

The generated `benchmarks/src/bin/normalized-clients.rs` includes these exact
candidate source files by absolute path. It never copies the whole benchmark
tree. Keep the candidate available during builds and record all included source
hashes. The generator refuses to overwrite a previous bootstrap. The historical
checkout keeps its lockfile, dependencies and private target directory. Build
all sides with the same explicit toolchain and `CARGO_INCREMENTAL=0`.

The v0.13 adapter supplies its earlier Device configuration and numeric error
values. It explicitly sets Object_List and server vendor 555 to match the current
fixture's intent. Both adapters keep the same loopback binding, ephemeral ports,
100 Analog Inputs, AI1 Present_Value 20.1, five-second timeout and default three
retries. This establishes the measured value contract, not equivalence of every
Device property. No production API is replaced or patched.

Run the historical bootstrap's tests with Tokio's test-only paused-clock feature:

```sh
cargo +1.99.0 nextest run --locked -p bacnet-benchmarks \
  --bin normalized-clients --features tokio/test-util
cargo +1.99.0 build --locked --release -p bacnet-benchmarks \
  --bin normalized-clients --message-format=json
```

The current package enables that same feature in dev dependencies. Record the
Cargo artifact features/profile for each build; release stress builds do not
request test-util. `encoding_guard.rs` checks independently derived literal
NPDU, ReadProperty and ACK bytes outside measurement. Every B/IP step also checks
its live first read's object, property, array index and application Real bytes
before the clock and baseline counters. The timed request loop keeps the existing
successful-call histogram behavior.

All clients connect before one shared deadline. The elapsed denominator ends at
the last request-loop completion, including an in-flight request's retry tail;
setup, counter serialization and shutdown are excluded. Failure timing is separate
from successful quantiles. Missing or duplicate client/step results and ordinary
setup, join, stop or counter failures invalidate the run. A panicked worker owns
its client, so its explicit stop cannot be proven; this is a failed run, never a
cleanup-success claim. Server counter endpoints are sampled before server stop,
after client loops and their cleanup; client endpoints precede each client's stop.
Active samples are endpoints, not peaks. `send.npdu_too_long` counts size refusals,
not general socket errors. Kernel UDP drops are outside the transport counters.

Do not compare the archived v0.13 stock stress harness directly with this driver:
its setup timing and recorder differ. Keep the original encoding workload and
its allocation/copy operations unchanged, and pass `--bench` when invoking its
Criterion executable directly. The #1979 run-level dataset records the fixed
balanced matrix, commands, hashes and remaining limitations.
