# CI and merge evidence

The repository, its pull requests and its CI live on
[GitHub](https://github.com/jscott3201/rusty-bacnet), and every CI job runs on
GitHub-hosted runners, the [release](#release) included.

| Host | Runs |
| --- | --- |
| GitHub (hosted runners) | Linux CI, [`.github/workflows/ci.yml`](../.github/workflows/ci.yml); [native macOS and Windows tests](#native-tests-macos-and-windows), [`native-tests.yml`](../.github/workflows/native-tests.yml); the nightly [flake hunt](#flake-hunt), [`flake-hunt.yml`](../.github/workflows/flake-hunt.yml); [fuzzing](#fuzzing) of the MS/TP and B/IP codecs, [`fuzz.yml`](../.github/workflows/fuzz.yml); website validation and the GitHub Pages deploy, [`docs-pages.yml`](../.github/workflows/docs-pages.yml); [releases](#release), built and tested on each platform and published to crates.io, PyPI and GitHub Releases, [`release.yml`](../.github/workflows/release.yml) |

| Platform | Where it is checked |
| --- | --- |
| Linux amd64 | CI: lint, clippy, rustdoc, tests, Python bindings, MSRV, audit and deny |
| macOS arm64 | Native tests: tests, doctests, clippy, rustdoc, Python bindings. CI: clippy and rustdoc for each published crate with default features, cross-checked |
| Windows x86_64 (MSVC) | Native tests: tests, doctests, clippy, rustdoc, Python bindings. CI: clippy and rustdoc for each published crate with default features, cross-checked |

A PR merges only when both required checks are green on its head SHA: `CI OK`
from `ci.yml` and `Native OK` from `native-tests.yml`, each of which passes
only when every job under it did (see [Merge evidence](#merge-evidence)).

## Pipeline

| Job | PR to `dev` | PR to `main` | Push to `dev` (merge) | Push to `main`, `v*` tag, weekly, manual |
| --- | --- | --- | --- | --- |
| Lint: rustfmt, 700-LOC cap, no-secret scan, script regressions, the `named_codes!` copies, changelog fragments, the [tool pins](#tool-pins), the [fuzz crates' lockfiles](#fuzzing) | ✓ | ✓ | ✓ | ✓ |
| Clippy and rustdoc, warnings denied: every feature, PyO3 crate, `bacnet-cli` without default features, each published crate with default features for Linux, Windows and macOS, the `no_std` crates on bare-metal targets and their `alloc` tests on the host; the every-feature and PyO3 rustdoc runs include private items. The workspace runs include the [samples](#samples). Also the [firmware link test](#firmware-link-test): the MS/TP, B/IP and MS/TP device images built without an allocator, and the APDU image with one, checked against their size budgets | ✓ | ✓ | ✓ | ✓ |
| Test: Linux, every feature (`LINUX_FEATURES`); also the [`no_std` crates](#no_std-checks)' tests and doctests with `alloc` and without `std`, and their tests without either | ✓ | ✓ | ✓ | ✓ |
| Python bindings: `maturin develop` (maturin at its [pinned](#tool-pins) version), then `python -m unittest discover -s crates/rusty-bacnet/tests` and the crate's Rust tests (`cargo nextest run -p rusty-bacnet`) | ✓ | ✓ | ✓ | ✓ |
| MSRV (`RUST_MSRV` in [`.github/ci-pins.env`](../.github/ci-pins.env)), Linux native (`check-msrv.sh --linux-native`), the `no_std` builds included |  | ✓ |  | ✓ |
| Cargo Audit + Cargo Deny |  | ✓ |  | ✓ |
| **CI OK**: fails if any job above failed | ✓ | ✓ | ✓ | ✓ |
| Plan: whether a merge can [skip the Lean jobs](#skipping-identical-merge-runs) |  |  | ✓ |  |
| Prune caches, after a green run on `dev` ([Caches](#caches)) |  |  | ✓ | weekly, and manual on `dev` |

`CI OK` is the required check. MSRV and Cargo Audit + Deny count as passing
when their tier skips them; a skipped Lean job fails it.
[`docs-pages.yml`](../.github/workflows/docs-pages.yml) validates the website
(Astro checks, unit tests, production build and Chromium tests) on PRs that
change `website/**`.

Merge pushes to `dev` run the Lean jobs, for two reasons (#904), unless
the merge could have been a fast-forward, so the PR's runs tested its tree
([Skipping identical merge runs](#skipping-identical-merge-runs)).

- **Caches.** Only runs on `dev` save the Rust caches. A cache saved by a
  `pull_request` run would be scoped to that PR, and a PR run restores from
  its base branch and the default branch (`dev`), so `dev`'s caches seed
  every PR.
- **Merge result.** A PR run tests the PR head merged into its base as the
  base stood when the run started (GitHub's `refs/pull/<n>/merge`). With the
  repo's merge commits, the `dev` run is the only test of what actually
  landed, after any merges since.

Each push to `dev` gets a concurrency group of its own, keyed by its commit,
and nothing cancels its run, as in the
[native tests](#native-tests-macos-and-windows): two merges minutes apart each
get a result, and a late duplicate push event for an older commit can't
cancel the run for `dev`'s head. Every other run is grouped by event and ref,
so a newer push to a PR, to `main` or to a tag, or a newer scheduled or manual
run on a branch, cancels the one it supersedes.
**After merging, check the `dev` run.** It isn't a required status, so a red
merge run is the only signal of merge skew; fix it forward on `dev` right away.
The weekly scheduled run checks the default branch (`dev`) with the Heavy jobs
too.

Rust caches are keyed per job on the toolchain, `Cargo.lock`, the manifests,
and, for Clippy and Test, `LINUX_FEATURES` (Clippy also on
`DEFAULT_FEATURES_TARGETS`, `NO_STD_TARGETS` and `FIRMWARE_TARGETS`). Only a job that succeeded
on `dev` saves its cache, so a failed or cancelled run can't leave a partial
cache that later runs restore by exact key. The MSRV job's cache comes from the weekly run, the Heavy run on
`dev`. A new push to a PR cancels its superseded run.

The workflow sets `CARGO_INCREMENTAL=0` and drops native debug info from dev and
test builds (`CARGO_PROFILE_{DEV,TEST}_DEBUG=0`) to cut codegen, link time and
cache size. Optimization level, debug assertions, overflow checks and test
selection keep their defaults, and there is no `RUSTFLAGS=-Dwarnings`: per-rule
severity lives in `[workspace.lints]`.

Tests run with [cargo-nextest](https://nexte.st), which gives each test its
own process. Its settings live in [`.config/nextest.toml`](../.config/nextest.toml);
CI uses the `ci` profile, and nextest does not run doctests, so a separate
`cargo test --doc` step covers them. The tests that take 3.5 s or more have a
`priority` there, so they start first instead of running alone at the end; the
file says how to refresh that list. The Linux test commands are below, with
`$LINUX_FEATURES` as set in `ci.yml`: every optional feature that builds on
Linux, including per-crate ones such as `bacnet-endpoint/sc-tls` and
`bacnet-cli/pcap`. Then come the `bacnet-cli` tests with default features,
because a few exist only when `sc-tls` or `pcap` is off, and with no default
features, the lean CLI without the TUI (`tui` is a default feature since
#974), whose own tests cover plain `bacnet` and `bacnet tui` without it.
Only Linux runs the lean CLI's tests; macOS and Windows lint it with clippy
in the native-tests workflow (and `local-macos.sh` runs both on a Mac). The last steps
run the tests and doctests of `bacnet-types`, `bacnet-encoding` and
`bacnet-services` with `alloc` and without `std`, the code their bare-metal
builds compile, which the runs before leave out (see
[`no_std` checks](#no_std-checks)).

```bash
cargo nextest run --workspace --exclude rusty-bacnet --locked --features "$LINUX_FEATURES" --profile ci
cargo test --doc --workspace --exclude rusty-bacnet --locked --features "$LINUX_FEATURES"
cargo nextest run -p bacnet-cli --locked --profile ci
cargo nextest run -p bacnet-cli --no-default-features --locked --profile ci
cargo nextest run -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --features alloc --locked --profile ci
cargo test --doc -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --features alloc --locked
cargo nextest run -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --locked --profile ci
cargo test --doc -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --locked
```

The Python job builds the PyO3 extension in debug mode into a fresh venv with
Ubuntu 24.04's own Python (3.12) and runs the unittest suite. The SC tests
generate certificates with the `openssl` CLI. Cargo Deny covers the bindings'
dependencies too; only `bacnet-benchmarks` is excluded.

The same job then runs the crate's Rust tests, its lib unit tests and
integration tests, with `cargo nextest run -p rusty-bacnet --locked --profile ci`.
The workspace doesn't turn on pyo3's `extension-module` feature, so these test
binaries link libpython (#919); maturin turns the feature on for wheel and
`maturin develop` builds from `crates/rusty-bacnet/pyproject.toml`. Linking
needs the interpreter's shared library and its unversioned `.so` symlink, which
the job installs with `libpython3-dev`. The workflow sets
`PYO3_PYTHON=/usr/bin/python3`, so every PyO3 build uses the apt Python that
package matches, whatever else is on `PATH`, and the Rust tests don't depend
on the venv from the maturin step. Tests that call into Python start the
interpreter with `Python::initialize()` first, since nothing enables pyo3's
`auto-initialize`.

Use cargo-nextest 0.9.145 or later locally. Older releases on macOS could
mark unrelated passing tests as leaky (#751), and the configuration warns
about them.

### Skipping identical merge runs

A PR run tests `refs/pull/<n>/merge`: the head merged with `dev` as `dev`
stood when the run started. When the PR head already contains `dev`'s
current tip, so that the `--no-ff` merge could have been a fast-forward,
every `dev` a PR run used is in the head's history (`dev` only moves
forward), each of those runs tested exactly the head's tree, and the merge
has that tree too. So on a push to `dev`, a **Plan** job in each workflow
([`scripts/ci/plan-skip.sh`](../scripts/ci/plan-skip.sh), #1466) checks out
the merge with both parents and skips the rest of that workflow when all of
these hold:

- the commit merges exactly two parents, and its tree equals the second's
  (the PR head);
- the first parent, `dev` before the merge, is an ancestor of the PR head
  (the compare API's `behind_by` is 0). Tree equality alone isn't enough: a
  commit on `dev` that a PR run merged in and that `dev` then reverted would
  be in the tested tree but not in the merge's;
- in the newest `pull_request` run of that same workflow file on the PR head
  from this repository, the aggregate (`CI OK` in `ci.yml`, `Native OK` in
  `native-tests.yml`) concluded `success`, its latest attempt counting;
- nothing that feeds a rust-cache key changed since the first parent: any
  `Cargo.lock` or `Cargo.toml`, `rust-toolchain` or `rust-toolchain.toml`,
  `.cargo/config` or `.cargo/config.toml`, anything under `.github/` (the
  feature lists and other key variables live in the workflows and the pins)
  and `scripts/ci/local-macos.sh` (the native feature list). This condition
  is about caches only; the first two make a skip correct. When a key moves,
  the merge run saves the new caches PRs restore.

The aggregate still runs and passes when Plan skips, so the merge commit
shows `CI OK` and `Native OK` green, and Prune caches still runs after it. A
job skipped for any other reason still fails the aggregate. Any other event
(PRs, `main`, tags, the schedule, manual runs) skips Plan itself and runs
everything, and so does a merge that was behind `dev`, an octopus merge, a
red, missing or still running aggregate, or any error in Plan, such as a
failed API call. Plan's job summary and log give the reason. Its token has
`actions: read`, `checks: read` and `contents: read` only.

Limits:

- **Evicted caches.** A skipped merge saves no caches. If GitHub evicted
  `dev`'s entries (the 10 GB cap, or seven days unused), PRs build cold until
  a merge that runs, or the weekly `ci.yml` run, saves them again;
  `native-tests.yml` has no scheduled run. The same holds when a runner image
  update changes the toolchains in the native jobs' key, which no diff sees.
- **A PR into `main` from the same commit.** GitHub empties a run's
  `pull_requests` once its PR merges, so Plan can't tell runs by their PR's
  base. If the PR head was also the head of a PR into `main` and that run is
  the newer one, its result counts, though it tested a merge with `main`.
  Release flow PRs into `main` have `dev`'s tip as their head, which is never
  a later `dev` merge's second parent.

### Samples

The sample programs in `examples/rust/samples` are workspace members with
`publish = false` (#1450). The root `Cargo.toml` lists each one in `members`,
so a new sample needs adding there: with a glob, a leftover directory without
a `Cargo.toml`, such as a renamed sample's ignored `target/`, would break every
cargo command. The samples share the workspace's `Cargo.lock`, so a version
bump or a new dependency of a `bacnet-*` crate needs no lock refresh, and every
`--workspace` run covers them: clippy, rustdoc and the tests, in CI and in the
[native tests](#native-tests-macos-and-windows). They stay out of
`default-members`, so a plain `cargo build` or `cargo test` at the root skips
them; `-p` picks one, as in `cargo run -p whois-scan -- --help`. A sample
inherits the workspace's license, which Cargo Deny checks for every member,
but not `[workspace.lints]`, since samples print to the terminal.

### `no_std` checks

`bacnet-types`, `bacnet-encoding` and `bacnet-services` build without `std`
when `alloc` replaces their default `std` feature, and without an allocator
when neither is on (#1614), and the MS/TP and B/IP
crates (`bacnet-mstp-codec`, `bacnet-mstp-core`, `bacnet-bip-codec` and
`bacnet-bip-core`), the router core (`bacnet-router-core`), the cores'
time type (`bacnet-time`) and the network-layer vocabulary (`bacnet-npdu`)
never use it. A host build always has `std` and an allocator, so no single run
proves all of that; each check covers a part.

| Check | Job | What it shows |
| --- | --- | --- |
| `scripts/ci/check-no-std.sh` | Clippy, `NO_STD_TARGETS` | Each crate and feature set compiles clean under clippy on `thumbv6m-none-eabi`, `thumbv7em-none-eabihf` and `riscv32imac-unknown-none-elf`, with nothing skipped, and no package in its graph turns `std` on or is `tracing`, `tokio` or `socket2`. Nor does the crate choose atomics for the firmware: its own graph holds no `portable-atomic` or `critical-section`, and no `bytes` with `extra-platforms` |
| `scripts/ci/check-default-features.sh` | Clippy | Clippy and rustdoc, private items included, for the three crates with `alloc` and without `std`, and without either, on Linux, Windows and macOS |
| `cargo clippy -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features [--features alloc] --all-targets` | Clippy | The three crates' tests with `alloc` and without `std`, and without either, which the Test job runs, are lint-clean too |
| The `alloc` tests and doctests | Test | The three crates' tests and doctests with `alloc` and without `std`: the code a bare-metal build compiles. The tests link `std`; `check-no-std.sh` shows the code needs none |
| The allocation-free tests | Test | The three crates' tests without `alloc` either, including `bacnet-services/tests/no_alloc_subset.rs`, which runs Who-Is, I-Am and ReadProperty through the APDU header under a counting allocator and finds no allocation, and their doctests, whose examples write to sinks (#1779) |
| `scripts/ci/check-msrv.sh` | MSRV | The three crates with `alloc` and without `std` build at the MSRV, in a run of their own, since the hosted crates turn `std` back on in the main run |
| [Firmware link test](#firmware-link-test) | Clippy, `FIRMWARE_TARGETS` | The MS/TP and B/IP stacks, and an MS/TP device with its application layer, link into bare-metal images with no allocator, and `bacnet-encoding` and `bacnet-services` into one with an allocator, each within its size budget |

**ARMv6-M and `bytes`.** `bytes` needs atomic compare-and-swap, which
`thumbv6m-none-eabi` lacks. With its `extra-platforms` feature it takes that
from `portable-atomic`, whose backend only the final firmware should choose,
so no library here picks one. `firmware/nostd-harness` (the `nostd-harness`
crate, `publish = false`, #1613) plays the firmware for the lint: on that
target alone it depends on `bytes` with `extra-platforms` and on
`portable-atomic` with its `critical-section` backend, and cargo unifies
those features into the linted crate's `bytes`. `check-no-std.sh` brings it
into each `thumbv6m` build whose graph has `bytes`: `bacnet-encoding`,
`bacnet-services`, and the MS/TP and B/IP codecs with their `bytes` feature.
Cargo unifies features across all the targets of one run, so those builds run
on their own, and `bytes` on the other targets builds as their firmware would
build it. The `std` check covers the harness's packages too. The harness
would hide a library that chose a backend itself, so the crate's own graph
is checked for that first, on every target. The harness keys on
`thumbv6m-none-eabi` and `bytes`; a library that needed `portable-atomic`
directly, or another target without compare-and-swap, would need the script's
test and the harness's target table both generalised.

- **No critical-section implementation.** Clippy doesn't link, so the
  harness provides none. A firmware that links these crates on ARMv6-M picks
  its own backend and, for `critical-section`, links an implementation such
  as `cortex-m`'s `critical-section-single-core`. The
  [firmware link test](#firmware-link-test)'s apdu image links one of its
  own, and its link fails without it.
- **Workspace.** Like the [firmware link test](#firmware-link-test), the
  harness is a workspace member outside `default-members`. On a host it is an
  empty `no_std` crate with no dependencies, so host builds, clippy, rustdoc
  and tests are unchanged. Its two dependencies were already in `Cargo.lock`,
  and Cargo Deny covers them.
- **Cost.** On an Apple M5, the `alloc` tests and doctests take about 6 s
  from a cold target directory and under 2 s warm, and the harness builds add
  about 3 s to a cold `check-no-std.sh` run.

```bash
rustup target add thumbv6m-none-eabi thumbv7em-none-eabihf riscv32imac-unknown-none-elf
bash scripts/ci/check-no-std.sh                       # every default target
bash scripts/ci/check-no-std.sh thumbv6m-none-eabi    # one
cargo nextest run -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --features alloc --locked
cargo test --doc -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --features alloc --locked
cargo nextest run -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --locked
cargo test --doc -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --locked   # examples on sinks, no allocator (#1779)
```

### Firmware link test

`check-no-std.sh` runs clippy on the `no_std` crates, and clippy never links.
The firmware link test (#1626, #1635, #1664, #1667, #1679, #1771) does.
`firmware/link` (the `firmware-link` crate, `publish = false`) builds eight
`#![no_main]` bare-metal images, one per stack, because code can't be
credited to a crate reliably within one LTO image:

- `mstp` links `bacnet-mstp-codec` and `bacnet-mstp-core` (default
  features, with the core's `FixedQueues`) and through the core
  `bacnet-time`, with no global allocator;
- `mstp-extended` links the same core with extended frames on, and the
  buffers they need: 1516 octets of receive storage, `ExtendedActionBuffers`
  and `FixedQueues<1, 1, 1497>`, with no global allocator;
- `bip` links `bacnet-bip-codec` (default features, plus its `heapless`
  sink), both its B/IP and its B/IPv6 messages, with no global allocator.
  It links the codec alone, so its budget measures the codec;
- `node` links `bacnet-bip-core`'s port driver over its node engine, a
  `NodePort` on a foreign-device `Node` with two exchange slots, and
  through it the codec and `bacnet-time`, with no global allocator;
- `bbmd` links `bacnet-bip-core`'s port driver on a BBMD, a `BbmdPort`
  over the receive rules of a `Link` in BBMD mode and the BBMD tables
  (`Tables<4, 4>`), which answer the management requests themselves, with
  no global allocator;
- `router` links `bacnet-router-core` with its `bip` and `mstp` glue: a
  `Router` over `Storage<2, 8, 8, 4>` between a B/IP port where the router
  is also the BBMD (bip-core's `BbmdPort`, `Tables<2, 2>`) and an MS/TP
  port with extended frames, which Clause 9.3 asks of a router, and through
  them the receive rules, the MS/TP core and both codecs, with no global
  allocator. It
  routes and nothing more: a product router also needs a Device object,
  since Clause 6.6 holds a router to the device requirements of Clause
  22.1.5, and the image has no application layer to host one;
- `device` links an MS/TP device with no global allocator: the MS/TP core
  over a `StandardStorage<1, 2, 1>`, `bacnet-npdu`'s view, and
  `bacnet-types`, `bacnet-encoding` and `bacnet-services` without default
  features, their allocation-free profile (#1614). It answers Who-Is and
  ReadProperty for a Device object and an Analog Input;
- `apdu` links `bacnet-encoding` and `bacnet-services` (and through them
  `bacnet-types`) without default features and with `alloc`, the way a
  firmware uses them, with a global allocator over a fixed arena.

A use of `alloc` anywhere in an image but `apdu`, the device's application layer included, fails its build with "no
global memory allocator found". In any image, a symbol that only a hosted
target or the firmware provides fails the link: an atomic library call, or
a critical section that nothing implements. A compare-and-swap on ARMv6-M or
RV32IMC fails earlier, at compile time: `core` has no such method on those
targets, so a crate that needs one builds only with a backend such as
`portable-atomic`'s.

- **The scenarios.** Each image's scenario is one `no_std` function in the
  crate's lib that returns a summary of what it saw:
  - `mstp::run` encodes a few frame types and decodes them with a
    `StreamDecoder`, noise and a bad CRC included. It drives a `MasterCore`
    with injected time through a token pass, a queued data frame, a poll for
    master, a received broadcast and Test_Response and an answered request,
    carrying out each action, and records each frame sent with its
    destination. Then it fills
    the unicast queue and offers an NPDU too long for a standard frame.
  - `bip::run` encodes one message of each `Bvll` variant into a slice and
    into a `heapless::Vec` sink and decodes each back, and decodes three
    datagrams written by hand from the Annex J.2 layouts, so a fault that
    encode and decode share can't cancel out. It reads the decoded BDT and
    FDT tables from both ends, takes a Forwarded-NPDU's origin, asks a few
    functions for their payload shape and NAK, and takes the rejection
    paths: a Length field that disagrees with the datagram, a table that
    isn't whole entries, an undefined result code on encode, an unknown
    function, and a message too long for a fixed slice and for a nearly
    full sink. Then `bip::ipv6::run` does the same for each `Bvll6`
    variant in the same storage: three datagrams written by hand from the
    Annex U.2 layouts, a Forwarded-NPDU's origin, and B/IPv6 rejections,
    among them a B/IP type octet, the deleted function 0x0B and an
    address of the wrong length.
  - `mstp_extended::run` queues a 512-octet request, which goes out in a
    Type 32 frame once the peer passes the token; decoded from the octets
    the core sent, it is the request. The peer's 600-octet reply comes back
    in a Type 33 frame, octet by octet, and the core delivers it. Then the
    core polls for a master and passes the peer the token. The scenario's
    own buffers live on the stack of functions kept out of line, so the
    image's `.bss` is the core's storage alone.
  - `node::run` drives a foreign device configured as the hosted
    transport configures one (#1622): retries that back off, broadcasts
    held until the first registration attempt resolves (two here),
    Forwarded-NPDUs taken only from its BBMD by unicast, and a send budget
    that carries 1497-octet NPDUs. A peer sends it the six requests only a
    BBMD serves, and it answers each with that request's NAK. Two
    broadcasts are held while its first registration is out, a third is
    refused, and the BBMD's success releases the two. It delivers its
    BBMD's Forwarded-NPDU from the origin and drops a stranger's. The
    registration renews; a Distribute-Broadcast-To-Network NAK from the
    BBMD brings the next attempt forward; then the BBMD stops answering,
    the attempts back off until the lease lapses, and the BBMD's next
    success registers it again. Every datagram it sends is hashed with its
    destination, against a hash worked out by hand from the Annex J.2
    layouts.
  - `bbmd::run` serves as a BBMD with a BDT of two peers, one reached by
    one-hop distribution and one by two-hop, to which the table appends its
    own row. A foreign device registers, and a Read-FDT lists it with its
    seconds left. An Original-Broadcast on the BBMD's subnet goes to both
    peers and the foreign device; the two-hop peer's Forwarded-NPDU is
    broadcast on the subnet and passed to the foreign device, and a
    stranger's is refused; the foreign device's
    Distribute-Broadcast-To-Network goes to the subnet and both peers, and
    an unregistered sender's draws the NAK; a Read-BDT returns the three
    rows. Its datagrams are hashed as the node's are.
  - `router::run` routes between network 1, B/IP, where the router is the
    BBMD with one two-hop peer, and network 2, MS/TP, where it is station
    1 and station 2 passes it the token. On one clock for all three cores,
    with every send carried out and MS/TP fed octet by octet, it runs eight
    steps. At start, each port gets a Network-Number-Is and an I-Am-Router,
    which the BBMD also forwards to its peer. A B/IP node's message for
    station 5 leaves on the next token with SNET 1 and the node's address
    as SADR; the peer's Forwarded-NPDU at life-safety priority leaves with
    its origin as SADR and its priority kept (#1678). Station 2's request
    for the node goes out as an Original-Unicast, and the MS/TP side
    answers Reply Postponed one T_turnaround later. A message for unknown
    network 9 draws a Who-Is-Router on MS/TP and reject reason 1 on B/IP,
    and a second within the cooldown the reject alone. A 1494-octet NPDU,
    1498 once rewritten for MS/TP, draws reason 4 (#1665), and a 1493-octet
    one leaves at exactly 1497 in a Type 33 frame. A Who-Is-Router naming
    no network is answered with network 1, and a routing-table query with
    one Ack. A global broadcast from MS/TP goes out on B/IP, and the BBMD
    forwards it. The summary keeps what the router made of each frame,
    every send it asked for, its events and counters, and every datagram
    and frame, with a hash over each port's octets, worked out apart from
    the codecs from the Annex J.2, Clause 6.2 and Clause 9.3 layouts with
    the MS/TP CRCs and COBS (Clause 9.6, 9.10, Annexes G and T); that code
    reproduces Annex T.3's whole example frame.
  - `apdu::run` follows a client that finds a device behind a router and
    reads its analog input: a Who-Is, the device's I-Am as the router
    passes it on, a ReadProperty request for Present_Value and the
    ComplexACK that answers it. It encodes each message's service
    parameters, APDU and NPDU into `BytesMut` buffers, compares the NPDU
    with a datagram written by hand from the Clause 6 and 20.1 layouts, and
    decodes it back through all three layers. It also decodes each
    hand-written datagram from a heap copy and keeps what it carries, and
    decodes the ACK's value. Then it takes rejection paths: an NPDU of
    protocol version 2, and on encode an NPDU to DNET 0 and an APDU size
    the header can't express, each with the error kind it should draw; and
    five malformed APDUs and service parameters, for each of which it
    records how the library classifies the fault, the Reject reason the
    error maps to. Two of them, a Who-Is and an I-Am, are unconfirmed, so
    no Reject would be sent for them.
  - `device::run` is station 1 on an MS/TP line where a client, station
    2, asks for every device and for devices 5000 to 6000, passes the
    token, and sends thirteen confirmed requests. The device answers the
    first Who-Is with a global I-Am on its next token and ignores the
    second. It answers each request in its reply window: reads that
    succeed, one through the wildcard Device instance and one relayed
    from network 5, which goes back with that DNET and DADR; the four
    ReadProperty Errors of Clause 15.5.1.3.1; a Reject for a request with
    no property identifier and for WriteProperty; and an Abort for a
    segmented request. Two confirmed requests by broadcast, one in a
    broadcast frame and one to every network, go unanswered. The client's frames are written by hand from the
    Clause 6.2 and 20.1 layouts, and so are the answers it expects, whose
    hashes `device::expected()` holds.

  The host tests in `firmware/link/tests/` (`mstp.rs`, `mstp_extended.rs`,
  `bip.rs`, `node.rs`, `bbmd.rs`, `router.rs`, `device.rs` and `apdu.rs`)
  run them and
  compare each
  whole summary, so the behaviour is checked natively, in every
  `--workspace` test run on Linux, macOS and Windows. Each image's reset
  entry calls its scenario, passing the inputs and the summary through
  `core::hint::black_box`. CI never
  runs the images: building them proves that they link, and measures them.
  The [emulation check](#emulation-manual), run by hand, runs them on
  emulated boards.
- **The apdu image's allocator and atomics.** A library never chooses
  these; the image does, as a firmware would, and only on bare-metal
  targets (a host build uses std's allocator):
  - **Allocator:** `embedded-alloc`'s linked-list first fit (`LlffHeap`,
    without its TLSF allocator) over a static arena of
    `firmware_link::apdu::ARENA` octets, 1 KiB. `tests/apdu.rs` runs the
    scenario a second time in that allocator (`linked_list_allocator`, which
    `LlffHeap` wraps) over an arena of the same size, and checks that it
    fits, succeeds, frees everything and peaks at no more than half the
    arena; its peak, which nextest prints, is 312 octets on a 64-bit host,
    whose blocks are no smaller than a 32-bit target's.
  - **Critical section:** the allocator locks its heap with one, on every
    target, and both kinds suit a part with one core: they mask interrupts
    and put back the mask they found. On RISC-V the image takes `riscv`'s
    `critical-section-single-hart`. On Cortex-M it implements its own with
    PRIMASK, as `cortex-m`'s `critical-section-single-core` does, because
    `cortex-m` depends on the unmaintained `bare-metal`
    (RUSTSEC-2026-0110), which Cargo Deny refuses. PRIMASK leaves NMI and
    HardFault unmasked, so their handlers must stay out of critical
    sections; the image's default handlers only loop.
  - **Atomics:** `bytes` needs compare-and-swap, which ARMv6-M and RV32IMC
    lack. On targets without it (`cfg(not(target_has_atomic = "ptr"))`), the
    image turns on `bytes`' `extra-platforms` feature and
    `portable-atomic`'s `critical-section` backend, as
    `firmware/nostd-harness` does for the lint, and the same critical
    section serves both. Nothing links an atomic library call: on ARMv6-M,
    `bytes`' reference counts compile to that critical section around a
    plain load and store.

  Without the global allocator the image fails to build with "no global
  memory allocator found"; without the critical section it fails to link,
  missing `_critical_section_1_0_acquire` and `_release`. Its `unsafe`
  code beyond the runtimes' is the call that hands the allocator its arena
  and the Cortex-M critical section. The allocator and the critical section
  add three packages to `Cargo.lock`: `embedded-alloc`,
  `linked_list_allocator` and, for the host test's lock, `spinning_top`.
- **Targets.** `thumbv7m-none-eabi` (Cortex-M3), `thumbv6m-none-eabi`
  (Cortex-M0/M0+), `riscv32imc-unknown-none-elf` (RV32IMC, the ESP32-C3
  class) and `thumbv7em-none-eabihf` (Cortex-M4F and M7F, with the
  hard-float ABI, since #1660), in `FIRMWARE_TARGETS` in `ci.yml`, each
  with budget tables of its own. `thumbv7em-none-eabihf`'s images are a
  little smaller than `thumbv7m-none-eabi`'s. They share one generic memory
  layout, `firmware/link/memory.x`: 256 KiB of flash at `0x08000000` and 64
  KiB of RAM at `0x20000000`. The startup code comes from `cortex-m-rt` and
  `riscv-rt`, bare-metal dependencies only; `firmware/link/build.rs` hands
  the linker `memory.x` and `link.x`. `memory.x` also discards `.eh_frame`,
  the unwind tables riscv-rt's script would leave in flash (2.6 KB in the
  MS/TP image). Only unwinding reads them, and the images never unwind:
  they build with `panic = "abort"`, which is both the `firmware` profile's
  setting and these targets' default. Were that ever to change, the worst
  case is a panic that aborts, which is what happens today. cortex-m-rt
  already drops ARM's unwind tables. The images' only `unsafe` code is in
  the runtimes' entry macros; on RISC-V, one block in each image but `apdu`
  that borrows its statics, which cortex-m-rt's macro does safely; and the
  apdu image's hand-over of its arena and its Cortex-M critical section.
- **Workspace.** The crate is a workspace member outside `default-members`,
  like the [samples](#samples). It shares `Cargo.lock`, so Cargo Deny and
  Cargo Audit cover the runtime crates and the apdu image's allocator; it
  follows `[workspace.lints]`; and it
  builds with the root `Cargo.toml`'s `firmware` profile (release with
  `opt-level = "z"`, LTO, one codegen unit and `panic = "abort"`), since
  cargo reads profiles only from the workspace root. On a host the binaries
  are empty stubs and neither the runtimes nor the apdu image's allocator
  and critical section are built, so `--workspace` builds, clippy, rustdoc
  and tests work as before.
- **The `alloc` feature.** The crate's `alloc` feature brings in the
  `apdu` scenario, the apdu image and their dependencies, and is on by
  default, so host builds, clippy, rustdoc and tests cover them. It links
  `alloc` into the crate's lib, so the other images build with
  `--no-default-features`: a bare-metal build of them with the feature on
  fails, as it should, with "no global memory allocator found". The device
  image depends on `bacnet-types`, `bacnet-encoding` and
  `bacnet-services` without default features; the feature turns on their
  `alloc` profile for `apdu`, so the device image fails the same way if
  anything turns it on for them.
- **The check.** `scripts/ci/check-firmware-link.sh` runs, per target,
  clippy on the images' code (a host build compiles it out) and the
  `firmware`-profile build, once without the `alloc` feature for every
  image but `apdu` and once with it for `apdu`, then
  `scripts/ci/firmware_size.py` for each image against its
  `[<target>.<image>]` table in `firmware/link/size-budget.toml`.
  The helper reads the ELF itself, so it needs no binutils. It prints:
  - every loaded section and which total it counts toward;
  - `.text`, `.rodata`, `.data`, `.bss` and two totals against their
    budgets. **flash** is every loaded section with contents: the vector
    table on Cortex-M, `.init.rust` on RISC-V, and `.data`'s initial image
    included. **RAM** is every loaded writable section, `.data` and
    riscv-rt's `.heap` (`_heap_size` octets, 0 here) included, but not its
    `.stack`, which fills whatever RAM is left. Neither counts the stack
    used at run time. Both go by section flags, not by memory region, so
    code placed in RAM would count toward flash only, and the alignment
    gaps between sections (at most 4 octets of flash per image today)
    count toward neither;
  - the counts of `core::fmt` functions, of 64-bit division routines and
    of memmove routines linked in, the largest objects in RAM, and `.text`
    by crate.

  It fails when a size is over budget, or when any formatting code is
  linked in: the panic handler never reads the message, so panics cost no
  formatting code, and a change that adds some shows up here. It fails the
  same way on any 64-bit division routine (`div64_functions`, budgeted at
  0): compiler-builtins' `u64_div_rem` and the `__aeabi_uldivmod`,
  `__udivdi3`, `__umoddi3` and similar entry points to it. These targets
  have no 64-bit divide instruction, so a 64-bit division by a runtime
  value costs about 0.9 KB, which the budgets' headroom would otherwise
  absorb; the MS/TP core divides only in 32 bits (#1618). Every image,
  apdu included, holds both counts at 0: the libraries' decoding and
  encoding errors carry static labels formatted only when displayed
  (#1646, #1777), which no image does. The apdu image used to build its
  messages with `format!`, which linked 15 `core::fmt` functions (about
  2.7 KB of `.text` on `thumbv7m-none-eabi`), and on `thumbv6m-none-eabi`,
  which has no multiply with a 64-bit product, formatting a `u64` divided
  by a constant through the 64-bit division routine (3 functions). It also fails
  when the image has no function symbols to look for that code in: the
  images must not be stripped. A size more than 20% under its budget gets a
  notice to tighten it, not a failure, and so does a count under its limit,
  so a ceiling such as the apdu image's old 15 `core::fmt` functions comes
  down once the code behind it goes. It fails the same way when an image
  links more memmove routines than its `memmove_functions` allows:
  compiler-builtins' `memmove` and the `memmove`, `__aeabi_memmove`,
  `__aeabi_memmove4` and `__aeabi_memmove8` entry points to it, 0.8 to
  1.4 KB in all. A copy of a whole value that the compiler can't prove
  doesn't overlap its source, or a slice rotation, calls it. The bip,
  node, bbmd, mstp, mstp-extended, router and device images link none, and their
  limit of 0 keeps it out. The apdu image links it (2); its limit is that count,
  with no headroom.
  `scripts/ci/test-firmware-size.py` tests the helper on small ELF files
  it builds itself, in the Lint job. CI runs the check in the Clippy job,
  after the `no_std` check, which already sets up bare-metal targets,
  rather than in a job of its own that would install the toolchain again
  for builds that take seconds.
- **What `.text` holds.** Each image's `.text` includes its scenario, not
  just the stack it links. The check's by-crate list shows the split: on
  `thumbv7m-none-eabi`, the MS/TP image's 16.4 KB is about 6.4 KB of
  `bacnet-mstp-core`, 5.5 KB of the scenario (with the core code it
  inlines), 1.5 KB of compiler-builtins and startup code (memory copies
  and fills, and the reset handler; the list shows the unmangled part
  separately) and 2.9 KB of `bacnet-mstp-codec`, about 1.3 KB of it the
  COBS and CRC-32K of extended frames, which its stream decoder and
  encoder can reach though the scenario sends none. Only `thumbv6m-none-eabi`,
  which has no divide instruction, also links compiler-builtins' 32-bit
  multiply and divide routines. The apdu image's 37.4 KB on
  `thumbv7m-none-eabi` is about 13 KB of `bacnet-encoding`, 8.6 KB of the
  scenario with the library code it inlines, 3.7 KB of `core` (formatting
  among it), 2.8 KB of `bacnet-services`, 2.4 KB of `bytes` and 2.1 KB of
  compiler-builtins; about 4 KB of its 5.2 KB of `.rodata` is text, nearly
  all of it error messages and their labels. Of the node image's 12.2 KB,
  about 3.0 KB is credited to `bacnet-bip-core`, 0.6 KB to
  `bacnet-bip-codec`, 0.7 KB to compiler-builtins (memory copies and
  fills) and 6.8 KB to the scenario, which inlines much of the engine; of
  the bbmd image's 8.5 KB,
  4.8 KB to the scenario with the receive rules and table code it inlines,
  1.1 KB to the codec, 0.9 KB to `bacnet-bip-core` and 0.7 KB to
  compiler-builtins. The mstp-extended image's 15.5 KB is about 5.5 KB of
  `bacnet-mstp-core`, 5.6 KB of the scenario with the core code it
  inlines and 2.9 KB of the codec: smaller than the mstp image, whose
  scenario also drives the codec's stream decoder on its own. The router
  image's 42.8 KB is about 10.6 KB of `bacnet-router-core`, 6.0 KB of
  `bacnet-mstp-core`, 2.7 KB of `bacnet-bip-core`, 2.6 KB of
  `bacnet-mstp-codec`, and 17.1 KB of the scenario and its reset entry,
  which inline much of the engine, both glue modules and the BBMD tables.
  `bacnet-bip-core` avoids the two patterns that link `memmove` (#1667):
  it copies a 6-octet B/IP address's IP and port apart into a BDT's own
  row and the FDT rows of a Read-FDT-Ack, which copied whole linked it on
  `thumbv6m-none-eabi`, and the node shifts its queued events by swaps,
  where `rotate_left` linked it on every target. The MS/TP stack avoids
  them too (#1691): the codec's stream decoder drops octets by moving a
  start offset and moves what it holds to the front only in pieces that
  don't overlap, and the core's queues keep each item in its slot until
  it leaves, so no removal shifts the others. `bacnet-router-core` avoids
  them as well (#1679): its sorted table and searches, its hold-down
  records, the networks one message claims and its output queue move
  elements by swapping two distinct ones, never with `copy_within` or a
  copy between elements, which linked it on every target (1.1-1.4 KB of
  text on Cortex-M, 0.8 KB on RISC-V); `slice::swap` links it too for an
  element as large as a route, since it has to allow equal indices.
- **RAM.** Most scenarios keep their buffers in statics, so `.bss`
  measures them; the mstp-extended image's scenario is the exception,
  below. The MS/TP image keeps everything large the core borrows in its
  static, the firmware pattern the core's crate docs describe: receive
  storage, queues (3,360 octets for `FixedQueues<4, 2>`) and
  `ActionBuffers<1>` (1,028), 5,544 octets in all with the codec part's
  buffers. The core itself, 520
  octets on these targets, is built on the stack. `.bss` doesn't count the
  stack; the [emulation check](#emulation-manual) measures how deep it goes.
  The mstp-extended image's static holds only what the core borrows,
  7,712 octets: receive storage of 1,516 octets, `FixedQueues<1, 1,
  1497>` and `ExtendedActionBuffers<1>`. Each more queue slot would add
  about 1.5 KB. Its scenario keeps its own NPDUs and frames on the stack,
  and only while it builds or checks them. The node image's `.bss` is
  240 octets: its two 64-octet datagram buffers, one received and one
  sent, since the port sends while the received one is still borrowed,
  and the two `HeldBroadcast<54>` slots the port holds the node's
  broadcasts in until its first registration resolves, 56 octets each. A
  slot is as long as the link's broadcasts may be, so the image gives its
  link the least budget a link takes, 64 octets, for 54-octet broadcasts;
  at the hosted transport's budget, 1,507 octets, each slot would take
  1,500. The bbmd image's is its two datagram buffers and
  what the port lends the tables for a decision's destinations and a
  Read-FDT-Ack's rows, 240 octets. The node and its port, 752 octets and
  a little more on these targets, and the bbmd image's port and tables
  are built on the stack. The router image's static is 15,536 octets on
  these targets:
  - the router's `Storage<2, 8, 8, 4>`, 3,152: a 1,497-octet output
    scratch, a table of 8 routes (120 octets each) and 8 hold-down
    records, 4 searches, the output queue and the counters (16 drained
    `u32`s, which the image takes once, at the end);
  - the MS/TP core's receive storage (1,516), `FixedQueues<1, 2, 1497>`
    (4,688) and `ExtendedActionBuffers<1>` (3,028). Two broadcast slots,
    since the router queues its startup Network-Number-Is and I-Am-Router
    together; one unicast slot, since each step's sends leave on the next
    token;
  - the B/IP socket's two datagram buffers, 1,507 octets each: the one
    received, which the router reads in place, and the one sent, which
    the port's `Datagrams` encodes into;
  - where the BBMD writes one decision's destinations, 40, and a
    Read-FDT-Ack's rows, 20.

  The router (88 octets), the MS/TP core (528), the BBMD port and the
  BBMD's tables (72) are built on the stack; a BBMD port runs no node. A
  BBMD port holds no broadcasts, so the image has no `HeldBroadcast`
  slots; a foreign-device port would need its `hold` of them, each as long
  as `Link::max_npdu(Cast::Broadcast)`, 1,497 octets at the default
  budget.
  `cargo nextest run -p firmware-link` prints the host sizes from the
  `ram_sizes` tests. The apdu image's
  `.bss` is its 1 KiB arena and the allocator's 32-octet state: what its
  scenario builds lives in the arena or on the stack, and
  `heap_fits_the_arena` measures the arena's use.
- **Raising a budget.** Each budget is the measured size plus 12%, rounded
  up to 16 octets. When a change grows an image on purpose, the error
  suggests a new budget; update it and the measured sizes in the file's
  comments, and say in the PR why the image grew. That makes the growth a
  reviewed change. `check-firmware-link.sh --suggest` prints a full budget
  table for each image as it builds now.

```bash
rustup target add thumbv7m-none-eabi thumbv6m-none-eabi riscv32imc-unknown-none-elf thumbv7em-none-eabihf
bash scripts/ci/check-firmware-link.sh                       # every target
bash scripts/ci/check-firmware-link.sh thumbv6m-none-eabi    # one
bash scripts/ci/check-firmware-link.sh --suggest             # budgets as the images build now
cargo nextest run -p firmware-link                           # the scenarios, natively
python3 scripts/ci/test-firmware-size.py                     # the size helper's tests
```

### Emulation (manual)

The [firmware link test](#firmware-link-test) builds its images and CI never
runs them. `scripts/emu/emulate-firmware.sh` (#1651) runs them, on boards
QEMU emulates, so each stack's code executes on the instruction set it was
built for. It is a manual check, run at intervals such as a toolchain bump
or a change to a `no_std` crate's hot paths: no push, pull request or
schedule runs it, and no required check depends on it.

- **What runs.** The `emulate` feature of `firmware-link`, with one board
  feature, builds an emulation build of each image (mstp, mstp-extended,
  bip, node, bbmd, router, device and apdu). Its reset entry runs the same scenario
  as the link test's image, with its input still behind `black_box`, then
  checks the summary against the one the host test asserts:
  `firmware_link::mstp::EXPECTED`, `mstp_extended::EXPECTED`,
  `bip::EXPECTED`, `node::expected()`, `bbmd::EXPECTED`,
  `router::expected()`, `device::expected()` and `apdu::expected()`,
  shared by both so they
  can't drift. The router image is built for every board but runs only on
  the two with more than 16 KiB of RAM (see Stack, below). It prints the
  summary and a verdict through semihosting and exits with a status QEMU
  passes on (below). The build uses the `emulate` profile, the `firmware`
  profile under its own name, so the link test's images in
  `target/<triple>/firmware` stay as they were; they are built without the
  feature, and are byte for byte what they were before it existed.
- **Semihosting** comes from the `semihosting` crate (taiki-e), which
  covers Arm M-profile and RISC-V without `cortex-m`, whose `bare-metal`
  dependency Cargo Deny refuses (RUSTSEC-2026-0110). It is a bare-metal,
  optional dependency with no dependencies of its own, so host builds never
  compile it; `Cargo.lock` lists it, so Cargo Deny covers it.
- **Boards.** Each has its own memory layout in `firmware/link/boards/`,
  which `build.rs` takes instead of the generic `memory.x` when the board's
  feature is on; the sizes come from QEMU's own memory map (`info mtree`).
  The script names a board by its QEMU machine.

  | Board (QEMU machine) | Feature | CPU | Target | Flash, RAM | Why |
  | --- | --- | --- | --- | --- | --- |
  | `microbit` | `board-microbit` | nRF51822, Cortex-M0 | `thumbv6m-none-eabi` | 256 KiB at 0, 16 KiB at `0x20000000` | QEMU's ARMv6-M board, with the smallest RAM, the tightest fit for the stacks |
  | `lm3s6965evb` | `board-lm3s6965evb` | LM3S6965, Cortex-M3 | `thumbv7m-none-eabi` | 256 KiB at 0, 64 KiB at `0x20000000` | The usual Cortex-M3 board for QEMU, with the link test's layout's sizes (QEMU's other is `mps2-an385`) |
  | `netduinoplus2` | `board-netduinoplus2` | STM32F405, Cortex-M4F | `thumbv7em-none-eabihf` | 1 MiB at `0x08000000`, 128 KiB at `0x20000000` | The link test's addresses, and an FPU, which cortex-m-rt turns on for an `eabihf` target |
  | `sifive_e` | `board-sifive-e` | SiFive E31, RV32IMAC | `riscv32imc-unknown-none-elf` | 512 KiB of the XIP flash from `0x20400000`, 16 KiB at `0x80000000` | Like an MCU: code runs in place from flash and data lives in a small RAM, where `virt` loads everything into its RAM. RV32IMC code runs unchanged on an RV32IMAC core |

  Each board's target is one of the link test's too, so its images have
  size budgets as well; `thumbv7em-none-eabihf` joined them for
  `netduinoplus2` (#1660).
- **Exit statuses.** QEMU exits with the image's status. A run fails on
  any status but 0:

  | Status | Meaning | What the image prints |
  | --- | --- | --- |
  | 0 | The summary is as expected, with stack paint to spare | `summary: …`, then `PASS <image> on <board> (<target>): …; stack peak N of M octets` |
  | 1 | The summary differs | `expected:` and `actual:`, each pretty-printed so they diff field by field, then `FAIL …` |
  | 2 | The summary is as expected, but no stack paint is left: the stack may have run into the statics | `FAIL …` |
  | 3 | The CPU faulted | `FAIL …: HardFault at pc …` on Cortex-M, `exception <mcause> at pc …` on RISC-V |
  | 101 | The image panicked | `FAIL …: panicked at <file>:<line>:<column>:` and the message |
  | 124 | Still running at the timeout (30 s by default), so the runner killed it | Whatever it printed before. A hang, or a fault before the handlers could report |

  For a fault, look up the program counter in the emulation build, such as
  `target/thumbv6m-none-eabi/emulate/mstp`, with `llvm-objdump -d` or
  `addr2line`. On `lm3s6965evb`, QEMU warns that the board's Ethernet
  controller "has no peer"; nothing uses it, and the warning is harmless.
- **Stack.** At reset the runtime paints the stack, from the end of the
  statics to the top of RAM, with `0xCCCC_CCCC`: cortex-m-rt's
  `paint-stack` feature on Cortex-M, and on RISC-V a `__pre_init` in
  assembly, which riscv-rt's `pre-init` feature calls before RAM is set up.
  After the scenario the harness scans up from the bottom for the first
  word that lost the paint, before it builds the expected summary or
  prints anything, and reports that depth as the peak. It counts the
  words the scenario's frames wrote, so a frame's untouched padding goes
  uncounted, and it covers the emulation build, which runs the same
  scenario code with the same profile as the link test's image. The peaks,
  measured with Rust 1.99.0 and QEMU 10.0.13 when the mstp-extended, node
  and bbmd images joined (#1664, #1667), the two MS/TP images' again
  when the core stopped copying a queued NPDU onto its stack (#1691), and
  the node, bbmd and router images' again after their ports changed
  (#1774), in octets, and on the two 16 KiB boards the stack's room, all
  of RAM above the statics:

  | Target (board) | mstp | mstp-extended | bip | node | bbmd | router | apdu |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | `thumbv6m-none-eabi` (`microbit`) | 2,040 (10,840) | 3,020 (8,672) | 2,220 (16,252) | 5,444 (16,144) | 2,380 (16,144) | not run (880) | 2,768 (15,328) |
  | `thumbv7m-none-eabi` (`lm3s6965evb`) | 1,824 | 2,868 | 2,100 | 4,972 | 2,260 | 4,456 | 2,536 |
  | `thumbv7em-none-eabihf` (`netduinoplus2`) | 1,832 | 2,868 | 2,100 | 4,972 | 2,260 | 4,456 | 2,536 |
  | `riscv32imc-unknown-none-elf` (`sifive_e`) | 1,712 (10,840) | 2,732 (8,672) | 2,020 (16,252) | 4,808 (16,144) | 2,132 (16,144) | not run (880) | 2,464 (15,328) |

  So on ARMv6-M the MS/TP image needs about 7.6 KB of RAM: 5,544 octets of
  statics and 2.0 KB of stack. With extended frames on it needs about
  10.7 KB, 7,712 octets of statics and 3.0 KB of stack, which leaves
  5.6 KB of a 16 KiB part free. The core encodes a queued NPDU's frame
  straight from the queue's slot, so its send no longer grows with
  extended frames; the mstp-extended image's deepest point is its
  scenario's, taking in the peer's 600-octet reply with about 1.2 KB of
  the scenario's own buffers for it live. Until #1691 the core copied the
  NPDU onto its stack, and the image peaked at 6,136 octets there.
  The node image's peak is mostly the scenario's own. When it joined
  (#1667), its ARMv6-M frame was 2.8 KB, holding the 800-octet summary,
  the 752-octet node and the result `Node::new` builds it in, and the
  reset entry kept the 0.8 KB summary it got back; the engine's calls went
  about 1.1 KB deeper. Driving the node through a `NodePort` (#1766)
  made it 0.7-0.8 KB deeper.
  The router image needs about 20 KB: 15,504 octets of statics and 4.5 KB
  of stack. When it joined (#1679), its BBMD port held a node, and the
  image peaked 1.2 KB deeper; the port no longer holds one (#1767). On the 16 KiB boards
  the statics leave 880 octets, so `emulate-firmware.sh` builds the image
  for them and doesn't run it; run anyway, it faults on both as its stack
  reaches the statics. The extended frames Clause 9.3 asks of a router's
  MS/TP port are most of those statics, so no standard-frame router image
  is offered for those boards.
  The device image, measured when it joined (#1771) with the same
  toolchain and QEMU, peaked at 2,652 octets on `microbit`, 2,284 on
  `lm3s6965evb`, 2,292 on `netduinoplus2` and 2,172 on `sifive_e`; its
  3,496 octets of statics leave the 16 KiB boards 12,888 for the stack.
- **Stack budgets.** `firmware/link/stack-budget.toml` (#1660) holds a
  budget for each board and image that runs there (the micro:bit and
  SiFive E tables have no router key), in a `[<board>]` table with a key
  per image: the peak measured when it was set plus 25%, rounded up to 16
  octets as the size budgets are. That is more headroom than their 12%,
  since one more buffer on the stack, or a change in what the compiler
  inlines, can move a peak by a whole frame. After the runs, the script
  hands what they printed to `scripts/ci/firmware_size.py --stack`, which
  reads the peak from each PASS line and prints a table of peaks against
  budgets. The run fails, and the script exits 1 as it does for a failed
  image, when a peak is over its budget, when an image the board's table
  lists printed no PASS line, or when one that passed has no budget. A run
  of every board also fails on a table for a board it doesn't have, left
  from one since removed; a run of some boards can't tell. When a run
  itself failed, the script keeps that run's status. A peak that needs a
  budget more than 20% under the one it has gets a notice to tighten it.
  As with a size budget, raising one is a reviewed change: say why the
  stack grew. `emulate-firmware.sh --suggest` prints the file's tables as
  the images run now, in place of the check; the file's comments record
  the peaks. CI never reads the file, since CI never runs the emulator.
- **The runner.** `emulate-firmware.sh` builds the images on the host with
  the pinned toolchain (clippy first, as the link test does), then hands
  them to `scripts/emu/run-qemu.sh`, which runs each on its board with a
  timeout, prints what it reports and a table of results, and exits
  non-zero if any failed; then the script checks the peaks the images
  printed against their stack budgets, above. By default the runs happen
  in a Docker image built
  from `scripts/emu/Dockerfile` on first use and tagged
  `rusty-bacnet-emu:<the first 12 hex digits of the Dockerfile's SHA-256>`,
  so editing the file builds a new image. It is reproducible: Debian 13.7
  slim pinned by its index digest (amd64 and arm64), apt pointed at the
  Debian snapshot archive at the date that image was built from, and
  `qemu-system-arm` and `qemu-system-riscv` pinned to version
  `1:10.0.13+ds-0+deb13u1` (QEMU 10.0.13). The image takes about 330 MB;
  remove it with `docker image rm rusty-bacnet-emu:<tag>`. The container
  gets no network and read-only mounts of the images and `scripts/emu`.
  `--native` runs the same `run-qemu.sh` on the host's QEMU instead
  (Homebrew's `qemu` on a Mac), whose version may differ.

```bash
rustup target add thumbv6m-none-eabi thumbv7m-none-eabi thumbv7em-none-eabihf riscv32imc-unknown-none-elf
bash scripts/emu/emulate-firmware.sh                     # every board, QEMU in Docker
bash scripts/emu/emulate-firmware.sh microbit sifive_e   # some boards
bash scripts/emu/emulate-firmware.sh --native            # the QEMU on PATH (brew install qemu)
bash scripts/emu/emulate-firmware.sh --timeout 60        # seconds per image (default 30)
bash scripts/emu/emulate-firmware.sh --suggest           # stack budgets as the images run now
```

- **The workflow.** `.github/workflows/emulation.yml` runs the script, with
  Docker, on GitHub's `ubuntu-24.04` runner. Its only trigger is
  `workflow_dispatch`: start it from the Actions tab ("Emulation", then
  "Run workflow" on a branch) or with `gh workflow run emulation.yml --ref
  <branch>`, and follow it with `gh run watch`. Its one job, "Emulation:
  firmware images under QEMU", reads the repository and nothing else. It
  isn't part of `CI OK` or `Native OK`, and `scripts/release/ci_gate.py`
  reads only `ci.yml` and `native-tests.yml` runs, so it can never stand in
  for either.
- **Cost.** On an Apple M5 with Docker Desktop, a run of every board takes
  about 30 s from a cold target directory, clippy and the builds included,
  and about 15 s warm, the twenty-six QEMU runs under a second each.
  Building the Docker image the first time adds about 30 s.

### NAT topology (manual)

`examples/docker/nat/run.sh` checks B/IP across real NAT routers (Annex
J.7.5, #940, #942): two BBMD sites, each behind an iptables router that
port-forwards UDP 47809, after the two-site example of the annex; a foreign
device on the "internet" network; and one behind a router that forwards
nothing. Each foreign device must hear both sites' devices answer its
Who-Is. The tests' loopback NAT shim
(`crates/bacnet-transport/tests/bip_nat`) covers the same rules on every
CI run; this is the check against a kernel's NAT.

- **Locally:** `bash examples/docker/nat/run.sh` (Docker with Compose v2;
  Linux, or Docker Desktop). It builds the image, probes, prints the BBMD
  logs on a failure and tears everything down.
- **On GitHub:** `.github/workflows/nat-docker.yml`, `workflow_dispatch`
  only, like the emulation check. Its job is named "NAT (manual): ...", so
  it never stands in for CI OK or Native OK.

### Runner

Every Linux job runs on GitHub's `ubuntu-24.04` hosted runner, an x86_64 VM
with 4 vCPUs, 16 GB of RAM and about 14 GB free on `/`. A job runs on the VM
itself, not in a container, as the unprivileged `runner` user with
passwordless `sudo`, and installs what it needs (see [Tool pins](#tool-pins)).
Hosted runners take any PR, including one from a fork, which a self-hosted
runner shouldn't.

**Disk.** The largest job, Test, needs about 6 GB: about 3 GB of `target/` for
the every-feature test build, which the doctests and the CLI's feature-off
tests mostly reuse, 1 to 2 GB of Cargo registry (the downloaded crates and
their unpacked sources) and under 1 GB of toolchain. Clippy's `target/` is about 1 GB for the host, plus the Windows and
macOS check builds. Those `target/` sizes are from the same builds on macOS
arm64 with the workflow's build settings (October 2026); Linux builds without
debug info should come out about the same. So the jobs neither delete the image's
preinstalled SDKs nor move `target/` to another disk. Each build job ends with
a "Disk use and limits" step that logs `df`, the sizes of `target/`,
`CARGO_HOME` and `RUSTUP_HOME`, and the descriptor limits. If a job ever runs
short, remove the preinstalled SDKs no job uses (Android, .NET, GHC) in a step
before the build.

**Privileges and limits.** The Forgejo jobs ran as root in a Docker container
whose soft limit on open files was 1024. On the hosted runner they run as
`runner` on the VM's own network stack:

- No test needs root or a capability. The tests that open AF_PACKET sockets
  (`bacnet-integration-tests`' `ethernet_network_numbers`, `CAP_NET_RAW`) or
  need an isolated IPv6 link (`bacnet-transport`'s `ipv6_selected_link*`) are
  `#[ignore]`d and run only on a host set up for them. Otherwise the
  `ethernet`, `serial` and `serial-gpio` features are compiled and
  unit-tested without hardware; the serial-port listing test reads
  `/sys/class/tty`, which the VM has.
- The MS/TP token ring is tested without a serial port on
  `bacnet-mstp-sim`, a simulated RS-485 bus in virtual time that runs real
  `bacnet-mstp-core` masters (#1605). Its scenarios, randomized fault runs
  included, take a few seconds of wall time and read no clock. To widen the
  randomized runs locally, run
  `MSTP_SIM_SEEDS=400 cargo nextest run -p bacnet-mstp-sim -E 'binary(property)'`.
  Host and mixed-node runs use buffered, timestamped reads, seeded chunk
  boundaries, serial-drain completion and optional content-matched echo.
  Paired paused-time fixtures also drive the public `MstpTransport` boundary.
  Ready application answers wait for drain/echo completion; first-reply
  timing follows the accepted request's physical end across unrelated traffic.
  Configurable post-drive holds keep physical ownership after the final data
  stop, with separate release records, overlap checks and receiver gating.
  The model uses a conservative maximum of 14 bit times after the stop ends
  to satisfy the different reference edges in 135-2020 9.2.3 and 9.5.3;
  core and host completion timestamps keep their final-data/drain semantics.
  Idle holds do not generate characters or extend the line-activity clock.
  Default-zero inter-octet gaps capture one schedule per transmission and
  permit at most 19 bit times after each non-final stop bit; the 20-bit idle
  limit includes that stop bit. Positive long-frame scenarios explicitly
  provision local watchdog and queue-age budgets. Octet-mode watchdog expiry
  truncates the frame and releases its driver; final-stop and later-octet
  deadline ties have separate regression tests. Host operations retain the
  production write/drain/echo wait without polling core deadlines. Their
  recorded start, watchdog, wire-end, completion and notification times
  distinguish budget overruns without claiming an observed host abort.
  Buffered hosts also model a captured local self-echo return delay before
  ordinary read latency. A bounded FIFO preserves physical character/error
  provenance across completion and new writes, and power loss cancels it.
  Compact counters expose overflow and cancellation; bounded traces show
  requested availability and FIFO deferral. This local policy changes no
  physical bus timing. Bus-generated read sequences are replayed through the
  public transport for Grace, Residual and Probe boundaries, expired and
  missing copies, and ordered peer payloads. A separate simulator negative
  test retains parser-error and missed-receipt diagnostics when delayed local
  echo interleaves with peer traffic.
  Opt-in receiver-specific digital contention can capture a contributor,
  alter UART-clean data or retain framing errors on the same physical grid.
  Tests preserve global collision/release evidence, check standard and extended
  winner payloads, independently reject an altered standard CRC, and cover
  identical simultaneous intents, lifecycle eligibility and delayed actual
  self-echo. Bounded receipt candidates are correlated with the terminal
  character through real forwarding and a separate reference codec instance;
  later valid traffic cannot hide a missing winner. Parser synchronization,
  unsupported extended input and explicit filter dispositions qualify those
  checks. Seed replay is independent of scenario/jitter/chunk draws. Mixed
  adapters at 9600, 38400 and 115200 baud receive exact known traffic after
  bounded contention; this does not promise universal duplicate-token recovery
  or model electrical arbitration, UART resynchronization or clock drift.
  Physical wire checks remain active: randomized fault runs retain and assert
  diagnosed failures when a host starts before injected activity becomes
  readable; clean and settled operation must pass the strict checks. These
  simulations do not qualify physical serial hardware or USB timing.
- The BACnet/SC hub capacity test needs about 2,100 descriptors and raises its
  own soft limit to that, which needs no privilege below the hard limit. The
  step above logs both limits.
- Two tests skip themselves on a host that doesn't deliver what they test:
  B/IP's `127.255.255.255` broadcast and B/IPv6's loopback multicast. Their
  output says when they skipped.

### Tool pins

The Linux jobs install their tools on the runner each time; there is no job
image.

- **Rust.** The runner image ships rustup. Each job installs the channel that
  `rust-toolchain.toml` pins, with the minimal profile plus the
  components that file lists, makes it the default, and uninstalls the
  image's own toolchains: rust-cache hashes every installed toolchain into its
  key, and the image's `stable` moves with image updates, which would start
  the jobs cold each time. The Clippy job adds the `x86_64-pc-windows-msvc` and
  `aarch64-apple-darwin` standard libraries for the per-crate default-features
  check, and the bare-metal targets in `NO_STD_TARGETS` for the `no_std`
  check (neither clippy nor rustdoc links, so nothing else is needed) and in
  `FIRMWARE_TARGETS` for the [firmware link test](#firmware-link-test), which
  links with the `rust-lld` that rustc ships. The
  MSRV job adds the MSRV toolchain and sets `RUSTUP_TOOLCHAIN` to it.
  [`fuzz.yml`](#fuzzing) installs only the dated nightly in
  `RUST_FUZZ_NIGHTLY` and sets `RUSTUP_TOOLCHAIN` to it the same way.
- **Tools.** [`.github/ci-pins.env`](../.github/ci-pins.env) pins
  cargo-nextest, cargo-deny, cargo-audit, maturin and cargo-fuzz, each with the
  SHA-256 of its Linux x86_64 release archive, the MSRV, the fuzz nightly and
  the native tests' Python version. [`scripts/ci/install-tools.sh`](../scripts/ci/install-tools.sh)
  downloads the tools a job names, checks each archive against its digest
  before unpacking it, and puts the binaries in `~/.cargo/bin`.
  `native-tests.yml` reads the cargo-nextest, maturin and Python versions from
  the same file.
- **apt.** Each job names its packages in `APT_PACKAGES` (`libpcap-dev` for
  Clippy, Test and MSRV; `python3-venv` and `libpython3-dev` for the Python
  job) and installs only those the image lacks, so a job whose packages are
  all there skips `apt-get update`. The image already has the C toolchain,
  `pkg-config`, CMake, Perl, `file`, `jq`, Python 3.12 and `openssl`. Nothing
  needs libudev: `serialport` builds without it.

The Lint job's "Check the pins" step fails when:

- a line of `.github/ci-pins.env` isn't a comment, blank, or `NAME=value` with
  a plain value (letters, digits, dots and hyphens, starting with a letter or
  digit), or a name is set twice;
- a `*_SHA256` value isn't 64 lowercase hex digits;
- `rust-toolchain.toml`'s channel isn't an exact release, or
  `RUST_FUZZ_NIGHTLY` isn't a dated nightly (`nightly-YYYY-MM-DD`);
- `RUST_MSRV` differs from `Cargo.toml`'s `rust-version` or from
  `scripts/ci/check-msrv.sh`'s default toolchain.

**Bumping a pin:**

- **Toolchain:** `rust-toolchain.toml`; both workflows install from it. A new
  compiler changes the firmware images' sizes, so re-measure
  `firmware/link/size-budget.toml` with it: `bash
  scripts/ci/check-firmware-link.sh --suggest` prints a budget table for each
  image, each size plus 12% (see [Firmware link test](#firmware-link-test)).
  Copy the tables into the file, and the sizes the check then prints into
  its comments. Then re-measure `firmware/link/stack-budget.toml` the same
  way: `bash scripts/emu/emulate-firmware.sh --suggest` runs the
  [emulation check](#emulation-manual) and prints its tables, each peak
  plus 25%. Copy them into the file, and the peaks into its comments and
  that section's table.
- **MSRV:** `Cargo.toml`'s `rust-version`, `RUST_MSRV` and
  `scripts/ci/check-msrv.sh` together.
- **Fuzz nightly:** `RUST_FUZZ_NIGHTLY`, normally to a nightly of the
  release `rust-toolchain.toml` pins (`rustc +nightly-<date> -V` names the
  version). Run `bash scripts/ci/fuzz.sh check` and `build` with it first:
  a new nightly can bring new clippy lints.
- **A tool:** its `*_VERSION` and `*_SHA256` together. The digest is of the
  archive the comment above it names:
  `curl -sSfL <url> | sha256sum`.

The [release](#release) reads the same file for pins of its own for its
[builds](#builds): the manylinux2014 images' digests, rustup-init's version
and digests, and libpcap's version and digest. The same format check covers
them. Its builds install maturin from
`scripts/release/maturin-requirements.txt`, by sha256, at the
`MATURIN_VERSION` here.

### Caches

GitHub gives the repository 10 GB of Actions cache. Past that it evicts the
least recently used entries, which can be `dev`'s current ones, and it evicts
any entry unused for seven days. On 4 October 2026 the caches had reached
12.3 GB, mostly entries no run would restore again (#1471).

- **Saves.** Only runs on `dev` save Rust caches: merges, the weekly run and
  manual runs there. PR runs only restore them, and so does the
  [flake hunt](#flake-hunt), which never saves. The
  [fuzz corpora](#fuzzing) follow the same rule: only scheduled and manual
  `fuzz.yml` runs on `dev` save one, `fuzz-corpus-<target>-<run id>-<attempt>`.
- **Pruning.** After a green run on `dev`, each workflow's **Prune caches**
  job runs [`scripts/ci/prune-caches.sh`](../scripts/ci/prune-caches.sh) with
  `actions: write`. It keeps the two newest rust-cache entries of each family
  (one job on one OS: the key without its environment and lock hashes, such
  as `v0-rust-test-Linux-x64`) and every other entry on `dev`, such as
  setup-node's, and deletes the rest: older Rust entries, and every entry on
  another ref. It logs each deletion with its key, ref, size and reason, and
  the total before and after.
- **Out-of-order runs.** Runs on `dev` don't always finish in merge order,
  so the newest entry of a family can come from an older merge. Only the run
  for `dev`'s current head prunes (an older one logs that and stops), and
  keeping two entries per family leaves the current head's in place even then. To see what it would delete, with `gh`
  signed in: `bash scripts/ci/prune-caches.sh --dry-run jscott3201/rusty-bacnet`.
- **Budget.** One set of Rust caches is about 2.3 GB: the four native jobs'
  took 1.26 GB on 4 October 2026, and the four Linux jobs' about 1 GB, going
  by the same jobs' caches on Forgejo (Clippy 320 MB, Test 300 MB, MSRV
  245 MB, Python 170 MB). setup-node's cache on `dev` adds 110 MB. Keeping
  two sets makes the steady state about 4.7 GB. A merge that changes
  `Cargo.lock` or the toolchain saves a third set, which the prune after it
  removes, so the peak is about 7 GB, plus 110 MB for each PR that changed
  `website/**` since the last prune. That leaves about 3 GB of headroom.

## Native tests (macOS and Windows)

[`.github/workflows/native-tests.yml`](../.github/workflows/native-tests.yml)
runs the tests, clippy and rustdoc natively on GitHub-hosted macOS
(`macos-latest`, Apple Silicon) and Windows (`windows-latest`, the MSVC
toolchain) runners (#950). Each platform has two jobs, which run in parallel:

- **Tests (macOS arm64)** and **Tests (Windows x86_64)**: the workspace tests,
  the stack guard and the doctests, which share one test build;
- **Lint and Python (macOS arm64)** and **Lint and Python (Windows x86_64)**:
  clippy, rustdoc, the CLI's default-feature tests, and the Python bindings
  with their Python and Rust tests.

**Native OK** waits for all four and passes only when every one succeeded. It
runs even when a job failed or was cancelled, so it always reports, like
`ci.yml`'s `CI OK`. With `CI OK`, it is one of the two checks the merge gate
reads.

**Trigger.** Every PR to `dev` or `main`, every push to them (merges), and a
manual dispatch. Tag pushes don't run it. A merge to `dev` that could have
been a fast-forward to a PR head whose run passed `Native OK` skips the four
jobs as `ci.yml` skips its Lean ones
([Skipping identical merge runs](#skipping-identical-merge-runs)).

**Steps.** `NATIVE_FEATURES` is the `features=` list in
[`scripts/ci/local-macos.sh`](../scripts/ci/local-macos.sh), which the
workflow reads: every optional feature except the Linux-only `serial-gpio`
and `ethernet`, including per-crate ones such as `bacnet-endpoint/sc-tls`, and
`serial` (#958) among them. Windows also leaves out `bacnet-cli/pcap`, which
needs the Npcap SDK. The Tests jobs run:

```bash
cargo nextest run --workspace --exclude rusty-bacnet --locked --features "$NATIVE_FEATURES" --profile ci
# stack guard (#953): STACK_GUARD_TESTS again with 1 MiB thread stacks, after
# `ulimit -s 1024` on macOS so the CLI tests' `bacnet` processes get a 1 MiB
# main thread, as on Windows
RUST_MIN_STACK=1048576 cargo nextest run --workspace --exclude rusty-bacnet --locked --features "$NATIVE_FEATURES" --profile ci -E "$STACK_GUARD_TESTS"
cargo test --doc --workspace --exclude rusty-bacnet --locked --features "$NATIVE_FEATURES"
```

The Lint and Python jobs run:

```bash
cargo clippy --workspace --exclude rusty-bacnet --all-targets --locked --features "$NATIVE_FEATURES" -- -D warnings
cargo clippy -p rusty-bacnet --all-targets --locked -- -D warnings
cargo clippy -p bacnet-cli --no-default-features --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --exclude rusty-bacnet --no-deps --locked --document-private-items --features "$NATIVE_FEATURES"
RUSTDOCFLAGS="-D warnings" cargo doc -p rusty-bacnet --no-deps --locked --document-private-items
cargo nextest run -p bacnet-cli --locked --profile ci   # the CLI's feature-off tests
# Python $PYTHON_VERSION from actions/setup-python, in a fresh venv; the
# versions come from .github/ci-pins.env
python -m pip install "maturin==$MATURIN_VERSION"
maturin develop -m crates/rusty-bacnet/Cargo.toml --locked
python -m unittest discover -s crates/rusty-bacnet/tests
cargo nextest run -p rusty-bacnet --locked --profile ci
```

The CLI's default-feature tests build `bacnet-cli` with its default features,
which the Tests job's build can't reuse, so they run in the shorter job.
PyO3 builds link setup-python's interpreter (`PYO3_PYTHON`), not the venv's.
Within a job every step runs even when an earlier one failed, so one run
reports each failure. `STACK_GUARD_TESTS` (in the workflow's `env`)
selects every server, client, endpoint, integration and CLI test, the
benchmark SC mTLS tests and bacnet-transport's BACnet/SC tests; the guard step
runs `--no-run` first because rustc reads `RUST_MIN_STACK` too, and adds about
two minutes to each Tests job. The per-crate default-feature checks
(`scripts/ci/check-default-features.sh`) aren't here: `ci.yml`'s Clippy job
runs them for Windows and macOS too, cross-checked (see
[Local checks](#local-checks)).

Before anything else, each Windows job stops the Microsoft Compatibility
Appraiser (#1003). Its `CompatTelRunner.exe` can take all four of the runner's
CPUs for seconds at a time, enough to blow a test's timing budget. The runner
image already disables the scheduled tasks that run it (those in
`\Microsoft\Windows\Application Experience\`). During a job, the Inventory and
Compatibility Appraisal service (`InventorySvc`) starts it instead, several
times, with its software-inventory module (`-m:aeinv.dll`). A diagnostic run
with process-creation auditing caught three launches in one job, the last of
which had used 53 s of CPU in five minutes; with this step, a second run had
none (October 2026, image windows-2025-vs2026). The step disables and stops that
service, disables any task that runs `CompatTelRunner.exe` in case a newer
image re-enables one, stops any running copy, and logs what it found. It
takes about two seconds and never fails the job.

**Toolchain and tools.** Both runner images ship rustup. The workflow reads
the channel and components from `rust-toolchain.toml`, so it has no toolchain
version to keep in step, and installs them with rustup's minimal profile. The
file's default profile would add `rust-docs`, thousands of small files that
took the step to about a minute on Windows, and up to three; without them it
takes 10 to 20 seconds. cargo-nextest is a prebuilt binary from
`taiki-e/install-action`, which checks the download against its own manifest,
and maturin comes from PyPI; their versions, and Python's, come from
[`.github/ci-pins.env`](../.github/ci-pins.env), as `ci.yml`'s do.
aws-lc-sys, which `sc-tls` pulls in, builds with the images' own C tools: on
Windows, MSVC with the NASM and CMake already on `PATH`. The SC tests make
certificates with the runner image's `openssl`.

**Efficiency.** The workflow can only read the repository
(`permissions: contents: read`; Prune caches alone can also write Actions
caches), and each job stops after 60 minutes. A newer push to a PR cancels
its running run. On `dev` each commit gets its own concurrency group, keyed by
its SHA, so no run is cancelled or replaced while pending and every merge gets
its own result. `Swatinem/rust-cache` keeps dependency builds, keyed per job
and OS on the toolchain, `Cargo.lock`, the manifests and `NATIVE_FEATURES`.
Only `dev` saves it, and only from a successful job, so a failed or cancelled
run never leaves a partial cache that later runs would restore by exact key.
GitHub lets a PR's run restore the default branch's (`dev`) cache, so every PR
starts from the last good `dev` build; see [Caches](#caches) for pruning. The
cache holds dependencies only, so most of a job is compiling the workspace and
its tests. In October 2026, with the jobs split, a run took about 11 to 12
minutes, cold or warm. The Windows Tests job, the usual floor, took 10 to 11.5
minutes; the macOS one 4.6 to 10.2, as macOS hosts vary widely in speed; the
Lint and Python jobs 6 to 8 minutes warm and 8 to 11 cold. With one job per
platform a run had taken a median 18.4 minutes, Windows being the slowest.

Two Windows build speedups were measured and left out. A ReFS Dev Drive for
the target directory, `CARGO_HOME` and `RUSTUP_HOME` made the test build two
to three minutes slower: Defender's real-time scanning is already off on the
runner image, with `C:\` and `D:\` excluded, and `D:` is a local NVMe disk, so
the virtual disk only added a layer. Linking with `rust-lld` instead of
MSVC's `link.exe` saved no more than the spread between runner VMs (the same
build took 3.6 to 5.1 minutes on different VMs).

**macOS capacity.** GitHub's free plan runs at most 5 macOS jobs at once
across the account, and the native tests already reach that cap: over 400
runs on 3 and 4 October 2026, when every pushed branch ran them, a macOS job
waited a median 0.1 minutes to start, 7.3 at the 90th percentile and up to 28.
Splitting macOS gives each run two macOS jobs and about 12% more macOS
minutes. Replaying those runs' arrival times against the caps, with each job's
time drawn from their history, the split came out ahead at 4 October's load
(median 10.9 minutes to a result against 13.0; 90th percentile 11.8 against
15.3) and at the median over both days (11.4 against 13.7). It queued longer
only in bursts of about 20 runs an hour, which fill the cap with either
layout: over both days its 90th percentile was 24.7 minutes against 22.9, and
over the busiest seven hours 36 against 29.

**Portable tests.** What the first Windows and macOS runs showed (#950):

- Text files check out with LF line endings on every OS (`.gitattributes`);
  Windows checkouts would otherwise get CRLF from `core.autocrlf`.
- Don't stand a loopback address in for a broadcast address. Windows reports
  delivery to `127.0.0.1` as unicast, and B/IP then drops an
  Original-Broadcast-NPDU. Drive a BBMD with what may arrive by unicast
  (Distribute-Broadcast-To-Network, Forwarded-NPDU), or call the handler with
  `Delivery::Broadcast`.
- Don't rely on sending to `255.255.255.255`: GitHub's macOS runners refuse it
  with `EHOSTUNREACH`.
- Compare `io::ErrorKind`, or the error the OS gives for the same call, not
  Unix error text.
- Stacks are smaller on Windows: the main thread gets 1 MiB (8 MiB on Linux
  and macOS), so `#[tokio::main]` binaries box their large futures, as
  `bacnet` does. Test threads get 2 MiB everywhere, and debug-build async
  fixtures can fill that. A debug build gives every future an async fn awaits
  at least one stack slot of its own, sized to the whole future, in that fn's
  poll frame, so a fixture or startup path that awaits many large futures has
  a large frame. Create such a future in a helper that boxes it
  (`boxed(|| step()).await`, as the server, SC and fixture code does since
  #953); `Box::pin(step())` in the caller still builds the full-size temporary
  in the caller's frame. The stack guard step catches regressions; to see how
  close a test is, bisect `RUST_MIN_STACK` (or `ulimit -s` for a binary's main
  thread). On nightly, `-Zprint-type-sizes` gives future sizes, and
  `-Cremark=prologepilog` gives each function's frame size.
- Another socket may bind `127.0.0.1:P` beside a wildcard `0.0.0.0:P` on
  Windows unless the first socket set `SO_EXCLUSIVEADDRUSE`, which an
  ephemeral B/IP or B/IPv6 socket now does. Linux refuses that bind. macOS
  refuses a plain one, but not one from a socket that sets `SO_REUSEADDR`, and
  has no option to prevent it.
- `localhost` resolves to `::1` first on Windows, and a refused loopback
  connect takes about 2 seconds there. The SC dialer races a host's
  addresses (RFC 8305 style), so a dial to `localhost` against an IPv4-only
  listener costs the 250 ms attempt delay rather than 2 seconds; a test whose
  timing depends on a dial must allow for it.

**Reading a run** (either workflow):

```bash
gh pr checks <n> -R jscott3201/rusty-bacnet   # every check on a PR's head
gh run list -R jscott3201/rusty-bacnet --workflow native-tests.yml --branch <branch>
gh run view -R jscott3201/rusty-bacnet <run id> --log-failed
gh workflow run native-tests.yml -R jscott3201/rusty-bacnet --ref <branch>  # run by hand
```

## Flake hunt

[`.github/workflows/flake-hunt.yml`](../.github/workflows/flake-hunt.yml) runs
tests over and over to catch failures too rare for a PR's single run. A
deadlock that failed about 3 runs in 1,000 got past PR CI and showed up only
as a rare macOS Python failure on `dev`. No check here is required for
merging; every job's name starts with `Hunt:`, so none can pass for `CI OK`
or `Native OK`, and the release's [CI gate](#ci-gate) reads only `ci.yml` and
`native-tests.yml` runs.

**When.** Nightly at 07:37 UTC, on the default branch (`dev`); by hand (see
below); and on a PR to `dev` or `main` that changes the workflow or its
script, [`scripts/ci/flake-hunt.py`](../scripts/ci/flake-hunt.py), with small
values (2 Python rounds, 2 minutes of stress), so that PR exercises the hunt
in about 15 minutes. A newer push to the PR cancels its older run; nightly and
manual runs are never cancelled, and a manual run waits for the one before it.

**Jobs.** **Hunt: settings** checks the values and sizes each hunt job's
timeout to fit them. Each hunt then runs on Linux x86_64 (`ubuntu-24.04`) and
macOS arm64 (`macos-latest`):

- **Hunt: Python suite** builds the extension as CI does (`ci.yml`'s Python
  bindings job on Linux, `native-tests.yml`'s Lint and Python job on macOS)
  and runs `python -m unittest discover -s crates/rusty-bacnet/tests` round
  after round, 20 by default. A round took about a minute in October 2026
  (63 s on Linux, 67 s on macOS). A failing round doesn't stop the loop:
  each failing test is recorded with its round, and the round's output goes
  in the job log and the job's artifact. The suite runs with `-v`, so a hang
  or crash names the test it happened in, and with `-X faulthandler`. A round
  still running after 10 minutes is a hang: SIGABRT makes faulthandler print
  every thread's Python stack, then the round's processes are killed. Rounds
  stop early, with a warning, when the next could overrun the job's timeout.
- **Hunt: nextest stress** runs `cargo nextest run --stress-duration 20m`
  over `bacnet-server`, `bacnet-client`, `bacnet-endpoint` and
  `bacnet-network`, about 3,500 tests an iteration, with the `ci` profile and
  CI's features for those crates: `LINUX_FEATURES` from `ci.yml` on Linux and
  `local-macos.sh`'s list on macOS, less the features of crates outside the
  build, which cargo refuses. The profile's `fail-fast = false` keeps the run
  going through failures, and its 2-minute limit per test turns a hang into a
  `TIMEOUT`. nextest finishes the iteration that is running when the time is
  up. Each failing test is recorded with its iteration, and
  `--status-level slow` keeps passing tests out of the log.

With the defaults each hunt job has a 60-minute timeout. Each job restores the
Rust cache of the CI job that builds the same way (Python bindings, Test,
Lint and Python, Tests) by naming that job in rust-cache's `key`, with
`add-job-id-key: false`, and saves nothing. rust-cache hashes every `CARGO*`,
`RUST*`, `CC*`, `CFLAGS*`, `CXX*` and `CMAKE*` variable and the feature list
into its key, so the workflow's `env` must keep matching `ci.yml`'s and
`native-tests.yml`'s; if it drifts, the jobs only build from cold.

**Bigger runs.** Dispatch it with any of three inputs:

```bash
gh workflow run flake-hunt.yml -R jscott3201/rusty-bacnet --ref dev \
  -f python_rounds=100 -f stress_duration=2h
gh workflow run flake-hunt.yml -R jscott3201/rusty-bacnet --ref dev -f stress_count=50
```

- `python_rounds`: 1 to 300 (default 20).
- `stress_duration`: `<n>s`, `<n>m` or `<n>h`, up to 5 hours (default `20m`).
- `stress_count`: 1 to 1,000 iterations, which replaces the duration
  (default empty).

The settings job sizes the timeouts, up to GitHub's 6-hour limit: 20 minutes
plus 2 a round for the Python jobs, and for the stress jobs 40 minutes plus
the duration, or 30 plus 3 an iteration; never under 60. `--ref` picks the
branch. A long run holds two of the account's 5 macOS slots (see
[Native tests](#native-tests-macos-and-windows), "macOS capacity") for its
whole length, so PRs' native tests queue longer meanwhile.

**Where failures go.** When a hunt job fails, or times out in a run nobody
cancelled, **Hunt: report** gathers every job's records and opens the issue
**Nightly flake hunt: failures** with the `flake-hunt` label, which it
creates the first time. While that issue is open it comments on it instead:
the oldest open issue with the label, or else with that exact title. Reports
run one at a time, so two can't open two issues. Each report gives the run's
URL, the commit, the settings, each job's result, and the failing tests, each
with its job and the Python rounds or stress iterations it failed in. Close
the issue once its failures are fixed or filed; the next failure opens a new
one.

- Only the report job can write issues (`issues: write`); every other job only
  reads the repository.
- Test names come from test output, so they reach the issue only through the
  jobs' record files and inside a code block, never a shell command or a
  `${{ }}` expression in a script.
- The artifacts, `flake-hunt-<job>-<platform>`, hold each job's records, the
  output of each failing round and the stress log, for 14 days.
- On a PR the report always runs, as a dry run: it writes the report to the
  job summary, logs what it would create or comment on, and writes nothing.

## Fuzzing

[`.github/workflows/fuzz.yml`](../.github/workflows/fuzz.yml) fuzzes the
MS/TP and B/IP codecs (#1610) and the MS/TP master core (#1640) with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), which drives libFuzzer.
No check here is required for merging: every job's name starts with `Fuzz:`,
so none can pass for `CI OK` or `Native OK`, and the release's
[CI gate](#ci-gate) reads only `ci.yml` and `native-tests.yml` runs.

**Targets.** Each fuzzed crate has a fuzz crate, `crates/<crate>/fuzz`,
outside the workspace, with its own `Cargo.lock`. Its `src/lib.rs` holds what
its targets share: for the MS/TP codec, an encoder and a frame check written
from the wire layout with bit-at-a-time CRCs (the CRC-32K included) and a COBS
of its own, without calling the codec, so a fault the codec's encode and
decode share can't cancel out; for both codecs, a check that encodes through
every sink; for the core, the harness that drives it and the checker that
judges it. The oracle of each target is in the doc comment at the top of its
file.

| Target | Input | Oracle |
| --- | --- | --- |
| `mstp_decode` | any octets | `decode_frame_in_place` accepts exactly the inputs that start with a whole valid frame, standard or extended, by the harness's own check, with its length, fields and decoded data, and changes only an extended frame's data field; `decode_frame` agrees without changing anything, except that a good extended header stops it with `ExtendedFrame` whatever follows; a refusal names the first check that failed, in the codec's order; an accepted frame re-encodes to the same octets unless the encoder's stricter checks refuse it or the input was an extended frame in a lenient encoding, which re-encodes to the harness's strict one |
| `mstp_encode` | any frame: every type, source and data length, up to 639 octets for a standard type and 2,099 for a COBS-encoded one | the outcome worked out from the crate docs, header checks in the codec's order. Every sink (a slice through `encode_into` and through `FrameSink`, `Vec`, `BytesMut`, `heapless::Vec`) holds exactly the expected octets, or refuses with the expected error; a refused encode leaves each sink as it was, compared before and after; each result decodes back to the frame |
| `mstp_stream` | valid standard and extended frames, frames of the reserved COBS types, frames damaged past the header, raw noise and noise without `0x55`, pushed into 1 to 2,117 octets of storage in pieces of any size, with empty pushes, drains and resets | each push checked against a shadow of the decoder's buffer and of any skip in progress: refusals, `NeedMore`, `Invalid` (one noise run), `Malformed` (error, discard, destination, and whether it skips the frame by count) and `Frame` (with its decoded data), the buffered length and the phase; never wedged; at the end, frames in order and each once, and every guaranteed generated frame delivered at its offset |
| `bip_decode`, `bip6_decode` | any octets | no panic; an accepted datagram re-encodes to itself, unless its result code is one the annex doesn't define |
| `bip_encode`, `bip6_encode` | any `Bvll` or `Bvll6`, some within 8 octets of the 65,535-octet limit on either side | the octets the Annex J.2 or U.2 layout gives, built field by field; an undefined result code and a message too long for the Length field refused; sinks as for `mstp_encode`; each result decodes back to an equal message |
| `mstp_core` | one `MasterCore`, its address, Max_Master, Max_Info_Frames, timers, extended frames and storage from the input, then up to 2,048 steps with injected time: octets, receive errors and silences; whole frames of every type for it, another station or broadcast, on the line or from a host assembler, some damaged or cut off part way; deadlines on time or late; the adapter's starts (some refused or deferred), completions with or without echo, and aborts; queued NPDUs, oversize ones included; request decisions; calls stamped in the past | after every step, by a checker that reads only the line and the adapter's side, never the core's state: no panic; data, Tokens and polls only while the station holds the token, as the frames on the line show it, or as a token retry, a successor search or token generation when Clause 9.5.6 allows them, and otherwise only answers to what was asked, once and in time; at most Max_Info_Frames data frames per token use; poll sweeps one address at a time within Max_Master; T_turnaround before every frame; owed answers sent on a quiet line in time; Tokens to NS and a use's first poll to PS + 1 or TS + 1; queues and delivery slots within their sizes; the adapter contract on every action (one frame at a time, starts only for polled frames, completion only after a start, echo ignored, payloads in the action buffers); deliveries matching frames heard; a call stamped earlier than the last doing what it would at the last |

A generated frame in `mstp_stream` is guaranteed when it fits the storage, no
reset falls inside it, and no `0x55` lies between it and the anchor: the end
of the last frame delivered before it, or dropped and skipped by count before
it, the last reset, or the stream's start. From there the decoder starts
fresh and nothing can start a candidate before the frame. Noise with a `0x55`
may hold a header that passes its checks and runs into the next frame, which
the decoder then has to deliver, or drop and skip, whole. A frame dropped
after a good header (too long for the storage, a reserved COBS type, or data
that fails its check) hides nothing: the decoder skips exactly the octets its
header declares (#1609).

`mstp_core`'s checker rebuilds the line with a stream decoder of its own and
follows the token through the master state machine at the level the line
shows: free, held, or passed and watched for its successor's use, with NS
and PS as far as the line shows them. It shares only the codec with the
core, to build and decode frames; the driver reads the core's `snapshot()`,
`counters()` and `next_deadline()` only to confirm that a refused call or
echo changed nothing, to schedule deadlines, and for traces
(`MSTP_CORE_FUZZ_TRACE`). Where the
line can't tell what the core did (a late deadline it didn't service, a
frame its decoder may or may not count as invalid, a waiting frame it
dropped silently on the way back to IDLE), the checker allows each choice the
core could have made, so it can miss a fault there but never fails a correct
core. A fault planted in the core, taking up a Token addressed to another
station, failed the target within two seconds from an empty corpus; one more
data frame per token use than allowed failed it within ten, and Reply
Postponed handled as an abandoned request within five.

**When.** On a PR to `dev` or `main` that changes
`crates/bacnet-mstp-codec/**`, `crates/bacnet-bip-codec/**`,
`crates/bacnet-mstp-core/**` (fuzz crates included),
`crates/bacnet-time/**` or `crates/bacnet-npdu/**`, the workflow,
`.github/ci-pins.env`, `scripts/ci/fuzz.sh` or
`scripts/ci/install-tools.sh`, each target runs for 60 seconds. Twice a
week (Sundays and Wednesdays at 03:41 UTC, on `dev`) and by hand, each runs
for 10 minutes. A newer push to the PR cancels its older run; scheduled and
manual runs are never cancelled.

**Jobs.** Everything goes through
[`scripts/ci/fuzz.sh`](../scripts/ci/fuzz.sh), on the nightly in
`RUST_FUZZ_NIGHTLY` (cargo-fuzz needs nightly-only flags), with cargo-fuzz at
`CARGO_FUZZ_VERSION` from its release archive, checked against its SHA-256
(see [Tool pins](#tool-pins)). A prebuilt binary takes about a second to
fetch, so there is nothing to cache. The script passes cargo-fuzz `--target`
with the nightly's host triple: cargo-fuzz otherwise builds for the triple it
was built for itself, which for that archive is musl, where AddressSanitizer
can't work.

- **Fuzz: check and build** lists the targets from the fuzz crates'
  `[[bin]]` entries, checks each fuzz crate's lockfile
  (`cargo fetch --locked`), rustfmt and clippy with warnings denied, runs
  its tests (for `mstp_core`, the checker's own and a replay of the inputs
  saved in `crates/bacnet-mstp-core/fuzz/regressions/mstp_core/`), and
  builds every target.
- **Fuzz: `<target>`**, one job per target from that list, so a new
  `[[bin]]` runs without a workflow change, builds its target and runs it.
  cargo-fuzz's defaults apply: optimized, with debug assertions, overflow
  checks and AddressSanitizer. An input that runs for 10 seconds counts as a
  hang. On a scheduled or manual run on `dev` it then minimizes the corpus
  (`fuzz.sh cmin`) and saves it.
- **Fuzz: advisories**, on scheduled and manual runs only, checks each fuzz
  crate's lockfile against the RustSec advisories with Cargo Deny (at
  `CARGO_DENY_VERSION`). It is a job of its own, so an advisory doesn't hold
  up the fuzzing.

**Corpora.** Each job restores the newest corpus its target has, saved as
`fuzz-corpus-<target>-<run id>-<attempt>` by a scheduled or manual run on
`dev` (failed runs included: the failing input isn't in the corpus), and
minimized before it is saved, so it doesn't grow from run to run. A PR run
replays it before fuzzing, so an input that once found new code paths runs
again on every change. GitHub evicts an entry that goes unused for seven
days; two scheduled runs a week keep the newest one in use even when one
starts late. The [prune](#caches) keeps such entries on `dev`, but nothing
restores an older one, so that eviction removes them. No corpus is checked
in.

**Lockfiles.** Each fuzz crate's `Cargo.lock` is committed, and every build
checks it first, so the fuzz builds use the dependencies it names. It also
records what the root `Cargo.toml` sets for the crate it fuzzes: its version
and the workspace's requirements for its dependencies. A version bump or a
moved requirement leaves the lockfile stale, so `ci.yml`'s required Lint job
checks every lockfile too (`bash scripts/ci/fuzz.sh lock --check`, any
toolchain). `bash scripts/ci/fuzz.sh lock` brings them up to date: cargo
re-resolves only the entries the manifests no longer allow and keeps every
other dependency where it was. Run it with the release's version bump (step 1
of [the release](#trigger-and-dry-run)) and in any PR that changes a
workspace dependency of a fuzzed crate, and commit the lockfiles. When such a
change moves a dependency the fuzz crate also names itself (`bytes`,
`heapless`) to a new major version, update the fuzz crate's `Cargo.toml` to
match, or the sinks won't build.

Cargo Deny's gate doesn't cover the fuzz crates: they are unpublished CI
tools whose binaries never ship. **Fuzz: advisories** checks their
advisories; their bans and sources pass too
(`cargo deny --manifest-path crates/<crate>/fuzz/Cargo.toml check`), and
their licenses fail only because libFuzzer's `NCSA` (in `libfuzzer-sys`)
isn't on the allow list and the fuzz crates name no license.

**Running locally.** Install the pinned nightly and cargo-fuzz, then use the
script as CI does. It works on macOS (Apple Silicon) as on Linux.

```bash
rustup toolchain install nightly-2026-08-15 --profile minimal --component rustfmt,clippy  # RUST_FUZZ_NIGHTLY
cargo install cargo-fuzz --version 0.13.2 --locked    # CARGO_FUZZ_VERSION
bash scripts/ci/fuzz.sh list                          # the targets
bash scripts/ci/fuzz.sh check                         # lockfiles, rustfmt, clippy, tests
bash scripts/ci/fuzz.sh build                         # every target
bash scripts/ci/fuzz.sh run mstp_stream 60            # one target for 60 seconds
bash scripts/ci/fuzz.sh run mstp_core 600             # the master core for 10 minutes
FUZZ_WORK=/tmp/fuzz bash scripts/ci/fuzz.sh run bip_encode 600 -max_len=16384
```

Arguments after the seconds go to libFuzzer. Without `FUZZ_WORK`, the corpus
and anything a run finds stay in the fuzz crate's `corpus/<target>` and
`artifacts/<target>`, which git ignores.

**Triage.** When a target fails, its job uploads the artifact
`fuzz-<target>` for 30 days (a re-run of the job replaces it): what libFuzzer saved (`crash-*` or `timeout-*`,
which the job minimizes into `minimized-from-*`, or `oom-*` or `leak-*`,
which it doesn't) and the run's log, whose panic message names the check that
failed. To reproduce it:

```bash
gh run download <run id> -R jscott3201/rusty-bacnet -n fuzz-mstp_stream -D /tmp/fuzz-mstp_stream
bash scripts/ci/fuzz.sh build mstp_stream
cd crates/bacnet-mstp-codec
host=$(rustc +nightly-2026-08-15 -vV | sed -n 's/^host: //p')    # as fuzz.sh does
cargo +nightly-2026-08-15 fuzz fmt --target "$host" mstp_stream /tmp/fuzz-mstp_stream/minimized-from-<hash>  # the input as a Rust value
cargo +nightly-2026-08-15 fuzz run --target "$host" mstp_stream /tmp/fuzz-mstp_stream/minimized-from-<hash>  # replay it
```

`fuzz fmt` prints a structure-aware target's input (frames, noise, pushes) as
the target decoded it. Decide whether the crate or the oracle is wrong; the
oracle's reasoning is in the target's doc comment (for `mstp_core`, in its
`src/checker.rs`, and the panic names the step, the time and what the checker
believed about the token). Turn a bug into a regression test in the fuzzed
crate's own `tests/`, fix it, and check that the replay passes. A fault in
`mstp_core`'s checker instead gets its minimized input saved in the fuzz
crate's `regressions/mstp_core/`, which the crate's tests replay. `bash
scripts/ci/fuzz.sh tmin <target> <input>` minimizes an input further.

## Local checks

Use the Rust release that `rust-toolchain.toml` pins, which rustup selects in
the checkout. The [native tests](#native-tests-macos-and-windows) run the
macOS tests, clippy and rustdoc on every PR, so a local macOS run is
optional: a quicker check before pushing changes that can affect macOS
(transports, sockets, TLS, platform `cfg`, build scripts, dependencies). It
isn't merge evidence.

```bash
bash scripts/ci/local-macos.sh          # lint, clippy, rustdoc, macOS tests
bash scripts/ci/local-macos.sh --quick  # lint, clippy and rustdoc only
```

`serial-gpio` and `ethernet` are Linux-only features, so macOS uses every
other optional feature, `serial` included (#958): the serial port crates build
on all three systems, and its pseudo-terminal tests run on macOS too. The
native-tests workflow reads the same list from the script. That includes
per-crate features such as `bacnet-endpoint/sc-tls` and `bacnet-cli/{sc-tls,pcap,serial}`, which nothing else in
the workspace turns on, so a transport-only list never built them (#906). CI's
`LINUX_FEATURES` is the same list plus `bacnet-transport/{serial-gpio,ethernet}`
and `bacnet-integration-tests/ethernet`.

The script also runs the PyO3 crate's Rust tests. pyo3 links the libpython of
`PYO3_PYTHON` if it's set, else of the active venv, else of the first `python`
or `python3` on `PATH`. The Homebrew and python.org framework builds both ship
the shared library.

Clippy and rustdoc deny warnings (#902). Every public item must be documented:
`missing_docs` is `deny`, and only the unpublished `bacnet-benchmarks` opts out.
Clippy runs six ways:

- the workspace with every feature;
- the PyO3 crate on its own;
- `bacnet-cli` with no default features (without the TUI), on Linux, macOS and Windows;
- the `no_std` crates (`bacnet-time`, `bacnet-npdu`, `bacnet-mstp-codec`,
  `bacnet-mstp-core`, `bacnet-bip-codec`, `bacnet-bip-core`,
  `bacnet-router-core`, and `bacnet-types`,
  `bacnet-encoding` and `bacnet-services` with `alloc` in place of their
  default `std`, and with neither) on
  bare-metal targets, once per feature set
  (`scripts/ci/check-no-std.sh`). A host build always has `std` and an
  allocator, so only a target without them shows that a crate needs neither.
  CI's Clippy job passes `NO_STD_TARGETS`: `thumbv6m-none-eabi`,
  `thumbv7em-none-eabihf` and `riscv32imac-unknown-none-elf`. On
  `thumbv6m-none-eabi`, the builds that use `bytes` bring in
  `firmware/nostd-harness` for `portable-atomic`'s backend (see
  [`no_std` checks](#no_std-checks)). It also reads each build's `cargo tree`
  and fails if any package in it has a `std` feature on, or is `tracing`,
  `tokio` or `socket2`: a dependency whose std leaks in through feature
  unification is caught even where it would still compile;
- `bacnet-types`, `bacnet-encoding` and `bacnet-services` with `alloc` in
  place of `std`, and with neither, and `--all-targets`, on the host, so the
  tests the Test job runs without `std` are linted too;
- each published crate alone with default features, plus the `no_std` builds
  of `bacnet-types`, `bacnet-encoding` and `bacnet-services`
  (`scripts/ci/check-default-features.sh`). This also runs rustdoc, which is
  how docs.rs builds, a rustdoc-only run of `bacnet-cli` without the TUI,
  and rustdoc-only runs of `bacnet-mstp-core` with `alloc`, of
  `bacnet-mstp-codec` and `bacnet-bip-codec` with `alloc` alone and
  `heapless` alone (#1681), and of `bacnet-router-core` with `bip` alone
  and `mstp` alone (#1679).

The last catches code that compiles only when another crate's feature unifies
in. With no arguments it checks the host; given target triples, it checks
those, side by side in one cargo run per crate. CI's Clippy job passes
`DEFAULT_FEATURES_TARGETS`: Linux, `x86_64-pc-windows-msvc` and
`aarch64-apple-darwin`, whose platform `cfg`s compile different code (#981).
Neither clippy nor rustdoc links, and no C code builds with default features,
so another target needs only `rustup target add`, not cargo-xwin or zig.

Rustdoc's every-feature run and the PyO3 crate's run pass
`--document-private-items` (#1164), so a broken intra-doc link in the docs of
a private or `pub(crate)` item fails the gate too. The flag still reports a
public item whose docs link to a private item
(`rustdoc::private_intra_doc_links` fires with or without it), so the
every-feature run replaces the public-only run rather than adding a second
one. Every module of the PyO3 crate is private, so without the flag rustdoc
would check none of its docs. The per-crate default-features run and the
feature-off rustdoc runs of `check-default-features.sh` (the `no_std` crates
with `alloc`, the MS/TP and B/IP crates with one storage feature, the
router core with one glue feature, `bacnet-cli` without `tui`) pass the flag too (#1192), so private items
that exist only in one feature configuration are covered; the run takes about
2 s longer on the host.

The individual gates are also runnable anywhere. `FEATURES` is
`LINUX_FEATURES` from `ci.yml`, without the serial-gpio and ethernet entries on macOS:

```bash
FEATURES=$(sed -n 's/^  LINUX_FEATURES: //p' .github/workflows/ci.yml)
cargo fmt --all --check
cargo clippy --workspace --exclude rusty-bacnet --all-targets --locked --features "$FEATURES" -- -D warnings
cargo clippy -p rusty-bacnet --all-targets --locked -- -D warnings
cargo clippy -p bacnet-cli --no-default-features --all-targets --locked -- -D warnings
bash scripts/ci/check-default-features.sh   # the host; or pass target triples, as CI does
cargo clippy -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --features alloc --all-targets --locked -- -D warnings
cargo clippy -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --all-targets --locked -- -D warnings
bash scripts/ci/check-no-std.sh             # needs the bare-metal targets (rustup target add)
bash scripts/ci/check-firmware-link.sh      # likewise; see "Firmware link test"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --exclude rusty-bacnet --no-deps --locked --document-private-items --features "$FEATURES"
RUSTDOCFLAGS="-D warnings" cargo doc -p rusty-bacnet --no-deps --locked --document-private-items
cargo nextest run -p bacnet-cli --locked   # the CLI's feature-off tests
cargo nextest run -p bacnet-cli --no-default-features --locked   # without the TUI
cargo nextest run -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --features alloc --locked   # no_std code, alloc
cargo test --doc -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --features alloc --locked
cargo nextest run -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --locked   # no_std code, no allocator
cargo test --doc -p bacnet-types -p bacnet-encoding -p bacnet-services --no-default-features --locked
cargo nextest run -p rusty-bacnet --locked # the PyO3 crate's Rust tests
bash scripts/ci/check-file-size.sh
bash scripts/ci/test-check-no-secrets.sh && bash scripts/ci/check-no-secrets.sh
bash scripts/ci/test-plan-skip.sh          # the merge-run skip (#1466)
python3 scripts/ci/test-check-msrv.py
python3 scripts/ci/test-firmware-size.py
python3 scripts/ci/check-named-codes.py     # the named_codes! copies match
python3 -m unittest discover -s scripts/release
python3 -m unittest discover -s scripts -p 'test_changelog.py'
python3 scripts/changelog.py check
bash scripts/ci/fuzz.sh lock --check        # the fuzz crates' lockfiles; see "Fuzzing"
actionlint .github/workflows/*.yml          # after editing a workflow
```

`changelog.py check` validates the [changelog fragments](../changelog.d/README.md)
and fails if `CHANGELOG.md`'s `[Unreleased]` section lists entries itself, so a
PR that edits it directly fails the lint job.

The no-secret scanner reports stable opaque path IDs and line numbers, never
paths or matching text, because filenames can contain credentials too. Compare
a suspected path locally with `printf '%s' "$path" | git hash-object --stdin`.
Run the file-size gate in its default strict mode, without `CHECK_FILE_SIZE_WARN=1`.

`bacnet-bip-codec` (`src/function.rs`), `bacnet-mstp-codec` (`src/frame.rs`)
and `bacnet-npdu` (`src/codes.rs`) each keep a private copy of
the `named_codes!` macro, which builds their open wire-code types
(`BvlcFunction`, `FrameType`, `NetworkMessageType` and the rest): none of them
depends on another for it (#1684). `check-named-codes.py` compares each
`macro_rules!` definition, from that line to the closing brace, with the
first, and fails with a diff when one differs, so a change to one is made to
all three. The doc comments above them may differ.

`scripts/ci/check-msrv.sh --linux-native` needs a native Linux GNU host with
the MSRV toolchain (`RUST_MSRV`) installed, Python 3, a C toolchain, `pkg-config`, `libpcap-dev`,
`cmake`, `perl`, `file` and `ldd`. It never installs tools or skips features.
CI runs it on PRs to `main`; on a Mac, rely on that job.

## Merge evidence

Before merging a PR:

- `CI OK` and `Native OK`, the two required checks, are green on the PR for
  the exact head SHA being merged;
- existing review and merge-authorization rules are met.

A `local-macos.sh` pass is optional and isn't needed to merge.

A check that was not run, failed or does not apply is never reported as passed.
Audit and deny read mutable advisory databases, so their result for a `main`
merge comes from that PR's run, not an older one. On a PR from `dev` to
`main`, the head commit also carries the `CI OK` of `dev`'s push run, which
covers only the Lean jobs, so check that the `pull_request` run's `CI OK`,
the one with MSRV and audit and deny, is green. The release's
[CI gate](#ci-gate) requires the CI run of the tag's own push, which runs
them.

## Release

[`.github/workflows/release.yml`](../.github/workflows/release.yml) builds,
tests and publishes each release on GitHub-hosted runners (#943, #1472). Every
artifact is built on a runner of its own platform and architecture, with that
platform's own toolchain, and run there before anything is published. GitHub
Pages publication remains the manual
[`docs-pages.yml`](../.github/workflows/docs-pages.yml) dispatch.

### Trigger and dry run

- **Tag.** Pushing a `v*` tag to GitHub runs the whole release.
- **Dry run.** A manual dispatch is a dry run by default (`dry_run` is true). It
  builds, checks and smoke-tests every artifact, keeps them as workflow
  artifacts, plans the publish steps read only (see
  [Publish plan](#publish-plan-dry-run)) and publishes nothing:

  ```bash
  gh workflow run release.yml -R jscott3201/rusty-bacnet --ref dev
  ```

  `dry_run=false` is accepted only on a `v*` tag, where it runs that tag's
  release again in a new run (see
  [Re-running a partial release](#re-running-a-partial-release)).
- **Pull request.** A PR to `dev` or `main` that changes what the release
  builds with runs a dry run, so a release change is tested before it merges:
  the workflow itself, `.github/ci-pins.env`, `scripts/release/**`,
  `Cargo.toml`, `Cargo.lock`, `crates/*/Cargo.toml`, `rust-toolchain.toml`,
  `crates/rusty-bacnet/pyproject.toml` and `scripts/changelog*.py`. Not
  `changelog.d/`, which nearly every PR changes. A newer push to the PR
  cancels the older dry run. The check isn't required for merging.

A dry run's notes come from the workspace version's `CHANGELOG.md` section if
it has one, otherwise from the section the waiting `changelog.d/` fragments
would make, which may be empty; `changelog.py assemble --output` writes that
into a copy, so the checkout is left alone. A dry run uses no secret, never
deploys to the `release` environment and writes nothing, so a fork's PR runs
it too.

To release:

0. Once, before the first release from this workflow: on pypi.org, under
   rusty-bacnet's Publishing settings, add a GitHub trusted publisher with
   owner `jscott3201`, repository `rusty-bacnet`, workflow `release.yml` and
   environment `release`, then remove the one that names `ci.yml`. The
   [PyPI trusted publishing](#secrets-and-environment) job stops a release
   before crates.io if this isn't done.
1. Set the workspace version, and bump `bacnet-time` (its own `version` and
   the workspace dependency's) when its API changed since its last release:
   it versions on its own (#1768), and the release skips a name and version
   already on crates.io. Update the fuzz crates' lockfiles (see
   [Fuzzing](#fuzzing)), assemble the version's `CHANGELOG.md` section from
   the fragments, add release highlights by hand under the new heading if the
   release has any, and merge:

   ```bash
   bash scripts/ci/fuzz.sh lock                # the fuzz crates' Cargo.lock files
   python3 scripts/changelog.py preview        # what the section will hold
   python3 scripts/changelog.py assemble --version 0.14.0 [--date 2026-10-21]
   ```

   `assemble` adds `## [0.14.0] - <date>` below `[Unreleased]`, with the
   entries grouped by heading and ordered by issue number, and deletes the
   fragments it used. Each entry ends with a link to the GitHub commit that
   brought its fragment into dev, found along the first-parent history, so run
   it on a branch cut from dev's tip with full history (#1188); `preview` shows
   the same links. A tag fails if any fragment is still waiting.
2. Optionally, dispatch a dry run on the release branch first: it runs
   everything the release does short of publishing.
3. Tag a commit on `main`'s or `dev`'s first-parent history (a merge commit,
   not one inside a merged branch), once its CI has passed:

   ```bash
   git tag -a v0.14.0 -m "Rusty BACnet 0.14.0"
   git push origin v0.14.0
   ```

The tag push also starts CI's heavy jobs on the tagged commit, and the release
waits for them (see [CI gate](#ci-gate)).

### Jobs

| Job | What it does |
| --- | --- |
| Validate | Runs the release script tests (`scripts/release/test_*.py`, `scripts/test_changelog.py`). Checks, with `release_version.py`, that every publishable crate has the workspace version, except `bacnet-time`, which versions on its own, and, for a tag, that the tag is `v<version>` and the commit is on `dev`'s or `main`'s first-parent history (`git rev-list --first-parent`), so a commit inside a merged PR's branch doesn't qualify. For a tag, checks that no `changelog.d/` fragment is left unassembled. Extracts the notes with `changelog_notes.py` and writes `THIRD-PARTY-NOTICES`. |
| CI gate | `ci_gate.py` on the release commit (a PR's head on a PR's dry run). On a tag it waits for the CI run the tag's push started; a dry run checks once and only warns. See [CI gate](#ci-gate). |
| Crates and sdist | `cargo publish --workspace --dry-run --locked`, which packages every publishable crate and builds each against the others as published. Then `cargo package` for the `crates` artifact, and `maturin sdist`. |
| Build (linux-x86_64, linux-aarch64, macos-x86_64, macos-arm64, windows-x86_64) | The wheels for CPython 3.11 to 3.14 (`PYTHONS`) and the CLI, on each platform's own runner. See [Builds](#builds). |
| Check the artifacts | `check_artifacts.py` over every file, then the `release-assets` artifact: what the publish jobs upload. See below. |
| Smoke test (Linux x86_64, Linux aarch64, macOS arm64, macOS x86_64, Windows x86_64) | Each platform's wheels and CLI, run on that platform. See [Smoke tests](#smoke-tests). |
| Publish plan (dry run) | Dry runs only, read only: `release_api.py plan` and `publish_crates.sh --dry-run`. See [Publish plan](#publish-plan-dry-run). |
| PyPI trusted publishing | Releases only, in the `release` environment, as soon as Validate passes: exchanges the job's OIDC token for a PyPI upload token, which proves PyPI's trusted publisher matches this workflow. The minted token is masked at once, never printed and discarded. See [Secrets and environment](#secrets-and-environment). |
| GitHub draft | `release_api.py stage`: makes the tag's GitHub draft hold exactly the `release-assets` files and their `SHA256SUMS`. See [Draft, then publish](#draft-then-publish). |
| Publish to crates.io | `publish_crates.sh`: one multi-package `cargo publish --no-verify` of the crates whose version isn't on crates.io yet. Cargo orders them and waits for the index. |
| Publish to PyPI | `pypa/gh-action-pypi-publish` with trusted publishing: the wheels and the sdist. |
| GitHub release | `release_api.py publish`: publishes the staged draft, unchanged. |

**Check the artifacts.** `check_artifacts.py` checks that there is one wheel
per Python and platform with the right tags, version (the workspace version
in PEP 440 form) and extension module, and `THIRD-PARTY-NOTICES` in each wheel
and the sdist. For every binary it checks the architecture and linkage: ELF
with nothing above glibc 2.17 (`objdump -T`) and no dynamic libpcap; Mach-O
with the tag's minimum macOS, a code signature on arm64, only the expected
libraries, and only Python's C API left to a flat lookup (`llvm-objdump`);
PE with only the expected DLLs (`llvm-readobj`). Output that parses to
nothing fails (no bind table, no Python lookups in an extension module, no
`kernel32.dll` import). `llvm-objdump` and `llvm-readobj` come from the
pinned toolchain's `llvm-tools`.

**Order.** The publish jobs run only for a tag, and only after Validate, the
CI gate, the artifact check and every smoke test passed. They run one at a
time in the order above, so a failure stops the jobs after it. The GitHub
draft comes first because it is the one reversible step: it shows that GitHub
takes the release before crates.io and PyPI, which can't be undone. crates.io
also waits for the PyPI trusted publishing check, so a PyPI publisher that
doesn't match stops the release before anything is published. The draft
is published last, so the release page and `releases/latest` show the
release only once its crates and wheels are out.

Release builds use no Rust cache: every build starts from the pinned
toolchain and `Cargo.lock`, and nothing a PR's run saved can reach a tag's
build.

#### Runners

| Artifacts | Runner | Toolchain |
| --- | --- | --- |
| Linux x86_64 wheels and `bacnet-linux-amd64` | `ubuntu-24.04` | the manylinux2014 image: CentOS 7, glibc 2.17, GCC 10 |
| Linux aarch64 wheels and `bacnet-linux-arm64` | `ubuntu-24.04-arm` | the manylinux2014 image for aarch64 |
| macOS x86_64 wheels and `bacnet-macos-amd64` | `macos-26-intel` | Xcode's clang and linker |
| macOS arm64 wheels and `bacnet-macos-arm64` | `macos-26` | Xcode's clang and linker |
| Windows x86_64 wheels and `bacnet-windows-amd64.exe` | `windows-2025` | MSVC, with the image's NASM for AWS-LC |

Nothing is cross-compiled or emulated. The labels name an image version, not
`-latest`, so a move to a newer image is a reviewed change. `macos-26-intel`
is GitHub's newest Intel macOS image (`macos-15-intel` is the other one
left); if GitHub retires its Intel images, the macOS x86_64 build can move to
`macos-26` with `--target x86_64-apple-darwin`, which Xcode builds natively,
but its smoke test would then have no runner.

#### What is and isn't tested

Every file gets the static checks above, and the smoke tests run every wheel
and CLI on its own platform: Linux on both architectures, on Ubuntu 24.04 and
in CentOS 7 (glibc 2.17), macOS on Apple Silicon and Intel, and Windows. The
Python suite runs against the installed cp312 wheel on both Linux
architectures; on macOS and Windows, native tests run it against each PR's
own build. These tests show that the files load and work on those hosts.
They say nothing about hardware, or about older macOS and Windows versions
than the runners'.

### CI gate

The release needs CI to have passed on its commit, read from GitHub's API by
[`scripts/release/ci_gate.py`](../scripts/release/ci_gate.py): the commit's
workflow runs (`GET /repos/{o}/{r}/actions/runs?head_sha=<sha>`), and for each
run of `ci.yml` and `native-tests.yml` the check runs of its check suite
(`GET /repos/{o}/{r}/check-suites/{id}/check-runs?filter=latest`). The job
token needs `actions: read` and `checks: read`.

- **CI.** On a release, the `ci.yml` run that the tag's push started (event
  `push`, `head_branch` the tag, `head_sha` the commit; the newest if it was
  re-run) must have succeeded: its `CI OK`, `MSRV (Linux native)` and `Cargo
  Audit + Deny` check runs all `success`. CI runs those two heavy jobs on
  tags, so that run checks the commit with that day's advisory databases, and
  no other run, however heavy, stands in for it. If it skipped them, the gate
  fails at once.

  A dry run has no tag, so there the gate takes the newest `ci.yml` run on the
  commit that ran MSRV and audit-deny. CI runs them on tags, pushes to `main`,
  PRs to `main`, the weekly run and manual runs, and skips them on PRs to
  `dev` and pushes to `dev`, whose `CI OK` passes without them. A commit can
  therefore carry several `CI OK` check runs: a `dev`→`main` PR's head has
  `dev`'s lean push run and the PR's heavy run. Only a run that ran the heavy
  jobs counts, so a lean `CI OK` never stands in for MSRV, audit and deny. A
  run that skipped both, or finished without either, is lean.
- **Native.** The newest `native-tests.yml` run on the commit must have
  `Native OK` `success`. Native tests don't run on tags, so the tagged commit
  must have come through a push to `dev` or `main`, which runs them.

Only this repository's runs count, not a fork's pull request runs, and only
check runs created by GitHub Actions. Within a run the latest check run of
each name counts, so a re-run of failed jobs replaces the attempt it re-ran.
Of several runs, the newest (by the start of its latest attempt) counts, so
an older success can't hide a newer failure. `test_ci_gate.py` reads both
workflow files and fails if a job name the gate relies on, or `ci.yml`'s
trigger on `v*` tags, is gone, so a rename shows in the PR that makes it, not
after a tag's 60-minute wait.

On a tag, the gate polls every 30 seconds for up to 60 minutes while the run
that counts is still going or doesn't exist yet. It fails at once when a
named check run finished other than `success`, or is missing from a finished
run, and its message says to re-run that run's failed jobs if the failure was
transient. At the deadline it fails with what it saw. A dry run checks once
and only warns. The gate runs alongside the builds, and every publish job
needs it.

### Smoke tests

Each Smoke test job downloads its platform's `wheels-<platform>` and
`cli-<platform>` artifacts, the same files `release-assets` holds, and sets up
CPython 3.11 to 3.14 with `actions/setup-python`. The jobs are independent:
one failing doesn't cancel the others.

- **Wheels.** Each wheel, installed with `pip --no-index --no-deps` into a
  fresh venv of its own CPython, runs
  [`wheel_smoke.py`](../scripts/release/wheel_smoke.py): the installed version
  is the release's; `list_serial_ports()` returns a list of port names, which
  on macOS goes through IOKit and CoreFoundation, both of which must then be
  loaded, and on Windows through SetupAPI; and a loopback round trip with the
  public API, where a `BACnetServer` on 127.0.0.1 serves an analog input and
  an analog value and a `BACnetClient` reads the input's present value, reads
  it and its name with ReadPropertyMultiple, writes the value's present value
  at priority 8 and reads it back.
- **CLI.** [`cli_smoke.sh`](../scripts/release/cli_smoke.sh): `--version` must
  print `bacnet <version>`, `--help` runs, and the README quickstart `read` and
  `--json readm` run against the README server on the cp312 wheel. On Linux,
  `capture --read` also decodes a one-packet pcap file, through the static
  libpcap and its filter compiler, without needing capture privileges. The
  macOS and Windows builds have no packet capture.
- **Python suite** (Linux): `python -m unittest discover -s
  crates/rusty-bacnet/tests` against the installed cp312 wheel.
- **glibc 2.17** (Linux):
  [`smoke_glibc217.sh`](../scripts/release/smoke_glibc217.sh) runs the cp312
  wheel's `wheel_smoke.py` and the CLI's `cli_smoke.sh`, capture included, in
  the manylinux2014 image the artifacts were built in, whose CentOS 7 has
  glibc 2.17: the oldest system the Linux artifacts support.

**Reading a failed smoke test.** Each job is one platform. In **Wheels, each
CPython**, each wheel's output is a log group named after the wheel, and the
last one shows which wheel and which check failed: the install, the import,
the version, the serial port listing or the round trip. On macOS, an abort
with "symbol not found" or "Library not loaded" points at the links that the
artifact check lists. **CLI** shows `cli_smoke.sh`'s output, with the Python
server's log if it didn't start. A long wait for a runner, most often the
Intel macOS one, shows as a queued job, not a failure.

A failed smoke test publishes nothing. A bug in an artifact needs a new
commit, released as a new version.

### Draft, then publish

GitHub releases in this repository are immutable once published: assets can't
be added, replaced or deleted, and the tag can't be used again.
[`release_api.py`](../scripts/release/release_api.py) therefore builds each
release as a draft and publishes it last. The GitHub draft job runs `stage`:

1. Check that the tag exists and points at the release commit.
2. `GET /repos/{o}/{r}/releases` (all pages), matching `tag_name`, because
   `/releases/tags/{tag}` doesn't return drafts. GitHub lists drafts only to a
   token that can write, so the job has `contents: write`. A published
   release is only checked, read only.
3. `POST /repos/{o}/{r}/releases` with `draft: true`, `target_commitish:
   <commit>` and the notes, unless a draft exists. A pre-release version
   (`-` in the tag) makes a pre-release.
4. Make the draft hold exactly the `release-assets` files: delete what an
   interrupted upload left (any asset not in state `uploaded`), every asset
   that isn't one of the files, every copy of a name that appears twice, and
   every asset whose digest differs from this run's file; then
   `POST uploads.github.com/.../releases/{id}/assets?name=...` for each file
   the draft lacks.
5. `SHA256SUMS` for those files, uploaded the same way; an outdated one is
   deleted first.
6. The final check (below), on a fresh `GET` of the draft.

It writes the draft's id and the sha256 of its `SHA256SUMS` as job outputs.
After crates.io and PyPI, the GitHub release job runs `publish` with both: the
tag's release must be that draft, this run's files must give the same
`SHA256SUMS`, and the draft must pass the final check again. Then
`PATCH /repos/{o}/{r}/releases/{id}` with `draft: false` and `make_latest:
"legacy"` is the only write. `legacy` has GitHub pick the latest release by
date and version, so a backport published after a newer release doesn't
become the latest. A publish without both staged values, or with an empty
one, publishes nothing: the step checks, and so does `release_api.py`.

- **Final check.** The last step before the irreversible publish. The raw
  asset list, before any filtering by state, must hold exactly the expected
  names, once each, with no entry other than `uploaded`. Each asset's size
  must match, and its reported `digest` (`sha256:<hex>`) must equal the
  sha256 of this run's file (for `SHA256SUMS`, of the text this run
  computed). An asset without a digest is downloaded and hashed.
- **Resuming.** A draft left by an earlier attempt keeps its notes, but only
  if its `target_commitish` is the release commit: a draft made for another
  commit stops the run with a message to delete it. Assets it already holds
  with this run's bytes aren't sent again.
- **Published.** The script never uploads to or deletes from a published
  release. It checks that every asset and `SHA256SUMS` are there and that each
  asset matches `SHA256SUMS`, and fails with an explanation otherwise.
- **Retries.** Reads, the final `PATCH` and deletes retry on 5xx and network
  errors, including a truncated response (`http.client.HTTPException`, such
  as `IncompleteRead`). Deletes treat 404 as done, so a retried delete
  succeeds. A `POST` doesn't retry: after an uncertain upload failure, or
  GitHub's 422 `already_exists`, the script lists the draft's assets again
  and accepts the asset if it's complete and its digest matches, or deletes
  it and sends the file again. A failed create looks for the draft before
  trying again.
- **Downloads.** GitHub serves a draft's assets only through the API asset URL
  with `Accept: application/octet-stream`. The token goes in an unredirected
  header, so the redirect to storage never carries it.

#### Publish plan (dry run)

The Publish plan job runs `release_api.py plan` for the tag `v<version>`,
which makes no write: it checks the tag, finds the release, checks a
published one and downloads its smallest asset, or prints the deletions and
uploads `stage` would make (or warns that a draft for another commit would
stop it). Its token can only read, so it doesn't see drafts. While the
workspace version is already released, it reports what the published release
lacks as a warning. `publish_crates.sh --dry-run` then lists the crates whose
version isn't on crates.io yet.

### Artifacts

- `release-assets`: what the publish jobs upload, which the GitHub release
  also gets with a `SHA256SUMS` file:
  - `bacnet-linux-amd64` and `bacnet-linux-arm64`, with BACnet/SC and packet
    capture;
  - `bacnet-macos-amd64`, `bacnet-macos-arm64` and `bacnet-windows-amd64.exe`,
    with BACnet/SC (no packet capture, as in 0.11.0);
  - `rusty_bacnet-<version>.tar.gz`, the sdist;
  - twenty wheels, `rusty_bacnet-<version>-cp3XY-cp3XY-<platform>.whl` for
    CPython 3.11 to 3.14 on five platforms:
    `manylinux_2_17_x86_64.manylinux2014_x86_64`,
    `manylinux_2_17_aarch64.manylinux2014_aarch64`, `macosx_10_12_x86_64`,
    `macosx_11_0_arm64` and `win_amd64`;
  - `THIRD-PARTY-NOTICES`.

  That is 0.11.0's asset names (the five CLI binaries on GitHub, the wheels
  and sdist on PyPI) plus the CPython 3.14 wheels and `THIRD-PARTY-NOTICES`.
- `release-notes`: `notes.md`. Issue references such as `#1134` link to the
  GitHub issue of that number, which is the right one since the issues moved
  to GitHub (#1472). GitHub refuses bodies over 125,000 characters, so a
  longer section is cut at 120,000 with a link to the full `CHANGELOG.md`,
  closing any code block the cut leaves open.
- `notices`: `THIRD-PARTY-NOTICES`, which the sdist and wheel builds put in.
- `crates`, `sdist`, `wheels-<platform>` and `cli-<platform>`: each build
  job's output.

The repository keeps a run's artifacts for 90 days.

The Linux wheels and binaries need glibc 2.17 or newer, which covers
RHEL/CentOS 7, Debian 8, Ubuntu 14.04 and later: they are built on glibc 2.17
itself. The CLI links libpcap statically. Debian and Ubuntu name the shared
library `libpcap.so.0.8` and RHEL `libpcap.so.1`, so no single dynamically
linked binary could run on both.

### Builds

The pins the builds read beyond `rust-toolchain.toml` are in
[`.github/ci-pins.env`](../.github/ci-pins.env): maturin's version, the
manylinux2014 images' digests, rustup-init's version and digests, and
libpcap's version and digest. To move to a newer image, take a dated tag's
`manifest_digest` for each architecture from
`https://quay.io/api/v1/repository/pypa/manylinux2014_<arch>/tag/?specificTag=<tag>`;
the PR's dry run then builds and tests with it.

**Linux** ([`build_linux.sh`](../scripts/release/build_linux.sh)). On each
architecture's own runner, the script runs itself in the
`quay.io/pypa/manylinux2014_<arch>` image, pinned by digest, with the checkout
mounted. In the image it:

- installs the toolchain that `rust-toolchain.toml` pins, through a
  rustup-init checked against its pinned SHA-256;
- builds libpcap from the pinned tarball, checked against its SHA-256
  ([`libpcap.sh`](../scripts/release/libpcap.sh)), as a static,
  position-independent archive (the CLI is a position-independent
  executable). flex and bison, from the image's package manager, only
  generate its filter parser. The pcap crate links `-lpcap`, and with only
  `libpcap.a` in `LIBPCAP_LIBDIR` the linker takes the archive. `LIBPCAP_VER`
  tells the crate's build script the version, which it would otherwise load a
  shared libpcap to ask;
- builds the wheels with the pinned maturin (see below):
  `--compatibility manylinux2014` tags them `manylinux_2_17`, and
  `--auditwheel check` fails the build on a symbol or library outside that
  policy instead of copying a library into the wheel;
- builds the CLI with `--features sc-tls,pcap,serial`. MS/TP (`serial`) is in
  the Linux CLI only, until a USB RS-485 adapter has been tested on macOS and
  Windows (#969).

AWS-LC (`aws-lc-sys`, for BACnet/SC) builds with the image's GCC 10 through
its default builder, which needs no CMake. Its build checks the compiler for
GCC's `memcmp` bug, which GCC 10.2 doesn't have.

**macOS and Windows** ([`build_native.sh`](../scripts/release/build_native.sh)).
The job installs the pinned toolchain and the four CPythons, and the script
builds the wheels with the pinned maturin, in a venv, then the CLI with
`--features sc-tls`.

Both scripts install maturin with `pip install --require-hashes --only-binary
:all: -r` [`maturin-requirements.txt`](../scripts/release/maturin-requirements.txt),
which names its version and the sha256 of each wheel the five build hosts can
pick (manylinux x86_64 and aarch64, macOS x86_64 and universal2, Windows
x86_64), from `https://pypi.org/pypi/maturin/<version>/json`.
`test_maturin_requirements.py` fails if that version differs from
`MATURIN_VERSION` in `.github/ci-pins.env`; bump them together.

- **Minimum macOS.** 10.12 on x86_64 and 11.0 on arm64, as in 0.11.0's wheel
  tags (`macosx_10_12_x86_64`, `macosx_11_0_arm64`) and Rust's defaults.
  `MACOSX_DEPLOYMENT_TARGET` sets it for Rust and the C code, and maturin
  takes the wheel tag from it. The x86_64 binaries carry
  `LC_VERSION_MIN_MACOSX` 10.12 and the arm64 ones `LC_BUILD_VERSION` with
  `minos 11.0`; the artifact check compares each with its tag.
- **Frameworks.** The CLI links no framework: BACnet/SC runs its own TLS
  handshake against its configured trust anchors and never uses the system's
  root certificates. The extension module links IOKit and CoreFoundation,
  for `serialport`'s port listing (MS/TP). Apple's linker binds their symbols
  to the frameworks; only Python's C API is left to a flat lookup at load
  time, since maturin links extension modules with `-undefined
  dynamic_lookup`. An arm64 file must carry a code signature
  (`LC_CODE_SIGNATURE`), which macOS requires on Apple Silicon; Apple's
  linker signs ad hoc.
- **Windows C runtime.** The CLI links it statically
  (`-C target-feature=+crt-static`), so it imports only Windows system DLLs
  and needs no Visual C++ Redistributable, which 0.11.0's did
  (`VCRUNTIME140.dll`). The wheels link it dynamically, like other extension
  modules: Python for Windows ships `VCRUNTIME140.dll`, and the Universal CRT
  (`api-ms-win-crt-*`) is part of Windows 10 and later. The artifact check
  enforces both.
- **Windows linking.** Both link with `-DEBUG:NONE` and `-Brepro` (link.exe's
  `/DEBUG:NONE` and `/Brepro`, spelled with `-` so that Git Bash doesn't take
  them for paths and convert them): the release ships no PDB, whose path and
  build ID change with every build, and `-Brepro` puts a hash of the output
  where the timestamps go. PyO3 links
  each extension module to its `pythonXY.dll` with raw-dylib, so no import
  library is needed; the artifact check makes sure each `.pyd` imports its
  own Python's DLL and no other. AWS-LC assembles its x86_64 assembly with the
  runner image's NASM.

`SOURCE_DATE_EPOCH`, the commit time, pins the sdist's and the wheels'
timestamps.

#### Release profile

Both build scripts build the wheels and the CLI with `--profile dist`
(#973), which the root `Cargo.toml` defines as `release` plus
whole-program LTO (`lto = "fat"`) and one codegen unit. It keeps the
symbol table (`strip = "debuginfo"`, which is also what `release` does by
default), so a panic run with `RUST_BACKTRACE=1` still names its
functions; with `strip = "symbols"` the backtrace comes out empty and only
the panic's file and line remain. `release` itself is unchanged, so
`cargo build --release`, the benchmark suites (the `bench` profile inherits
`release`) and `maturin develop --release` build as fast as before and the
numbers in `Benchmarks.md` still hold. `dist` lands in `target/<triple>/dist/`.

Measured on an Apple Silicon Mac (10 cores) with the pinned toolchain,
for the CLI with the release workflow's macOS features (`--features
sc-tls`, plus the default `tui`) and for one CPython wheel. A clean build is
from an empty profile directory, dependencies included; a rebuild is after
touching `crates/bacnet-cli/src/main.rs`. Another build shared the machine,
so build times vary by about a third between runs; sizes are exact. MB is
10⁶ bytes.

| Artifact | Profile | Size | Clean build | Rebuild |
| --- | --- | --- | --- | --- |
| CLI, macOS arm64 | `release` (before #973) | 18.9 MB | 48–60 s | 21–24 s |
| | `release` + `strip = true` | 15.5 MB (−18%) | 50–77 s | 24–27 s |
| | `release` + `strip = true` + `lto = "thin"` | 15.8 MB (−16%) | 46–58 s | 23–24 s |
| | `release` + `lto = "thin"` | 19.1 MB (+1%) | 47 s | 22 s |
| | **`dist`** (fat LTO, one codegen unit) | **12.4 MB (−34%)** | 114 s | 64 s |
| | `dist` + `strip = true` | 10.6 MB (−44%) | 87 s | 62 s |
| CLI, macOS x86_64 (built on arm64) | `release` (before #973) | 20.6 MB | 45 s | 20 s |
| | `release` + `strip = true` | 17.2 MB (−17%) | 71 s | 25 s |
| | `release` + `strip = true` + `lto = "thin"` | 17.5 MB (−15%) | 53 s | 29 s |
| | **`dist`** | **14.5 MB (−30%)** | 83 s | 63 s |
| Wheel, macOS arm64 (`.whl`; its `.so`) | `release` (before #973) | 10.4 MB; 34.1 MB | 69 s | |
| | **`dist`** | **7.4 MB (−29%); 20.0 MB** | 149 s | |
| | `dist` + `strip = true` | 6.8 MB; 15.7 MB | 142 s | |

Thin LTO doesn't pay here: cargo's default for `release` already runs
thin LTO within each crate, and across crates it made the binary slightly
larger. Fat LTO with one codegen unit is what shrinks it, at about twice the
build time. On the release runners that is a few minutes more per build job,
well inside its 60-minute timeout. Each job's `ls -l dist out` prints the
sizes for Linux and Windows.

- **Code signature.** Apple's linker signs the arm64 binaries ad hoc after
  LTO, and `strip`, if used, re-signs them: every arm64 binary above passed
  `codesign -v`. The x86_64 ones carry no signature, which macOS doesn't
  require there and the artifact check doesn't ask for.
- **The TUI's share.** With `release`, the arm64 CLI was 13.4 MB on dev just
  before #974 made `tui` a default feature (09c332ce7) and 18.9 MB after.
  On today's dev, `--no-default-features` gives 12.7 MB under `release`
  and 8.9 MB under `dist`, so `tui` adds 6.2 MB and 3.5 MB respectively.

### Third-party notices

`scripts/release/third_party_notices.py` writes `THIRD-PARTY-NOTICES`: Rusty
BACnet's own licence, then every third-party component in the release
binaries with the licence files it ships, identical texts printed once.

- The crates come from `cargo tree --locked --offline -e normal,no-proc-macro`
  for the CLI (`-p bacnet-cli --features sc-tls,pcap,serial` on Linux, `--features
  sc-tls` on macOS and Windows) and the Python extension (`-p rusty-bacnet`) on
  each of the five release targets, so build scripts, proc-macros,
  dev-dependencies and crates for other platforms, which no release binary
  contains, are left out. Platform crates such as `windows-sys` and
  `io-kit-sys` are in because a release binary contains them. The licence
  files are the ones at each crate's root, plus three for the C library that
  `aws-lc-sys` bundles: `aws-lc/LICENSE`, fiat-crypto's
  `aws-lc/third_party/fiat/LICENSE` (MIT), and the licence comment of
  jitterentropy's `jitterentropy.h`, which is built on Linux and Windows and
  whose BSD-3-Clause terms AWS-LC elects (the crate doesn't ship
  jitterentropy's `LICENSE`).
- Every component's row gives where its source is: a crate's crates.io page
  for that version (`https://crates.io/crates/{name}/{version}`), or its
  repository if it isn't from crates.io; libpcap's release tarball on
  tcpdump.org. MPL-2.0 needs this for `serialport`, which is in the wheels.
- Generation fails if a crate ships no licence file while its licence
  expression has any identifier other than `0BSD`, `BSL-1.0`, `CC0-1.0`,
  `MIT-0`, `Unlicense` and `WTFPL`, whose terms don't ask for the notice in a
  binary (so MIT, BSD-*, ISC, Apache-2.0, MPL-2.0 and unknown ones all count),
  unless `ALLOW_NO_LICENSE_FILE` in the script names it with the reason. The
  list is empty: every such crate ships a licence file. A crate under one of
  those exempt licences that ships none would be listed at the end with its
  reason; none does since the shell and rustyline went (#974), which took
  `clipboard-win` (BSL-1.0) with them.
- libpcap's licence and version come from the pinned tarball, which Validate
  fetches with `libpcap.sh`, as the Linux builds do.
- The file depends only on `Cargo.lock`, the crate sources and libpcap, so a
  rebuild writes the same file.

It's attached to each release, and `pyproject.toml`'s `license-files` puts it
in each wheel's `.dist-info/licenses/` and in the sdist; the artifact check
checks both. Local builds have no such file, and maturin skips it.

cargo-audit and cargo-deny don't cover libpcap, so its advisories need
tracking by hand: watch the [tcpdump/libpcap
releases](https://www.tcpdump.org/) and their security fixes, and bump
`LIBPCAP_VERSION` and `LIBPCAP_SHA256` in `.github/ci-pins.env` together.

### Re-running a partial release

Every publish job is safe to run again:

- **GitHub draft** resumes the tag's draft and leaves it holding exactly this
  run's files; on a published release it only checks.
- **crates.io**: `publish_crates.sh` publishes only the crate versions that
  aren't on crates.io yet.
- **PyPI trusted publishing** only mints and discards a token, so it can run
  any number of times.
- **PyPI** uploads without `skip-existing`. PyPI takes a file it already has
  with the same bytes as a no-op, and refuses a file of the same name with
  other bytes, so a re-run finishes a partial upload, and a rebuilt wheel that
  differs from the one on PyPI stops the release there.
- **GitHub release** publishes the staged draft, or only checks a release that
  is already published.

To finish a release after a failure, for example a network error, use
**"Re-run failed jobs"** on the tag's run. GitHub keeps the run's artifacts
and the outputs of the jobs that passed, so nothing is rebuilt: the jobs that
failed, and the ones after them, run again with the same files, and the
publish jobs pick up where they stopped. The CI gate runs again only if it
failed. The artifacts expire after 90 days.

"Re-run all jobs", or a `dry_run=false` dispatch on the tag, rebuilds every
artifact. Hosted runner images change every week or so, so a rebuild can give
other bytes than the first build. Before anything reached PyPI that is
harmless: the draft then takes the rebuilt files. Once PyPI has some of the
files, it refuses rebuilt ones that differ, and the release stops there with
the GitHub release still a draft. Finishing it then needs a new version.

### Secrets and environment

The release uses no repository secret. Each job's `GITHUB_TOKEN` has only the
permissions it declares (the workflow's default is none):

| Job | Token permissions | Also |
| --- | --- | --- |
| Validate, Crates and sdist, Build, Check the artifacts, Smoke test, Publish plan | `contents: read` | |
| CI gate | `actions: read`, `checks: read`, `contents: read` | |
| GitHub draft, GitHub release | `contents: write` | |
| Publish to crates.io | `contents: read` | the `release` environment's `CARGO_REGISTRY_TOKEN` |
| PyPI trusted publishing, Publish to PyPI | `id-token: write` only | the `release` environment, for trusted publishing |

- **The `release` environment.** Only tags matching `v*` may deploy to it (a
  custom deployment policy since 2026-10-04; before, only protected branches
  could). It holds `CARGO_REGISTRY_TOKEN`. Only the two jobs above deploy to
  it, so a dry run, whatever started it, gets no secret.
- **`CARGO_REGISTRY_TOKEN`**: a crates.io token that can publish new crates
  and update existing ones. 0.13.0 is the first release of `bacnet-time`,
  `bacnet-mstp-codec`, `bacnet-bip-codec`, `bacnet-mstp-core`,
  `bacnet-bip-core` and `bacnet-router-core`, which a token scoped to
  existing crates couldn't publish, as 0.12.0 was for `bacnet-endpoint` and
  `bacnet-cli`.
  crates.io trusted publishing may replace it later.
- **PyPI trusted publishing.** PyPI accepts the PyPI job's OIDC token for
  `rusty-bacnet` when the project's trusted publisher names owner
  `jscott3201`, repository `rusty-bacnet`, workflow `release.yml` and
  environment `release`. No PyPI token is stored anywhere. 0.11.0 was
  published from `ci.yml` (environment `release`), so PyPI's publisher still
  names `ci.yml`, whose release jobs moved out in #943. Before the first
  release, the owner adds the `release.yml` publisher on pypi.org and removes
  the `ci.yml` one (step 0 of [To release](#trigger-and-dry-run)). The PyPI
  trusted publishing job checks this before crates.io: it requests the job's
  OIDC token for PyPI's audience (`https://pypi.org/_/oidc/audience`),
  exchanges it at `https://pypi.org/_/oidc/mint-token`, masks the minted
  token with `::add-mask::` at once, never prints it and discards it (it
  expires after 15 minutes). If PyPI refuses, the job prints PyPI's reason and
  fails, and nothing is published.
- **Fork PRs** run the dry run with a read-only token and no secret, as GitHub
  gives every fork's `pull_request` run.
- The actions are pinned to full commit SHAs, as the repository requires, and
  no step expands an input or event field into its script: they reach the
  scripts as environment variables.

### macOS and Windows

The macOS and Windows artifacts are built on GitHub's macOS and Windows
runners and smoke-tested there before a release publishes anything. These
checks do not establish hardware qualification, support for macOS or Windows
versions older than the runners', or release readiness.
