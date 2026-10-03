# CI and merge evidence

Forgejo is the primary forge. GitHub is its push mirror (sync on commit), so
every branch and tag pushed to Forgejo reaches GitHub within a minute or so.
Heavy work runs on Forgejo's fixed-cost runner VM; GitHub runs only what needs
its native runners or its services.

| Host | Runs |
| --- | --- |
| Forgejo (self-hosted Linux runner) | Linux CI, [`.forgejo/workflows/ci.yml`](../.forgejo/workflows/ci.yml); website validation, [`docs.yml`](../.forgejo/workflows/docs.yml); releases and publishing to crates.io, PyPI and Forgejo, [`release.yml`](../.forgejo/workflows/release.yml) |
| GitHub (hosted runners) | [Native macOS and Windows tests](#native-tests-github), [`.github/workflows/native-tests.yml`](../.github/workflows/native-tests.yml); the [release smoke test](#smoke-test) of the macOS and Windows artifacts, [`release-smoke.yml`](../.github/workflows/release-smoke.yml), which Forgejo's release workflow dispatches; the GitHub Pages docs deploy, [`docs-pages.yml`](../.github/workflows/docs-pages.yml); the GitHub release copy, which Forgejo's release workflow makes through GitHub's API |

| Platform | Where it is checked |
| --- | --- |
| Linux amd64 | Forgejo CI: lint, clippy, rustdoc, tests, Python bindings, MSRV, audit and deny |
| macOS arm64 | GitHub, native tests: tests, doctests, clippy, rustdoc, Python bindings. Forgejo CI: clippy and rustdoc for each published crate with default features, cross-checked |
| Windows x86_64 (MSVC) | GitHub, native tests: tests, doctests, clippy, rustdoc, Python bindings. Forgejo CI: clippy and rustdoc for each published crate with default features, cross-checked |

A PR merges only when both are green on its head SHA: `CI OK` on Forgejo and
both jobs of the native tests on GitHub (see [Merge evidence](#merge-evidence)).

## Pipeline

| Job | PR to `dev` | PR to `main` | Push to `dev` (merge) | Push to `main`, `v*` tag, weekly, manual |
| --- | --- | --- | --- | --- |
| CI image: build and push the job image if its tag is missing | ✓ | ✓ | ✓ | ✓ |
| Lint: rustfmt, 700-LOC cap, no-secret scan, script regressions, changelog fragments | ✓ | ✓ | ✓ | ✓ |
| Clippy and rustdoc, warnings denied: every feature, PyO3 crate, `bacnet-cli` without default features, each published crate with default features for Linux, Windows and macOS; the every-feature and PyO3 rustdoc runs include private items | ✓ | ✓ | ✓ | ✓ |
| Test: Linux, every feature (`LINUX_FEATURES`) | ✓ | ✓ | ✓ | ✓ |
| Python bindings: `maturin develop` (maturin 1.15.0), then `python -m unittest discover -s crates/rusty-bacnet/tests` and the crate's Rust tests (`cargo nextest run -p rusty-bacnet`) | ✓ | ✓ | ✓ | ✓ |
| MSRV 1.93, Linux native (`check-msrv.sh --linux-native`) |  | ✓ |  | ✓ |
| Cargo Audit + Cargo Deny |  | ✓ |  | ✓ |
| **CI OK**: fails if any job above failed | ✓ | ✓ | ✓ | ✓ |

`CI OK` is the single status to require in branch protection (its context is
`CI / CI OK (pull_request)`); jobs skipped by tier count as passing. [`.forgejo/workflows/docs.yml`](../.forgejo/workflows/docs.yml)
validates the website (Astro checks, unit tests, production build and Chromium
tests) on PRs that change `website/**`.

Merge pushes to `dev` run the Lean jobs, for two reasons (#904).

- **Caches.** The runner scopes cache writes from `pull_request` events to that
  PR. A PR's first run falls back only to caches from non-PR events: merges,
  `main`, tags, the schedule or manual runs. Before this, those were rare, so
  most new PRs started cold.
- **Merge result.** A PR run checks out the PR head, not the merge. With the
  repo's default merge commits, the `dev` run is the only test of the combined
  code, and an outdated branch can still merge.

A newer merge cancels the previous merge's run, since it tests a superset.
**After merging, check the `dev` run.** It isn't a required status, so a red
merge run is the only signal of merge skew; fix it forward on `dev` right away.
The weekly scheduled run checks the default branch (`dev`) with the Heavy jobs
too.

Rust caches are keyed per job on the toolchain, `Cargo.lock`, the manifests,
and, for Clippy and Test, `LINUX_FEATURES` (Clippy also on
`DEFAULT_FEATURES_TARGETS`). They're saved even when a job fails.
A new push to a PR cancels its superseded run.

Tests run with [cargo-nextest](https://nexte.st), which gives each test its
own process. Its settings live in [`.config/nextest.toml`](../.config/nextest.toml);
CI uses the `ci` profile, and nextest does not run doctests, so a separate
`cargo test --doc` step covers them. The Linux test commands are below, with
`$LINUX_FEATURES` as set in `ci.yml`: every optional feature that builds on
Linux, including per-crate ones such as `bacnet-endpoint/sc-tls` and
`bacnet-cli/pcap`. The last steps run the `bacnet-cli` tests with default
features, because a few exist only when `sc-tls` or `pcap` is off, and with no
default features, because a few exist only when the default `tui` feature is
off.

```bash
cargo nextest run --workspace --exclude rusty-bacnet --locked --features "$LINUX_FEATURES" --profile ci
cargo test --doc --workspace --exclude rusty-bacnet --locked --features "$LINUX_FEATURES"
cargo nextest run -p bacnet-cli --locked --profile ci
cargo nextest run -p bacnet-cli --no-default-features --locked --profile ci
```

The Python job builds the PyO3 extension in debug mode into a fresh venv with
the CI image's Python (3.12) and runs the unittest suite. The SC tests generate
certificates with the `openssl` CLI. Cargo Deny covers the bindings'
dependencies too; only `bacnet-benchmarks` is excluded.

The same job then runs the crate's Rust tests, its lib unit tests and
integration tests, with `cargo nextest run -p rusty-bacnet --locked --profile ci`.
The workspace doesn't turn on pyo3's `extension-module` feature, so these test
binaries link libpython (#919); maturin turns the feature on for wheel and
`maturin develop` builds from `crates/rusty-bacnet/pyproject.toml`. Linking
needs the interpreter's shared library and its unversioned `.so` symlink, which
the CI image gets from `libpython3-dev`. The step sets `PYO3_PYTHON=/usr/bin/python3`
so it links the apt Python that package matches, whatever else is on `PATH`,
and without depending on the venv from the maturin step. Tests that call into
Python start the interpreter with `Python::initialize()` first, since nothing
enables pyo3's `auto-initialize`.

Use cargo-nextest 0.9.145 or later locally. Older releases on macOS could
mark unrelated passing tests as leaky (#751), and the configuration warns
about them.

### Runner

Jobs run on a self-hosted Linux VM (8 vCPU, 32 GB) that is preemptible. It
runs two runner daemons, set up in the infrastructure repository (forgejo-dev):

- **Main runner:** up to three jobs at once, every build and test job.
- **Gate runner:** label `gate`, host mode, up to two jobs at once, this
  repository only. It runs the seconds-long CI image and CI OK jobs, so a run
  no longer waits for one of the three slots before its builds start and again
  after they finish. When the CI image job has to build a new image, that build
  runs alongside the main runner's three jobs.

While no runner with the `gate` label is online, every run stops at CI image,
and its `CI OK` status stays pending. The gate runner is listed under this
repository's Settings → Actions → Runners.

`Swatinem/rust-cache` keeps Cargo state in the runner's cache. The cache lives
on the VM, so a preemption starts the next run cold, and a preempted job must be
re-run.

### CI image

Every job except CI OK runs in one prebuilt image,
`forgejo.taile9ca5.ts.net/jscott3201/rusty-bacnet-ci:<tag>`, built from
[`.forgejo/ci-image/Dockerfile`](../.forgejo/ci-image/Dockerfile) (#904). It
contains:

- the runner's default `ghcr.io/catthehacker/ubuntu:act-24.04`, pinned by
  digest;
- Rust 1.99.0 with rustfmt and clippy, and the 1.93 MSRV toolchain;
- cargo-nextest, cargo-audit, cargo-deny and maturin at pinned versions, each
  download checked against its SHA-256;
- the apt packages the jobs need;
- for the [release](#release): zig and cargo-zigbuild, the
  `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`
  and `x86_64-pc-windows-msvc` Rust targets, a static libpcap for each Linux
  release target with its licence, cargo-xwin with Microsoft's CRT and Windows
  SDK in `/opt/xwin`, clang (for clang-cl) and nasm, Rust's `llvm-tools`
  (llvm-ar for cargo-xwin, llvm-objdump and llvm-readobj for the artifact
  test), uv for the artifact test's extra Pythons, and `qemu-aarch64-static`
  with the aarch64 glibc to run the arm64 CLI. See
  [macOS and Windows builds](#macos-and-windows-builds). The Clippy job's
  per-crate default-features check uses the Windows and `aarch64-apple-darwin`
  targets too.

The jobs no longer spend time on apt, rustup or tool downloads.

The first job, **CI image**, runs on the gate runner (`gate` label) in host
mode. It does three things:

1. **Tag.** Checks that `CI_IMAGE`'s tag equals the first 12 hex digits of the
   Dockerfile's SHA-256, that `release.yml` uses the same `CI_IMAGE`, and that
   `.forgejo/ci-image` holds nothing but the Dockerfile.
2. **Pins.** Checks that the Dockerfile's `RUST_TOOLCHAIN` matches
   `rust-toolchain.toml`, and that its `RUST_MSRV` matches `Cargo.toml`'s
   `rust-version` and the MSRV job's `RUSTUP_TOOLCHAIN`.
3. **Image.** Queries Forgejo's container registry for the tag:
   - If the tag exists, it pulls the image into the VM's Docker. The registry
     requires sign-in to pull, and the runner never pulls job images itself
     (`force_pull: false`), so this pull is what gets the image onto a fresh VM
     for the jobs that follow.
   - If the tag is missing, it builds and pushes the image.
   - Any other registry error fails the job.

A PR that changes the Dockerfile therefore builds and tests its own image.

**Changing the image** (a toolchain bump, a tool version, an apt package):

1. Edit the Dockerfile.
   - For a toolchain bump, also move `rust-toolchain.toml`.
   - For an MSRV bump, also move `Cargo.toml`'s `rust-version`, the MSRV job's
     `RUSTUP_TOOLCHAIN` and `scripts/ci/check-msrv.sh`.
   - For a tool bump, update its `*_SHA256` along with its version.
2. Set `CI_IMAGE`'s tag, in both `ci.yml` and `release.yml`, to
   `$(sha256sum .forgejo/ci-image/Dockerfile | cut -c1-12)`.

Never re-push an existing tag. Change the Dockerfile, even just a comment, to get
a new one.

**Credentials:** the image job logs in with the `CI_IMAGE_TOKEN` repository
secret, a personal access token with only package read and write scope, used for
both the pull and the push.
- Forgejo's automatic job token can log in, but gets 401 on uploads.
- The login uses a Docker config under `RUNNER_TEMP`, which the runner deletes
  even if the job is cancelled.
- The pull reaches the job containers because the gate runner shares the VM's
  Docker with the main runner. If the runners ever span more than one VM, or
  turn on `force_pull`, give the jobs `container.credentials` with a separate
  read-only package token.

**Storage:** each image version takes space on Forgejo's data disk, and old tags
stay cached on the runner VM until it's rebuilt. Keep the last few versions with
a package cleanup rule, set in the owner's Settings → Packages.

**Caches:** the image sets `CARGO_HOME=/usr/local/cargo`, so switching to it
started every job with a cold Rust cache once. Later image changes keep the same
paths, and the caches still hit while the toolchain stays the same.

The workflow sets `CARGO_INCREMENTAL=0` and drops native debug info from dev and
test builds (`CARGO_PROFILE_{DEV,TEST}_DEBUG=0`) to cut codegen, link time and
cache size. Optimization level, debug assertions, overflow checks and test
selection keep their defaults, and there is no `RUSTFLAGS=-Dwarnings`: per-rule
severity lives in `[workspace.lints]`.

## Native tests (GitHub)

[`.github/workflows/native-tests.yml`](../.github/workflows/native-tests.yml)
runs the tests, clippy and rustdoc natively on two GitHub-hosted runners
(#950), which the Linux-only Forgejo runner can't:

- **Test (macOS arm64)**: `macos-latest`, Apple Silicon;
- **Test (Windows x86_64)**: `windows-latest`, the MSVC toolchain.

**Trigger.** Every push to any branch, and a manual dispatch. PRs live on
Forgejo, so GitHub's `pull_request` event never fires; the push mirror brings
every PR branch, and every merge to `dev`, to GitHub instead. Tag pushes don't
run it. A newer push to a branch cancels that branch's running run.

**Steps.** `NATIVE_FEATURES` is the `features=` list in
[`scripts/ci/local-macos.sh`](../scripts/ci/local-macos.sh), which the
workflow reads: every optional feature except the Linux-only `serial` and
`ethernet`, including per-crate ones such as `bacnet-endpoint/sc-tls`. Windows
also leaves out `bacnet-cli/pcap`, which needs the Npcap SDK. Each job runs:

```bash
cargo nextest run --workspace --exclude rusty-bacnet --locked --features "$NATIVE_FEATURES" --profile ci
# stack guard (#953): STACK_GUARD_TESTS again with 1 MiB thread stacks, after
# `ulimit -s 1024` on macOS so the CLI tests' `bacnet` processes get a 1 MiB
# main thread, as on Windows
RUST_MIN_STACK=1048576 cargo nextest run --workspace --exclude rusty-bacnet --locked --features "$NATIVE_FEATURES" --profile ci -E "$STACK_GUARD_TESTS"
cargo test --doc --workspace --exclude rusty-bacnet --locked --features "$NATIVE_FEATURES"
cargo nextest run -p bacnet-cli --locked --profile ci   # the CLI's feature-off tests
cargo clippy --workspace --exclude rusty-bacnet --all-targets --locked --features "$NATIVE_FEATURES" -- -D warnings
cargo clippy -p rusty-bacnet --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --exclude rusty-bacnet --no-deps --locked --document-private-items --features "$NATIVE_FEATURES"
RUSTDOCFLAGS="-D warnings" cargo doc -p rusty-bacnet --no-deps --locked --document-private-items
# Python 3.12 from actions/setup-python, in a fresh venv
python -m pip install maturin==1.15.0
maturin develop -m crates/rusty-bacnet/Cargo.toml --locked
python -m unittest discover -s crates/rusty-bacnet/tests
cargo nextest run -p rusty-bacnet --locked --profile ci
```

PyO3 builds link setup-python's interpreter (`PYO3_PYTHON`), not the venv's.
Every step runs even when an earlier one failed, so one run reports each
failure. `STACK_GUARD_TESTS` (in the workflow's `env`) selects every server,
client, endpoint, integration and CLI test, the benchmark SC mTLS tests and
bacnet-transport's BACnet/SC tests; the guard step runs `--no-run` first
because rustc reads `RUST_MIN_STACK` too, and adds a minute or two to each
job. The per-crate default-feature checks
(`scripts/ci/check-default-features.sh`) aren't here: Forgejo's Clippy job
runs them for Windows and macOS too, cross-checked (see
[Local checks](#local-checks)).

Before anything else, the Windows job stops the Microsoft Compatibility
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

**Toolchain and tools.** Both runner images ship rustup, and
`rustup toolchain install` with no arguments installs what
`rust-toolchain.toml` pins, so the workflow has no toolchain version to keep in
step. cargo-nextest is a prebuilt binary from `taiki-e/install-action`, and
maturin comes from PyPI, both at the CI image's versions. aws-lc-sys, which
`sc-tls` pulls in, builds with the images' own C tools: on Windows, MSVC with
the NASM and CMake already on `PATH`. The SC tests make certificates with the
runner image's `openssl`.

**Efficiency.** The workflow can only read the repository
(`permissions: contents: read`), and each job stops after 60 minutes. A newer
push to a branch cancels that branch's running run. On `dev` each commit gets
its own concurrency group, keyed by its SHA, so no run is cancelled or
replaced while pending and every merge gets its own result. `Swatinem/rust-cache` keeps
dependency builds, keyed per OS on the toolchain, `Cargo.lock`, the manifests
and `NATIVE_FEATURES`. Only `dev` saves it, and only from a successful job, so
a failed or cancelled run never leaves a partial cache that later runs would
restore by exact key. GitHub lets a branch's run restore the default branch's
(`dev`) cache, so every branch starts from the last good `dev` build. The cache
holds dependencies only, so most of a run is compiling the workspace, its
tests, clippy and rustdoc: in October 2026 a run took about 16 minutes on
macOS and 21 on Windows cold, and 13 and 16 with a warm cache.

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

**Reading a run.** The run for a push appears once the mirror has the commit:

```bash
gh api repos/jscott3201/rusty-bacnet/branches/<branch> --jq .commit.sha
gh run list -R jscott3201/rusty-bacnet --workflow native-tests.yml --branch <branch>
gh run view -R jscott3201/rusty-bacnet <run id> --log-failed
gh workflow run native-tests.yml -R jscott3201/rusty-bacnet --ref <branch>  # re-run by hand
```

## Local checks

Use Rust 1.99.0 from `rust-toolchain.toml`. The [native tests](#native-tests-github)
now run the macOS tests, clippy and rustdoc on every push, so a local macOS run
is optional: a quicker check before pushing changes that can affect macOS
(transports, sockets, TLS, platform `cfg`, build scripts, dependencies). It
isn't merge evidence.

```bash
bash scripts/ci/local-macos.sh          # lint, clippy, rustdoc, macOS tests
bash scripts/ci/local-macos.sh --quick  # lint, clippy and rustdoc only
```

`serial` and `ethernet` are Linux-only features, so macOS uses every other
optional feature. The native-tests workflow reads the same list from the
script. That includes per-crate features such as
`bacnet-endpoint/sc-tls` and `bacnet-cli/{sc-tls,pcap}`, which nothing else in
the workspace turns on, so a transport-only list never built them (#906). CI's
`LINUX_FEATURES` is the same list plus `bacnet-transport/{serial,serial-gpio,ethernet}`
and `bacnet-integration-tests/ethernet`.

The script also runs the PyO3 crate's Rust tests. pyo3 links the libpython of
`PYO3_PYTHON` if it's set, else of the active venv, else of the first `python`
or `python3` on `PATH`. The Homebrew and python.org framework builds both ship
the shared library.

Clippy and rustdoc deny warnings (#902). Every public item must be documented:
`missing_docs` is `deny`, and only the unpublished `bacnet-benchmarks` opts out.
Clippy runs three ways:

- the workspace with every feature;
- the PyO3 crate on its own;
- `bacnet-cli` with no default features (without the TUI);
- each published crate alone with default features, plus the `no_std` build of
  `bacnet-types` (`scripts/ci/check-default-features.sh`). This also runs
  rustdoc, which is how docs.rs builds.

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
would check none of its docs. The per-crate default-features run above stays
public, as docs.rs builds.

The individual gates are also runnable anywhere. `FEATURES` is
`LINUX_FEATURES` from `ci.yml`, without the serial and ethernet entries on macOS:

```bash
FEATURES=$(sed -n 's/^  LINUX_FEATURES: //p' .forgejo/workflows/ci.yml)
cargo fmt --all --check
cargo clippy --workspace --exclude rusty-bacnet --all-targets --locked --features "$FEATURES" -- -D warnings
cargo clippy -p rusty-bacnet --all-targets --locked -- -D warnings
cargo clippy -p bacnet-cli --no-default-features --all-targets --locked -- -D warnings
bash scripts/ci/check-default-features.sh   # the host; or pass target triples, as CI does
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --exclude rusty-bacnet --no-deps --locked --document-private-items --features "$FEATURES"
RUSTDOCFLAGS="-D warnings" cargo doc -p rusty-bacnet --no-deps --locked --document-private-items
cargo nextest run -p bacnet-cli --locked   # the CLI's feature-off tests
cargo nextest run -p bacnet-cli --no-default-features --locked   # without the TUI
cargo nextest run -p rusty-bacnet --locked # the PyO3 crate's Rust tests
bash scripts/ci/check-file-size.sh
bash scripts/ci/test-check-no-secrets.sh && bash scripts/ci/check-no-secrets.sh
python3 scripts/ci/test-check-msrv.py
python3 -m unittest discover -s scripts/release
python3 -m unittest discover -s scripts -p 'test_changelog.py'
python3 scripts/changelog.py check
python3 -m unittest discover -s scripts -p 'test_ledger_*.py'
```

`changelog.py check` validates the [changelog fragments](../changelog.d/README.md)
and fails if `CHANGELOG.md`'s `[Unreleased]` section lists entries itself, so a
PR that edits it directly fails the lint job.

The no-secret scanner reports stable opaque path IDs and line numbers, never
paths or matching text, because filenames can contain credentials too. Compare
a suspected path locally with `printf '%s' "$path" | git hash-object --stdin`.
Run the file-size gate in its default strict mode, without `CHECK_FILE_SIZE_WARN=1`.

`scripts/ci/check-msrv.sh --linux-native` needs a native Linux GNU host with
Rust 1.93 installed, Python 3, a C toolchain, `pkg-config`, `libpcap-dev`,
`cmake`, `perl`, `file` and `ldd`. It never installs tools or skips features.
CI runs it on PRs to `main`; on a Mac, rely on that job.

## Merge evidence

Before merging a PR:

- `CI OK` is green on Forgejo for the exact head being merged;
- both jobs of the [native tests](#native-tests-github) are green on GitHub
  for the same head SHA;
- existing review and merge-authorization rules are met.

A `local-macos.sh` pass is optional and isn't needed to merge.

A check that was not run, failed or does not apply is never reported as passed.
Audit and deny read mutable advisory databases, so their result for a `main`
merge comes from that PR's run, not an older one.

## Release

[`.forgejo/workflows/release.yml`](../.forgejo/workflows/release.yml) builds,
tests and publishes a release from Forgejo (#943), which keeps the heavy work
on the fixed-cost runner VM; GitHub only receives a copy of each release. The
Linux runner cross-compiles every artifact: Linux and macOS with zig, Windows
for the MSVC target with cargo-xwin (#944). Before anything is published,
GitHub-hosted runners run the macOS and Windows artifacts, which the Linux
runner can't (the [smoke test](#smoke-test), #951). GitHub Pages publication
remains the manual [`docs-pages.yml`](../.github/workflows/docs-pages.yml)
dispatch on GitHub.

### Trigger and dry run

- **Tag.** Pushing a `v*` tag to Forgejo runs the whole release.
- **Dry run.** A manual dispatch is a dry run by default (`dry_run` is true). It
  builds and tests every artifact, keeps them as workflow artifacts, runs the
  [smoke test](#smoke-test) against a throwaway GitHub draft that it deletes
  again, makes the release API calls read only (see
  [Release API](#release-api-dry-run)) and publishes nothing. The notes come
  from the workspace version's `CHANGELOG.md` section if it has one, otherwise
  from the section the waiting `changelog.d/` fragments would make, which may
  be empty. `changelog.py assemble --output` writes that into a copy, so the
  checkout is left alone:

  ```bash
  tea api -X POST repos/jscott3201/rusty-bacnet/actions/workflows/release.yml/dispatches \
    --data '{"ref":"dev","inputs":{"dry_run":"true"}}'
  ```

  `dry_run=false` is accepted only on a `v*` tag, where it runs that tag's
  release again.

To release:

1. Set the workspace version, assemble its `CHANGELOG.md` section from the
   fragments, add release highlights by hand under the new heading if the
   release has any, and merge:

   ```bash
   python3 scripts/changelog.py preview        # what the section will hold
   python3 scripts/changelog.py assemble --version 0.12.0 [--date 2026-10-02]
   ```

   `assemble` adds `## [0.12.0] - <date>` below `[Unreleased]`, with the
   entries grouped by heading and ordered by issue number, and deletes the
   fragments it used. A tag fails if any fragment is still waiting.
2. Optionally, dispatch a dry run on the release commit first: it runs
   everything the release does, the smoke test included, without publishing.
3. Tag the commit, on `main` or `dev`:

   ```bash
   git tag -a v0.12.0 -m "Rusty BACnet 0.12.0"
   git push origin v0.12.0
   ```

The tag push also starts CI on the tagged commit, with the heavy jobs, and the
release waits for it (see [CI gate](#ci-gate)). Before that, and before any
build, the [preflight](#preflight) checks that both release hosts will take
the release.

### Jobs

| Job | What it does |
| --- | --- |
| CI image | Pulls `CI_IMAGE` into the VM's Docker. Only `ci.yml` builds the image, and its image job fails if `release.yml` carries another tag. |
| Validate | Runs the release script tests (`scripts/release/test_*.py`, `scripts/test_changelog.py`). Checks that every publishable crate has the workspace version and, for a tag, that the tag is `v<version>` and the commit is on `dev` or `main`. For a release, checks that the publish secrets are set and runs the [preflight](#preflight), before anything is built; a dry run runs the preflight's read-only part. For a tag, checks that no `changelog.d/` fragment is left unassembled. Extracts the notes with `changelog_notes.py`, writes `THIRD-PARTY-NOTICES`, then runs the [CI gate](#ci-gate). |
| Crates and sdist | `cargo publish --workspace --dry-run --locked`, which packages every publishable crate and builds each against the others as published. Then the crates.io job's plan (read only), `cargo package` for the `crates` artifact, and `maturin sdist`. |
| Wheels (linux-x86_64, linux-aarch64, macos-x86_64, macos-arm64, windows-x86_64) | `maturin build --release --locked` for CPython 3.11 to 3.14: with `--zig --compatibility manylinux2014` for Linux, `--zig` for macOS, and maturin's built-in xwin for Windows ([macOS and Windows builds](#macos-and-windows-builds)). The image has only Python 3.12; maturin uses its bundled sysconfig for the others and for macOS and Windows. |
| CLI (linux-amd64, linux-arm64, macos-amd64, macos-arm64, windows-amd64) | Linux: `cargo zigbuild --release --locked -p bacnet-cli --features sc-tls,pcap` for `<target>.2.17`, against the image's static libpcap. `LIBPCAP_VER` gives the pcap crate libpcap's version, which its build script can't load through the linker-script shim, and must match the image's `/opt/libpcap/VERSION`. macOS: `cargo zigbuild` with `--features sc-tls`. Windows: `cargo xwin build` with `--features sc-tls` and the C runtime linked statically. |
| Test the artifacts | `check_artifacts.py`: one wheel per Python and platform with the right tags, version (the workspace version in PEP 440 form) and extension module, and `THIRD-PARTY-NOTICES` in each wheel and the sdist. For every binary, its architecture and linkage: ELF with nothing above glibc 2.17 (`objdump -T`) and no dynamic libpcap; Mach-O with the tag's minimum macOS, a code signature on arm64, and only the expected libraries and flat lookups (`llvm-objdump`); PE with only the expected DLLs (`llvm-readobj`). Output that parses to nothing fails (no bind table, no Python lookups in an extension module, no `kernel32.dll` import). The Python suite against the installed x86_64 cp312 wheel. `cli_smoke.sh`: the amd64 CLI's `--version` and `--help`, the README quickstart read on loopback, and an offline capture of a one-packet pcap file. The quickstart read again with the server on each other x86_64 wheel, in CPython 3.11, 3.13 and 3.14 from `uv python install`. The arm64 CLI's `--version`, `--help` and offline capture under `qemu-aarch64-static`. |
| Smoke test (macOS, Windows) | `release_smoke.py gate`: stages the release's files on a GitHub draft (the release's own; a dry run's throwaway `release-smoke-<run id>`), dispatches [`release-smoke.yml`](../.github/workflows/release-smoke.yml) against it and waits for it, failing on a failure or after 60 minutes. A dry run deletes its draft again. Skipped, with a warning, by a dry run without `GH_RELEASE_TOKEN`. See [Smoke test](#smoke-test). |
| Release API (dry run) | Dry runs only. `release_api.py forgejo --dry-run` against Forgejo with the job token, and `release_api.py github --dry-run` if `GH_RELEASE_TOKEN` is set (otherwise a notice says it was skipped). Both read only, for the tag `v<version>`. |
| Publish to crates.io | `publish_crates.sh`: one multi-package `cargo publish --no-verify` of the crates whose version isn't on crates.io yet. Cargo orders them and waits for the index. |
| Publish to PyPI | `maturin upload --skip-existing` of the wheels and the sdist. |
| Forgejo release | `release_api.py forgejo`: [draft, upload, publish](#draft-then-publish). |
| GitHub release copy | `release_api.py github --staged-release <id> --staged-sums <sha256>`: checks again that the mirror's tag points at the release commit (the preflight already waited for it), then publishes the draft the smoke test ran against, unchanged: it must be that draft, holding exactly these files with their digests and the same `SHA256SUMS`, and nothing is uploaded or deleted. |

The publish jobs run only for a tag, after every build and test passed, the
smoke test included, one at a time in the order above. A failure stops the
jobs after it: each publish
job's `if:` starts with `success() &&`, because Forgejo leaves the implicit
`success()` to the runner. Release builds use no Rust cache.

#### What is and isn't tested

Every file gets the static checks above. The Forgejo runner is Linux x86_64,
so it runs only the x86_64 Linux wheels and CLI, and the arm64 Linux CLI under
qemu; the [smoke test](#smoke-test) runs the macOS and Windows ones on
GitHub-hosted runners:

- the aarch64 Linux wheels: static checks only (tags, module names, ELF
  machine and glibc symbols); the arm64 CLI's network commands aren't run;
- the macOS wheels and CLI, on Apple Silicon (`macos-latest`) and Intel
  (`macos-26-intel`), and the Windows ones (`windows-latest`): each wheel in
  its own CPython, with an import, `list_serial_ports()` and a loopback
  round trip, and each CLI's `--version`, `--help` and quickstart read. The
  Python suite doesn't run against them; native tests run it on macOS arm64
  and Windows against a local build of each PR.

These smoke tests show that the files load and work on those hosts, not that
the Python suite passes against them, and they say nothing about hardware or
older macOS and Windows versions than the runners'.

### CI gate

`scripts/release/ci_gate.py` reads Forgejo's combined status for the commit
(`/repos/{owner}/{repo}/commits/{sha}/status`), which holds the latest status of
each context, so an older success can't hide a newer failure. A release needs
all three of these to be `success`:

- `CI / CI OK (push)`
- `CI / MSRV (Linux native) (push)`
- `CI / Cargo Audit + Deny (push)`

MSRV and audit/deny are heavy jobs, which a dev push skips and a tag push
runs. A dev push's run posts `skipped` for them under the same context names,
and the tag is often on a commit that dev's run has already reported on, so
`skipped` can be the latest state until the tag's run reports. On a tag, the
gate polls every 30 seconds for up to 60 minutes while any of the three is
`pending`, `skipped` or not reported yet. It fails at once on `failure` or
`error` (including `CI OK` failing) or any other state. At the deadline it
fails with what each context showed, and says to re-run the release once the
tag's CI run has passed; a failure's message says to re-run the failed CI jobs
if it was transient. A dry run checks once and only warns. A dispatched CI run
posts no commit statuses, so only push runs count.

The gate reads the combined status page by page. On this Forgejo,
`total_count` is the size of the page, not of the list, so the gate stops at
the first page shorter than the 50 it asks for.

### Preflight

Before anything is built, a release run checks that both hosts will take it,
so that a token or host setting problem stops the run before crates.io and
PyPI, which can't be undone. Validate's "Release preflight" step runs
`release_api.py forgejo --preflight` and then `release_api.py github
--preflight`:

1. **Tag** (GitHub only). `GET /repos/{o}/{r}/git/ref/tags/{tag}` (and
   `git/tags/{sha}` for an annotated tag), polling every 30 seconds for up to
   15 minutes until the push mirror has the tag, which must point at the
   release commit. The repository is public, so this call is anonymous.
2. **Release list.** The release list (`GET /repos/{o}/{r}/releases`, every
   page) with the token, which shows whether the release for the tag is
   absent, a draft the publish job will resume, or published (then only
   checked). A draft made for another commit stops the run here, as the
   publish job would. Drafts named `release-preflight-*` or `release-smoke-*`
   (a dry run's [smoke test](#smoke-test) draft) that an earlier run couldn't
   delete are reported, not deleted, since another run may be using its own.
3. **Write check.** A disposable draft named `release-preflight-<run id>-<8 hex>`.
   It isn't a `v*` name, so no tag rule applies, and it's unique even when a
   run is re-run. The calls:
   - `POST /repos/{o}/{r}/releases` with `draft: true`, `prerelease: true` and
     `target_commitish: <commit>`. Neither host creates a tag for a draft.
   - The release list again, which must show the new draft: proof that the
     token sees drafts, which resuming a release depends on.
   - Four uploads named like the release's assets: `bacnet-linux-amd64` (one
     byte, extension-less like the Linux and macOS CLI binaries, `SHA256SUMS`
     and `THIRD-PARTY-NOTICES`), `bacnet-windows-amd64.exe` (two bytes),
     `rusty_bacnet-0.0.0-py3-none-any.whl` (an empty zip) and
     `rusty_bacnet-0.0.0.tar.gz` (an empty gzip). This proves the
     host's allowed attachment types (Forgejo's `[repository.release]
     ALLOWED_TYPES`) accept every kind.
   - The [final check](#draft-then-publish) on the draft: on GitHub, each
     reported `digest`, and a download of the one-byte file through the API
     asset URL; on Forgejo, each size.
   - `DELETE /repos/{o}/{r}/releases/{id}`.
   - Confirmation: the release list has no release of that name, `GET
     .../releases/{id}` is 404, and no tag of that name exists (GitHub:
     `GET git/ref/tags/<name>` is 404; Forgejo: the whole `GET .../tags` list,
     which comes from git).

The draft is deleted in a `finally` block, whatever failed before: by its id
if the create returned one, and by name from the release list in case the
create's response was lost. If the deletion itself fails, the step fails and
names the draft to delete by hand; if the check had already failed, that
error is reported as well. Forgejo's API never removes a deleted release's
database row: it keeps it as a tag record without a commit, which no API list,
git, the web tags page or the tag count shows, so nothing visible is left.

Any failure stops the release before anything is built, with the reason and
"Nothing has been built or published". A dry run does only the read-only part
(steps 1 and 2, for `v<version>`, and without waiting for the tag), never the
write check; on GitHub it does only the anonymous tag check while
`GH_RELEASE_TOKEN` isn't set, and says so in a notice.

### Smoke test

The Forgejo runner can't run the macOS and Windows artifacts. GitHub-hosted
runners can, but they can't reach Forgejo, which is on the tailnet and needs
sign-in. So the files reach GitHub on a draft release, and the "Smoke test
(macOS, Windows)" job runs after the artifact tests and before anything is
published. It runs `release_smoke.py gate`:

1. **Stage.** On a release, the GitHub draft for the tag (`release_api.py`'s
   `stage`): created at the release commit, or resumed (a draft for another
   commit stops the run), and made to hold exactly this run's
   `release-assets` files and their `SHA256SUMS`, then the
   [final check](#draft-then-publish). Unlike the publish job's resume, it
   replaces an asset that differs from this run's file instead of keeping
   it, so the smoke test runs exactly what PyPI and both releases get. A
   dry run makes a throwaway draft, `release-smoke-<run id>`, at the commit
   instead (deleting one left by an earlier attempt of the run). A release
   that is already published is only checked, and its files are tested.
2. **Dispatch** [`release-smoke.yml`](../.github/workflows/release-smoke.yml)
   on the run's ref (the tag, or a dry run's branch) with the inputs
   `release_id`, `sums_sha256` (of the staged `SHA256SUMS`), `version`,
   `commit`, `pythons` (`PYTHONS`) and `correlation_id`
   (`<run id>-<8 hex>`). The run is named `Release smoke <correlation id>`.
   The dispatch asks GitHub for the run's id (`return_run_details`), and the
   gate checks that it is a `workflow_dispatch` run of `release-smoke.yml`
   (not its name: GitHub calls a new run after the workflow until it has
   evaluated `run-name`). Without the id, or after a lost response, the gate
   finds the run by its name rather than dispatching twice. The run must be
   for the release commit: its fetch job runs only its own commit's scripts,
   so if the ref has moved on (a branch pushed to during a dry run), the gate
   stops there. A tag doesn't move.
3. **Wait.** Poll the run every 30 seconds, printing each job's state as it
   changes, for up to 60 minutes, then fail. Whenever the gate fails while
   the run is still going, it asks GitHub to cancel the run before a dry run
   deletes its draft. Cancelling is asynchronous, so a job that is already
   running may still look for the draft after it's gone; it then fails on the
   missing release, and nothing is published either way.
4. **Report.** Print every job with its result, its link and any failed step.
   The gate passes only if the run succeeded and each of its four jobs did,
   all of them present (a job skipped by mistake fails it).
5. **Clean up** (dry run): delete the throwaway draft, whatever happened, and
   check that no release or tag of that name is left. The job's last step
   does the same in case the gate was killed first. Both refuse any name but
   `release-smoke-*` (checked before anything else) and any release that
   isn't a draft.

The gate writes the draft's id and the `SHA256SUMS` sha256 as job outputs.
The GitHub release copy publishes that draft with `--staged-release` and
`--staged-sums`: the tag's release must be that draft, this run's files must
give the same `SHA256SUMS`, and the draft must still hold exactly them with
matching digests. It uploads and deletes nothing, so what is published is
what was smoke-tested. It fails closed: a GitHub publish without both values,
or with an empty one, publishes nothing (the step checks, and so does
`release_api.py`), and there is no other way to publish on GitHub.

**The GitHub workflow.** `release-smoke.yml` runs only when dispatched:

- **Fetch the draft's assets** (`ubuntu-latest`, 10 minutes): checks each
  input's form and that `commit` is the run's own commit (`github.sha`), so a
  dispatch can't make this job, whose token can write, run another commit's
  code. It then checks out `scripts/release` at that commit and runs
  `release_smoke.py fetch`. That reads the draft by id, checks that its
  `SHA256SUMS` has the staged sha256, downloads each macOS and Windows wheel
  and CLI binary through the API, checks it against `SHA256SUMS`, and hands
  each platform's files on as a workflow artifact kept for a day.
- **Smoke test (macOS arm64)** on `macos-latest`, **(macOS x86_64)** on
  `macos-26-intel`, GitHub's newest Intel image, and **(Windows x86_64)** on
  `windows-latest` (20 minutes each; one failing doesn't cancel the others),
  with CPython 3.11 to 3.14 from `actions/setup-python`:
  - each wheel, installed with `pip --no-index --no-deps` into a fresh venv
    of its own CPython, runs
    [`wheel_smoke.py`](../scripts/release/wheel_smoke.py): the installed
    version is the release's; `list_serial_ports()` returns a list of port
    names, which on macOS goes through IOKit and CoreFoundation, linked
    through the [framework stubs](#macos-and-windows-builds) and resolved by
    flat lookup, and both frameworks must then be loaded, and on Windows goes
    through SetupAPI; and a loopback round trip with the public API, where a
    `BACnetServer` on 127.0.0.1 serves an analog input and an analog value
    and a `BACnetClient` reads the input's present value, reads it and its
    name with ReadPropertyMultiple, writes the value's present value at
    priority 8 and reads it back;
  - the CLI binary runs `cli_smoke.sh --no-capture --expect-version`:
    `--version` must print `bacnet <version>`, `--help` runs, and the README
    quickstart `read` and `--json readm` against the README server on the
    cp312 wheel. These builds have no packet capture.
- **Permissions.** The workflow's default is none. GitHub shows a draft
  release only to a token with write access: with `contents: read`, the job
  token gets HTTP 403, "Resource not accessible by integration", for the
  draft (run 36913737816, 2026-10-01). So the fetch job has `contents:
  write`, the least that works, and runs only the release commit's
  `release_smoke.py`, which only reads. The smoke jobs, which run the
  artifacts, have only `contents: read`, for the checkout; their files come
  from the run's artifact storage. No secret is used, and every action is
  pinned to a full commit SHA, as the repository requires.
- **The token.** Forgejo's `GH_RELEASE_TOKEN` stages the draft (Contents
  write), dispatches the run (Actions write), and reads and, at the
  deadline, cancels it (Actions read and write).

GitHub dispatches only a workflow it knows: one on the default branch (`dev`),
or one that has already run. A dispatch on a tag or branch then runs that
ref's copy of the file, so a renamed or new workflow must reach `dev` before a
release can dispatch it. (While #951 was on its branch, a temporary push
trigger registered the workflow.)

**Reading a failed smoke test.** The smoke job's log ends with the GitHub run's
link, each job's result and link, and every failed step:

- **Fetch the draft's assets** failed: the draft isn't what was staged (its
  `SHA256SUMS` changed, a wheel or binary is missing, or a file doesn't match
  its digest), or the token can't read it. Nothing ran.
- **Smoke test (*platform*)**, step **Wheels, CPython 3.11 to 3.14**: each
  wheel's output is a log group named after the wheel, and the last one shows
  which wheel and which check failed: the install, the import, the version,
  the serial port listing or the round trip. On macOS, an abort with "symbol
  not found in flat namespace" or "Library not loaded" points at the
  framework links.
- Step **CLI**: `cli_smoke.sh`'s output, with the Python server's log if it
  didn't start.
- The gate timed out: it cancelled the run. A long wait for a runner, most
  often the Intel macOS one, is the usual cause.
- The gate says the run is for another commit: the branch moved during a dry
  run. Dispatch the dry run again.

A failed smoke test publishes nothing, and on a release the tag's GitHub draft
stays a draft. "Re-run all jobs" (or a `dry_run=false` dispatch on the tag)
recovers only from a runner, network or GitHub failure: it rebuilds the same
commit, stages the draft again and runs the smoke test before publishing. A
bug in an artifact needs a new commit. Release it as a new version, or, to
reuse the tag, first delete the tag's GitHub draft: it was made for the old
commit, and a draft for another commit stops the run. To run the smoke
workflow by hand against an existing draft:

```bash
gh workflow run release-smoke.yml -R jscott3201/rusty-bacnet --ref <tag or branch> \
  -f release_id=<draft id> -f sums_sha256=<sha256 of its SHA256SUMS> -f version=<version> \
  -f commit=<full sha> -f pythons="3.11 3.12 3.13 3.14" -f correlation_id=manual-1
```

### Draft, then publish

GitHub releases in this repository are immutable once published: assets can't
be added, replaced or deleted, and the tag can't be reused. `release_api.py`
therefore builds each release as a draft and publishes it last. For GitHub:

1. `GET /repos/{o}/{r}/releases` (all pages), matching `tag_name`, because
   `/releases/tags/{tag}` doesn't return drafts. A published release is only
   checked, read only. Otherwise:
2. `POST /repos/{o}/{r}/releases` with `draft: true` and
   `target_commitish: <commit>`, unless a draft exists.
3. `POST uploads.github.com/.../releases/{id}/assets?name=...` for each asset
   the draft lacks.
4. `SHA256SUMS`, computed over the draft's final asset set, uploaded the same
   way (an outdated one is deleted first).
5. The final check, on a fresh `GET` of the draft (below).
6. `PATCH /repos/{o}/{r}/releases/{id}` with `draft: false` and
   `make_latest: "legacy"`, the last call. `legacy` has GitHub pick the latest
   release by date and version, so a backport published after a newer
   release doesn't become the latest.

On GitHub, the smoke job does steps 1 to 5 when it stages the draft (see
[Smoke test](#smoke-test)), and the GitHub release copy checks that same draft
again (step 5) and publishes it (step 6), without uploading or deleting
anything.

The Forgejo release follows the same order through Forgejo's API, so
`releases/latest` never shows a half-uploaded release. Forgejo's API has no
`make_latest`: its latest release is the newest published non-prerelease by
creation date, so a backport published after a newer release does show as
Forgejo's latest until the next release.

- **Final check.** The last step before the irreversible publish. The raw
  asset list, before any filtering by state, must hold exactly the expected
  names, once each, with no entry other than `uploaded` (GitHub's `state`).
  Each asset's size must match, and on GitHub its reported `digest`
  (`sha256:<hex>`) must equal the expected sha256: the local file's for an
  upload, the verified digest of an asset kept from an earlier run, and for
  `SHA256SUMS` the sha256 of the text this run computed or found up to date.
  An asset without a digest is downloaded and hashed. Forgejo reports no
  digest and won't serve the assets to the job token, so there the check is
  each asset's size against the local file's (every Forgejo asset is this
  run's upload).
- **Resuming.** A draft left by an earlier run keeps its notes, but only if its
  `target_commitish` is the release commit: a draft made for another commit
  stops the run with a message to delete it. Assets that aren't part of this
  release (the local assets plus `SHA256SUMS`) are deleted first, as are all
  copies of a name that appears more than once (Forgejo allows that; the run
  then uploads one). On GitHub, the other assets are kept, with the checksums
  GitHub reports for them, and `SHA256SUMS` lists what the draft holds.
  Forgejo's web routes, the only way to download an attachment, don't accept
  the job token for a private repository, so on a resumed Forgejo draft this
  run's files replace the existing ones.
- **Published.** The script never uploads to or deletes from a published
  release. It checks that every asset and `SHA256SUMS` are there and, on
  GitHub, that each asset matches `SHA256SUMS`, and fails with an explanation
  otherwise.
- **Retries.** Reads, the final `PATCH` and deletes retry on 5xx and network
  errors, including a truncated response (`http.client.HTTPException`, such
  as `IncompleteRead`). Deletes treat 404 as done, so a retried delete
  succeeds. A `POST` doesn't retry: after an uncertain upload failure, or
  GitHub's 422 `already_exists`, the script lists the draft's assets again
  and accepts the asset if it's complete and matches (digest on GitHub, size
  on Forgejo), or deletes it and sends the file again. A failed create looks
  for the draft before trying again.
- **Downloads.** GitHub serves a draft's assets only through the API asset URL
  with `Accept: application/octet-stream`. The token goes in an unredirected
  header, so the redirect to storage never carries it.

#### Release API (dry run)

The dry run calls the same code with `--dry-run`, which makes no write: it
finds the release, checks a published one, prints the deletions and uploads a
real run would make (or warns that a draft for another commit would stop it)
and, on GitHub, downloads the smallest existing asset. While the workspace
version is already released, the check reports what the published release
lacks as a warning.

### Artifacts

- `release-assets`: what the publish jobs upload, which the releases also get
  with a `SHA256SUMS` file:
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
- `release-notes`: `notes.md` for Forgejo, and `notes-github.md` for GitHub.
  GitHub refuses bodies over 125,000 characters, so a longer section is cut at
  120,000 with a link to the full `CHANGELOG.md`, closing any code block the
  cut leaves open. The 0.11.0 section is about 171,000.
- `notices`: `THIRD-PARTY-NOTICES`, which the sdist and wheel jobs build in.
- `crates`, `sdist`, `wheels-<platform>` and `cli-<platform>`: each build
  job's output.

The Linux binaries and wheels need glibc 2.17 or newer, which covers
RHEL/CentOS 7, Debian 8, Ubuntu 14.04 and later. zig links them against that
glibc, so no manylinux container is involved. The CLI links libpcap 1.10.7
statically, built for each target in the CI image: Debian and Ubuntu name the
shared library `libpcap.so.0.8` and RHEL `libpcap.so.1`, so one dynamically
linked binary couldn't run on both. The pcap crate links `-lpcap` as a shared
library and zig won't fall back to an archive, so the image puts a one-line
linker script named `libpcap.so` next to `libpcap.a`.

### macOS and Windows builds

The Linux runner cross-compiles these too, in the CI image (#944).

**macOS** (`x86_64-apple-darwin`, `aarch64-apple-darwin`) uses zig, through
cargo-zigbuild for the CLI and `maturin build --zig` for the wheels. zig carries
the macOS libc headers and libSystem link stubs. No Apple SDK is involved: its
licence doesn't allow redistributing it. rustc warns in these jobs that `xcrun`
can't find an SDK; zig doesn't need one.

- **Minimum macOS.** 10.12 on x86_64 and 11.0 on arm64, as in 0.11.0's wheel
  tags (`macosx_10_12_x86_64`, `macosx_11_0_arm64`) and Rust's defaults.
  `MACOSX_DEPLOYMENT_TARGET` sets it, and maturin derives the wheel tag from
  it. cargo-zigbuild passes zig a target without an OS version
  (`<arch>-macos-none`), for which zig defaults to macOS 13, and zig ignores
  `-mmacosx-version-min`. The jobs therefore set
  `CARGO_ZIGBUILD_ZIG_PATH` to
  [`scripts/release/zig-macos.sh`](../scripts/release/zig-macos.sh), which
  writes the version into zig's target (`x86_64-macos.10.12-none`) and fails
  on a macOS target without `MACOSX_DEPLOYMENT_TARGET`. The x86_64 binaries
  carry `LC_VERSION_MIN_MACOSX` 10.12 and the arm64 ones `LC_BUILD_VERSION`
  with `minos 11.0`; the artifact test checks each against its tag.
- **Apple frameworks.** zig has none, so linking any `-framework` fails. Two
  sets of crates in the macOS dependency trees linked one:
  - `security-framework` and `core-foundation`, through `rustls-native-certs`,
    which tokio-tungstenite's `rustls-tls-native-roots` feature turned on.
    BACnet/SC never used the system's root certificates (it runs its own
    tokio-rustls handshake against its configured trust anchors), so the
    workspace dropped tungstenite's TLS features, and the CLI links no
    framework at all.
  - `serialport` (through `tokio-serial`, for MS/TP, which the Python package
    enables on every platform) links IOKit and CoreFoundation, through
    `io-kit-sys` and `core-foundation-sys`. The wheels keep MS/TP:
    [`scripts/release/macos-frameworks`](../scripts/release/macos-frameworks)
    holds a text stub (`.tbd`) for each of the two frameworks, with only its
    install name and versions and no symbols, which the wheel jobs pass to the
    linker with `-F`. The extension module then has a load command for each
    framework, and leaves its references to them, like those to Python's C
    API, to a flat lookup when it's loaded: maturin links macOS extension
    modules with `-undefined dynamic_lookup`. The stubs contain nothing from
    Apple's SDK.

  The artifact test checks that an extension module loads only libSystem,
  libiconv, libcharset, IOKit and CoreFoundation, and that every symbol it
  leaves to a flat lookup is either Python's C API (`_Py*`) or one of the 82
  CoreFoundation and IOKit symbols listed in `check_artifacts.py`, with that
  framework loaded. The list is exactly what the 0.12.0 wheels use, so a new
  symbol, or a lookalike from another framework such as `_CFNetwork*`, fails
  until someone reviews it and adds it. A CLI binary may load only the first
  three libraries and may leave nothing to a flat lookup. libcharset comes from
  cargo-zigbuild's libiconv stub and is part of macOS. Every arm64 file must
  carry a code signature (`LC_CODE_SIGNATURE`), which macOS requires on Apple
  Silicon; zig signs ad hoc, as Apple's linker does.
- **AWS-LC** (`aws-lc-sys`, for BACnet/SC) builds with zig's clang for both
  architectures with its default builder, assembly included. It needs no
  CMake, no bindgen (its bindings for both targets are pregenerated) and no
  `AWS_LC_SYS_NO_ASM`.

**Windows** (`x86_64-pc-windows-msvc`) uses cargo-xwin for the CLI and
maturin's built-in xwin, the same cargo-xwin 0.23.1, for the wheels.

- **CRT and SDK.** The image holds Microsoft's CRT (MSVC 14.44.17.14) and
  Windows SDK 10.0.26100 for x86_64 in `/opt/xwin` (630 MB), put there by xwin
  0.10.0 from a pinned Visual Studio 2022 17.14.41 channel manifest; xwin
  checks each download's SHA-256 against the manifest. Building the image
  accepts Microsoft's licence terms for them, which the owner accepted, and the
  image stays in the private registry. `XWIN_CACHE_DIR=/opt/xwin` and the
  `DONE` file, which lists the architectures cargo-xwin has, stop cargo-xwin
  and maturin from downloading a copy of their own, and the Windows jobs fail
  if a build changed anything under `/opt/xwin/xwin` or used cargo-xwin's
  default cache, so a build can't quietly fetch an unpinned CRT or SDK. To move to a newer CRT or
  SDK, update `VS_CHANNEL_URL` and `VS_CHANNEL_SHA256` in the Dockerfile
  (`curl -sI https://aka.ms/vs/17/release/channel` shows the current URL), and
  `MSVC_CRT_VERSION` and `WINDOWS_SDK_VERSION` (`xwin --accept-license
  --manifest <file> list` shows what a manifest offers).
- **Toolchain.** clang-cl is apt's clang 18, lld-link is rust-lld, and
  llvm-lib is llvm-tools' llvm-ar; cargo-xwin links the last two. AWS-LC
  compiles with clang-cl, and nasm assembles its x86_64 assembly from source,
  so the prebuilt NASM objects that `aws-lc-sys` ships aren't used.
- **Python.** PyO3 0.29 links each extension module to its `pythonXY.dll`
  with raw-dylib, so no import library is needed. The artifact test checks
  that each wheel's `.pyd` imports its own Python's DLL and no other.
- **C runtime.** The CLI links it statically (`-C target-feature=+crt-static`;
  cargo-xwin then links `libucrt` instead of `ucrt`), so it imports only
  Windows system DLLs and needs no Visual C++ Redistributable, which 0.11.0's
  did (`VCRUNTIME140.dll`). The wheels link it dynamically, like other
  extension modules: Python for Windows ships `VCRUNTIME140.dll`, and the
  Universal CRT (`api-ms-win-crt-*`) is part of Windows 10 and later. The
  artifact test enforces both.
- **No PDB.** Both link with `/DEBUG:NONE` and `/Brepro`: the release ships
  no PDB, and without one the Windows files rebuild identically (see
  [Re-running a partial release](#re-running-a-partial-release)).

### Third-party notices

`scripts/release/third_party_notices.py` writes `THIRD-PARTY-NOTICES`: Rusty
BACnet's own licence, then every third-party component in the release
binaries with the licence files it ships, identical texts printed once.

- The crates come from `cargo tree --locked --offline -e normal,no-proc-macro`
  for the CLI (`-p bacnet-cli --features sc-tls,pcap` on Linux, `--features
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
  list is empty: every such crate ships a licence file. One crate ships none
  and is listed at the end with its reason: `clipboard-win` (BSL-1.0, in the
  Windows CLI through rustyline), whose licence exempts machine-executable
  object code.
- libpcap's licence and version come from `/opt/libpcap` in the CI image.
- The file depends only on `Cargo.lock`, the crate sources and libpcap, so a
  rebuild writes the same file.

It's attached to each release, and `pyproject.toml`'s `license-files` puts it
in each wheel's `.dist-info/licenses/` and in the sdist; the artifact test
checks both. Local builds have no such file, and maturin skips it.

cargo-audit and cargo-deny don't cover libpcap, so its advisories need
tracking by hand: watch the [tcpdump/libpcap
releases](https://www.tcpdump.org/) and their security fixes, and bump
`LIBPCAP_VERSION` in the Dockerfile and `LIBPCAP_VER` in `release.yml`
together.

### Re-running a partial release

Every publish job skips what's already there: crate versions on crates.io,
files on PyPI, and a release that is already published. A draft is resumed as
[above](#draft-then-publish). The smoke job stages the tag's GitHub draft
again on every run, replacing any file that differs from the run's, and tests
it before anything is published; the GitHub copy publishes only that staged
draft.

Forgejo deletes all of a run's artifacts whenever any of its jobs is re-run, so
re-running only a failed publish job would find nothing to upload. To finish a
release, for example after a network failure, use **"Re-run all jobs"** on the
tag's run, or dispatch the workflow on the tag with `dry_run=false`. Both
rebuild and test everything before the publish jobs pick up where they stopped.

Rebuilding a commit has given identical files: two dry runs of `685e23ed`
(runs 79 and 80, 2026-10-01) produced the same SHA-256 for all 12
`release-assets` files. The jobs use the same image, paths and toolchain, with
`SOURCE_DATE_EPOCH` set to the commit time for the sdist and wheels, but
nothing enforces it. If a rebuild ever differed, PyPI would keep the files it
already has, a published release wouldn't change, and `SHA256SUMS` would
still list exactly what each release holds.

With the macOS and Windows builds (#944), two dry runs of `2e393fe4` (runs 89
and 90, 2026-10-01) again produced the same SHA-256 for all 27
`release-assets` files. The Windows files need two linker flags for that,
`-C link-arg=/DEBUG:NONE -C link-arg=/Brepro`: without them, the PE timestamps
and the PDB build ID changed on every build.

"Rebuilds match" means rebuilding a commit's artifacts in the same CI image,
not rebuilding the image. The image tag is content-addressed: it's a hash of
the Dockerfile, and the published image under that tag never changes, so every
run with that tag gets the same tools. Only a Dockerfile change builds a new
image, and that build installs whatever Ubuntu's archive then has for the apt
packages, which can change the release binaries: clang and nasm build AWS-LC
for Windows, and flex and bison generate libpcap's filter parser. They aren't
pinned on purpose: Ubuntu removes superseded package versions from its
archive, so a pinned version would stop the image from building later. The
downloaded tools (rustup, zig, cargo-zigbuild, cargo-xwin, xwin, maturin, uv,
libpcap and the others) are pinned by version and SHA-256, the Rust
toolchains by version, and the Windows CRT and SDK by version, from a Visual
Studio channel manifest pinned by SHA-256 that lists each download's SHA-256.

### Secrets

Repository secrets, each passed only to the step that needs it:

- `CI_IMAGE_TOKEN`: pulls the CI image, as in `ci.yml`.
- `CARGO_REGISTRY_TOKEN`: a crates.io token that can publish new crates and
  update existing ones. 0.12.0 is the first release of `bacnet-endpoint` and
  `bacnet-cli`.
- `PYPI_PUBLISH`: a PyPI API token for `rusty-bacnet`, used as `__token__`.
- `GH_RELEASE_TOKEN`: a fine-grained GitHub token for `jscott3201/rusty-bacnet`
  with Contents read and write and Actions read and write. The GitHub release
  copy publishes with it; Validate's preflight lists releases and creates and
  deletes its disposable draft; the smoke job stages the draft, dispatches
  `release-smoke.yml`, and reads and cancels its run.
- The job's automatic token creates the Forgejo release, makes and deletes the
  preflight's draft, and reads commit statuses. Validate and the Forgejo
  release job declare `contents: write` for when Forgejo honours
  `permissions`.

For a release, Validate fails before any build if `CARGO_REGISTRY_TOKEN`,
`PYPI_PUBLISH` or `GH_RELEASE_TOKEN` is empty; a dry run only warns. The step
sees only whether each is set, never its value.

### macOS and Windows

The Linux runner cross-compiles the macOS and Windows builds (#944), and
GitHub-hosted runners smoke-test them before a release publishes anything
(#951). These checks do not establish hardware qualification, support for
macOS or Windows versions older than the runners', or release readiness.
