# Website audit for 0.15.0

Status: decided (2026-10-25), part of #1989. The maintainer approved the sitemap
in section 7 as written and chose to run the loopback lab's Python tests in
the existing Python bindings CI job (section 8). Audited tree: `dev` at
`d7df2c954`, after the navigation split (#1995), the Python first-read
execution test (#1996), the Rust first-read validation (#2010) and the
runnable shared-endpoint guide (#2014). This note records what the site holds,
how it divides work with `docs/`, where it is thin, how far each code sample is
verified, and a sitemap and PR sequence for the restructure. It builds on the
two audit comments on #1989 of 2026-10-21
([part 1](https://github.com/jscott3201/rusty-bacnet/issues/1989#issuecomment-6318838606),
[part 2](https://github.com/jscott3201/rusty-bacnet/issues/1989#issuecomment-6318839533),
at revision [`714497ec`](https://github.com/jscott3201/rusty-bacnet/commit/714497ec8621262366db86a99cb351f3dc097274))
and the sitemap direction the maintainer
[approved the same day](https://github.com/jscott3201/rusty-bacnet/issues/1989#issuecomment-6318941536); it does not repeat their evidence.

Nothing here is a support claim. Spec references are clause numbers only.

## 1. Summary

- **The site is accurate for v0.14.0.** Every CLI sample that can run against
  loopback ran against the v0.14.0 macOS arm64 release binary and against a
  `dev` build with the same results. All 74 links into the repository (count at `d7df2c954`) resolve,
  anchors included. Every Python keyword and Rust item the pages name exists in
  the 0.14.0 stub and on `dev`. One stale statement was fixed (section 9).
- **Only five displayed sources (nine blocks, counting their run lines and
  expected output) are executed by CI.** The Rust first read and its manifest,
  the shared-endpoint program and its manifest, and the Python first read run
  in the workspace and Python CI jobs. The loopback lab's tests exist but no
  workflow runs them. Everything else is parse-checked or reviewed by hand.
- **The 0.15 changes on `dev` touch about a dozen pages.** The MSRV moves from
  1.93 to 1.95, server policy gains a DCC decision and certificate accessors,
  routed targets reach ten more one-shot commands (#1786), `--passive` discovery listens
  for announcements, the baud scan ships, and Python gains a B/IP `BACnetRouter`.
  Section 4 lists each page's change.
- **The biggest gaps are tasks, not facts.** There is no page for serving
  objects, routing, authorizing requests, deploying across subnets, NAT,
  multicast, IPv6, MS/TP real-time scheduling or performance. The facts exist,
  scattered across the transport reference, the upgrade guide and `docs/`.
- **The restructure is cheap to make safe.** `src/data/legacy-routes.json`
  already generates compatibility pages, section anchors and raw exports for
  moved routes, and an end-to-end test checks each one.

## 2. How the site and `docs/` divide content today

The split is written down in three places and holds in practice:
`docs/README.md` says the site gives the release's task path and `docs/` holds
the engineering contracts; `development/documentation.md` says not to copy
large API tables onto the site; and the site links to versioned `docs/` files
instead of restating them.

Two things blur it:

1. **The reference role sits in hand-written Markdown.** `reference/api.md` and
   `integrate/overview.md` send readers to `docs/rust-api.md` (9,713 lines) and
   `docs/python-api.md` (5,902 lines) for exact signatures. Those files mix
   signatures, which rustdoc and the `.pyi` stub already carry, with lifecycle,
   error and cancellation contracts that no generated output carries.
2. **Policy contracts have no task page.** `docs/mutation-policy.md`,
   `docs/dcc-policy.md` and `docs/time-sync-policy.md` define how a server
   decides; the site mentions `ServerPolicy` only in the upgrade guide and the
   support summary.

### Proposed rule

| Layer | Owns | Does not own |
|---|---|---|
| Website | User journeys: install, connect, read, write, serve, route, embed, operate, troubleshoot, evaluate. Each page has one task, runnable samples and links to the exact reference. | Signature tables, option lists, normative ordering rules |
| rustdoc (docs.rs, versioned) | The Rust reference: every public item, feature gates, examples as doctests | Cross-crate journeys |
| `rusty_bacnet.pyi` (and a page generated from it) | The Python reference: signatures, keywords, defaults, docstrings | Lifecycle prose too long for a docstring |
| `docs/*.md` | Engineering contracts: policy order, budgets, audit delivery, qualification methods, CI evidence, design notes | Tutorials and a second copy of signatures |
| clap help (and a page generated from it) | CLI syntax, options and defaults | Session behaviour, safety and workflows, which stay authored on the site |

Under this rule `docs/rust-api.md` and `docs/python-api.md` shrink over time to
the contracts that a signature cannot express. That migration is its own
stream and outside the site PRs; the site only changes where it links.

### Overlap between `docs/` and the site

| `docs/` file(s) | Site pages that summarize it | Proposed relationship |
|---|---|---|
| `rust-api.md` | start/rust, integrate/*, guides/bacnet-sc, reference/transports, reference/api, project/upgrading | Site links to rustdoc for items and to this file only for contracts |
| `python-api.md` | start/python, start/local-lab, guides/*, integrate/shared-endpoints, reference/api | Site links to a generated stub page; contracts stay here |
| `CLI.md` | reference/cli, guides/*, tui/* | Generated CLI reference replaces the option lists; session and safety prose stays authored |
| `architecture.md` | reference/api | Linked from a new contributor architecture page |
| `ci.md` | development/*, start/embedded | Linked, not summarized |
| `mutation-policy.md`, `dcc-policy.md`, `time-sync-policy.md`, `local-access-authorization.md` | project/support, project/upgrading, guides/bacnet-sc | New security task page links each one |
| `request-admission.md`, `rpm-budget.md`, `read-range-budget.md`, `server-segmentation.md`, the five file/alarm/event budgets | integrate/shared-endpoints (admission only) | New server resource-limits section links them |
| `target-audit-reporters.md`, `device-audit-recipient.md`, `delayed-target-audit.md`, `audit-log-forwarding.md` | integrate/shared-endpoints | Optional audit task page |
| `encoding-sinks.md` | start/embedded | Linked from the embedded pages |
| `mstp-qualification.md` (+ result JSON) | guides/mstp, reference/transports | Linked from the MS/TP pages; never turned into a hardware claim |
| `sc-check.md`, `sc-offline-inspection.md` | tui/bacnet-sc, project/upgrading | New offline credential section on the SC page |
| `design/tui.md` | tui/* | Design record only |

## 3. Page inventory

43 files: the landing page, the 404 page, 35 navigation guides and 6
compatibility pages for moved routes. Audience codes: **Ev** evaluator,
**R** Rust application developer, **Py** Python developer, **Op**
integrator or operator using the CLI or TUI, **Em** embedded developer, **C**
contributor. The "0.15 Δ" (delta) column names what the page needs when the site moves to 0.15;
"none" means the page is correct for both.

### Start

| Path | Title | Audience | Purpose | `docs/` overlap | 0.15 Δ |
|---|---|---|---|---|---|
| `index.mdx` | Rusty BACnet — start locally, build confidently | all | Choose a first task | — | Banner series via `site.mjs`; add evaluator and embedded entry points |
| `404.md` | Page not found | all | Recover | — | none |
| `start/choose-your-path.md` | Choose your starting point | all | Pick CLI, Python, Rust or firmware | — | MSRV 1.93 → 1.95; hard-coded 0.14.0 |
| `start/installation.mdx` | Install the tools | Op, Py, R | Install and check each artifact | `ci.md#artifacts` | MSRV text; crate list if a crate is added |
| `start/local-lab.mdx` | Try a local client/server lab | Py, Op | Loopback read without hardware | `python-api.md` | Rerun lab tests on the new wheel |
| `start/first-read.md` | Your first read | Op | Read one property with the CLI | `CLI.md` | none for the samples; version text |
| `start/python.md` | Read with Python | Py | Complete async first read | `python-api.md`, `.pyi` | Watch the error-type PR (#1815): the script catches `BacnetError` |
| `start/rust.md` | Embed the Rust client | R | Complete Rust first read | `rust-api.md` | MSRV; pinned versions; watch the error-type PR (#1815) |
| `start/embedded.md` | Run on a microcontroller | Em | Crates, clock, sizes, adapter rules | `encoding-sinks.md`, `ci.md` | Sources link to `dev`, not the release; sizes follow `size-budget.toml` (unit-tested) |

### Build and integrate

| Path | Title | Audience | Purpose | `docs/` overlap | 0.15 Δ |
|---|---|---|---|---|---|
| `integrate/overview.md` | Build and integrate | R, Py | Choose client, server, endpoint | `rust-api.md`, `python-api.md` | Add router owner (Rust `BACnetRouter`, Python `BACnetRouter`, #1858) |
| `integrate/shared-endpoints.mdx` | Compose a shared endpoint | R, Py | One transport, two roles | `rust-api.md#bacnet-endpoint`, `request-admission.md`, audit docs | MSRV in "minimum Rust version is 1.93" |
| `integrate/network-ports.md` | Network Port and Number controls | R, Py | Register a port; passive Number learning | `rust-api.md` | none known |

### Operate

| Path | Title | Audience | Purpose | `docs/` overlap | 0.15 Δ |
|---|---|---|---|---|---|
| `guides/configuration.md` | Configure your connection | Op, Py, R | Local vs remote settings, units | `CLI.md` | none known |
| `guides/discovery.mdx` | Discover devices deliberately | Op, Py | Directed, ranged, foreign, routed discovery | `CLI.md` | Routed targets now reach `alarms`, `read-range`, `create-object`, `delete-object`, `ack-alarm`, `control`, `reinit`, `time-sync`, `file-read` and `file-write` (#1786); line 42 lists only the original five |
| `guides/bacnet-sc.md` | Configure BACnet/SC | Op, Py, R | Credentials, identity, failure stages | `CLI.md`, `python-api.md`, `rust-api.md` | Certificate subject and SAN accessors for policy (#1965) |
| `guides/mstp.mdx` | Commission an MS/TP lab | Op, Py, R | Serial lab, adapter options | `mstp-qualification.md` | none known; real-time detail belongs on a new page |
| `guides/observe-changes.md` | Observe changes with COV | Op, Py | Bounded subscriptions | `CLI.md`, `python-api.md` | none known |
| `guides/safe-writes.md` | Write and relinquish deliberately | Op | Inspect, write, relinquish, write guard | `CLI.md` | none known |

### Terminal UI

| Path | Title | Audience | Purpose | 0.15 Δ |
|---|---|---|---|---|
| `tui/overview.mdx` | Use the terminal UI | Op | Install, open, layout, quit | none |
| `tui/profiles.mdx` | Profiles and safety | Op | Profiles, attestation, session levels | `--passive` now listens (#1850) |
| `tui/discovery.mdx` | Discover devices | Op | Who-Is, table, routed devices | Passive discovery and local `:find` (#1850) |
| `tui/browser.mdx` | Browse a device | Op | Panes, flags, priority arrays | none |
| `tui/writes.mdx` | Write safely | Op | Arm, preview, confirm, ledger | none |
| `tui/watch.mdx` | Watch points | Op | COV and polling, CSV | Initial COV retry (#1792) if user-visible |
| `tui/network.mdx` | Inspect BBMDs and routers | Op | FD panel, BDT/FDT, routers | none known |
| `tui/bacnet-sc.mdx` | Diagnose BACnet/SC | Op | Dial checks, doctor, certificates | none known |
| `tui/mstp.mdx` | Monitor and join MS/TP | Op | Monitor, safe join, participant | Baud scan becomes released: drop the "Development" labels (#1873) |
| `tui/keys.mdx` | Keys, commands and flags | Op | Reference for keys, commands, flags | Same as above; `--passive` description |
| `tui/troubleshooting.mdx` | Troubleshoot the terminal UI | Op | Terminal problems | none |

### Reference, help, project and contributing

| Path | Title | Audience | Purpose | `docs/` overlap | 0.15 Δ |
|---|---|---|---|---|---|
| `reference/cli.md` | CLI reference entry points | Op | Command families, exit codes | `CLI.md` | Routed targets for the ten commands of #1786 (line 44 lists five); replace with generated reference |
| `reference/transports.mdx` | Transport paths and boundaries | all | Choose a link; evidence scope | `rust-api.md`, `mstp-qualification.md` | Split: selection moves to a connect page, evidence stays |
| `reference/api.md` | API and example library | R, Py, Em | Find exact signatures | `rust-api.md`, `python-api.md`, `architecture.md` | Versioned docs.rs links; list all nine Python examples |
| `help/troubleshooting.md` | Troubleshoot by symptom | all | Isolate the failing layer | — | Routed-target sentence at line 28 (#1786); link policy denials |
| `project/support.md` | Support status and evidence | Ev | What is supported, what evidence means | — | DCC policy, SAN accessors, Python `BACnetRouter`, routed CLI reads, access authorization (#1569–#1572); generated conformance tables per #1496 (decided: option C) |
| `project/upgrading.md` | Upgrade from v0.13.0 to v0.14.0 | all | Breaking changes by area | changelog | Becomes history; a 0.14 → 0.15 guide is needed |
| `development/build-from-source.md` | Build from source | C | Checkout, CLI and Python builds | `ci.md` | none |
| `development/documentation.md` | Improve the documentation | C | Writing and validation rules | `website/README.md` | none |
| `development/{overview,shared-endpoints,network-number,transports,bacnet-sc}.md`, `project/contributing.md` | Moved: … | — | Compatibility pages from #1995 | — | Retarget when their targets move again |

## 4. Freshness against the 0.15 development API

The site deliberately describes the release, so none of the items below is an
error today. They are the work list for the release PR that moves `site.mjs` to
0.15.0, grouped by source change:

| Change on `dev` | Pages |
|---|---|
| Edition 2024 and MSRV 1.95 (#1011) | start/choose-your-path, start/installation, start/rust, integrate/shared-endpoints |
| `ServerPolicy` decides DCC by verified principal (#1964); policy contexts expose the verified leaf certificate, its subject and SANs (#1965) | guides/bacnet-sc, project/support, integrate/security (fold its "On `dev` after the release" section into the page, and pin `website/examples/rust/sc-operator-policy/Cargo.toml` and its test `sc_operator_policy_source` to the release instead of the `dev` branch) |
| `TransportProvenance` no longer `Copy` (#1965); `router.stop()` returns `Result` (#2002); `OriginateError` changes (#1809); `MstpMonitor::start` is async (#1854) | 0.14 → 0.15 upgrade guide |
| CLI `alarms`, `read-range`, `create-object`, `delete-object`, `ack-alarm`, `control`, `reinit`, `time-sync`, `file-read` and `file-write` take routed targets (#1786) | guides/discovery.mdx:42, reference/cli.md:44, help/troubleshooting.md:28 (each lists only `read`, `readm`, `write`, `writem`, `subscribe`); integrate/router (fold its dev-section bullet into the routed-target paragraph) |
| TUI passive discovery and local `:find` (#1850); baud scan (#1873) | tui/profiles, tui/discovery, tui/keys, tui/mstp |
| Python `BACnetRouter` with `BipRouterPort` (#1858); `router.ports()` and the `Result` from `router.stop()` (#2002); access rights and local authorization (#1569–#1572) | integrate/overview, project/support, integrate/router (fold its "On `dev` after the release" section into the page, and pin `website/examples/rust/bip-router/Cargo.toml` and its test `bip_router_source` to the release instead of the `dev` branch) |
| In flight: the error-type PR (#1815, Rust and Python error types) and the B/IP follow-ups PR | start/python, start/rust, guides/discovery, NAT and multicast pages |

The Rust and shared-endpoint samples compile against `dev` in CI today, so a
breaking API change fails there first. The Python first read runs against the
`dev` extension in CI's Python job, so an error rename in #1815 would fail it
too. That is the intended behaviour of #1996 and #2010.

## 5. Thin or missing coverage

| Area | What the site has | What is missing | Sources to build from |
|---|---|---|---|
| **Security** | One paragraph in guides/bacnet-sc ("transport identity is not operation authority"); a migration list in project/upgrading; one line in project/support | A task page: what `ServerPolicy` decides (mutation, LifeSafetyOperation, DCC, audit notifications), `FnPolicy` composition, defaults (allow; a panicking policy denies), `principal()` vs `forwarding_peer_principal()`, the certificate and SAN accessors that survive renewal (#1965), why hub-relayed and routed requests carry no principal, SC admission and control-rate limits, and what Python exposes (`mutation_policy="permissive" | "deny_all"`, no principal callbacks) | `docs/mutation-policy.md`, `docs/dcc-policy.md`, `docs/time-sync-policy.md`, `bacnet_server::policy`, `bacnet_transport::port::TransportProvenance` |
| **Across subnets, NAT, multicast** | A dense paragraph in reference/transports; table rows; upgrade-guide bullets; one discovery section | Procedures with a topology, prerequisites, the commands or builder calls for each surface, and what success and failure look like: foreign device and BBMD (Annex J.4–J.5), BBMD behind NAT (J.7.5), B/IP multicast (J.8) including the one-BBMD-per-group rule | `reference/transports.mdx`, `examples/docker/nat`, `BbmdOptions`, Python `Bbmd`/`Multicast`, CLI `--bip-multicast` |
| **BACnet/IPv6** | One reference section on interface selection | Setup: VMAC naming, `resolve_vmac`, foreign mode, Network Port registration | `reference/transports.mdx`, `python-api.md#bacnetipv6` |
| **MS/TP real-time** | A paragraph in guides/mstp; a CLI section in tui/mstp | One page: `MstpThreadConfig`, `SCHED_FIFO` priority and CPU pinning, privileges, the latency timer, the lateness histogram, how to compare runs, and what remains unqualified | tui/mstp, `MstpExecutionMode`, `docs/mstp-qualification.md`, #502 |
| **Performance** | Nothing, apart from one figure in project/upgrading | An evaluator page: what is measured, units, environments, how to reproduce, what a budget is versus a measurement | `Benchmarks.md` (sections 1, 2, 10, 11) |
| **Serving objects** | A table row in integrate/overview | A full-server task in Rust and Python: database, objects, Device identity, start and stop, resource limits | `rust-api.md`, `python-api.md`, `examples/python/bip_client_server.py` |
| **Routing** | Nothing beyond the embedded router example | Rust `BACnetRouter` and Python `BACnetRouter` tasks with lifecycle (#2002) | `bacnet-network`, `rust-api.md`, #1858 |
| **Upgrade 0.14 → 0.15** | None yet | A guide in the shape of the 0.13 → 0.14 page, written once the error-type PR (#1815) and the B/IP follow-ups PR land | `changelog.d/*-migration.md` |
| **PICS and conformance** | One paragraph in project/support | Generated capability tables and a sample PICS, decided in `docs/design/conformance-reporting.md` (#1496) | `bacnet_server::pics` |

## 6. Code samples

67 displayed blocks, counted as 58 Markdown fences (42 shell, 7 text, 5
PowerShell, 2 TOML, 1 Rust, 1 Python) plus 9 Starlight `<Code>` components
(4 shell, 1 PowerShell, 2 TOML, 1 Rust, 1 Python). Three of the components show
files imported with `?raw`: the shared-endpoint manifest and program from
`website/examples/rust/shared-endpoint/`, and the lab from
`website/examples/python/loopback_lab.py`. Totals: 46 shell, 7 text, 6
PowerShell, 4 TOML, 2 Rust, 2 Python. Every shell block passes `sh -n`
with the release substituted.

**How this audit ran them (2026-10-25, macOS arm64):** a CPython 3.14 venv
with `rusty-bacnet==0.14.0` from PyPI; the loopback lab in `--serve` mode as
the target; the v0.14.0 `bacnet-macos-arm64` release binary (checksum
verified with the page's own command) and a `dev` build with
`sc-tls,serial`. Device addresses were replaced with the lab's loopback
address and port; nothing was sent off the host. Samples that would broadcast
or contact a non-loopback host were run as parse checks: the same arguments
with `--help` appended, which makes clap validate everything before it.

The classes below (A to G) are this audit's labels, from strongest evidence to
none. This table is the state at the audit (`d7df2c954`, 67 blocks); the
[table after the sample harness](#coverage-after-the-sample-harness) is the
current one.

| Status | Count | Blocks (page:line) |
|---|---|---|
| **A. Run or compiled by a CI test** | 9 | start/rust.md:17 TOML, :31 Rust, :66 run line (`first_read_source`, `first_read`); start/python.md:14, :65 (`test_website_first_read.py`, CI Python job); integrate/shared-endpoints.mdx:34 TOML, :36 Rust, :42 run line, :48 expected output (`shared_endpoint_source`, `shared_endpoint`) |
| **B. Run by a test that no workflow runs** | 4 | start/local-lab.mdx:40, :56 output, :78, :115 (`website/tests/python/test_loopback_lab.py`; 8/8 passed here) |
| **C. Run in this audit only** | 17 | start/first-read.md:14, :25, :37, :45, :54; guides/configuration.md:26; guides/discovery.mdx:14; guides/observe-changes.md:10; guides/safe-writes.md:16, :28, :39, :50 (without `--allow-write` exit 4, with it the request is sent); start/installation.mdx:37, :153, :159; start/embedded.md:72 (both examples); project/upgrading.md:20 |
| **D. Parse check only** | 13 | guides/discovery.mdx:22, :34, :54; guides/bacnet-sc.md:37 (also run with missing files: refused before any socket); guides/mstp.mdx:32 (argparse `--help` with the 0.14.0 wheel); tui/overview.mdx:27; tui/profiles.mdx:68, :103; tui/troubleshooting.mdx:33, :43; tui/mstp.mdx:15, :133, :141 (refused on macOS, as the page says) |
| **E. Checked against source only** | 5 | tui/profiles.mdx:111 TOML (matches the `Profile`/`Bip`/`Safety` serde types, which deny unknown fields); tui/watch.mdx:18, tui/writes.mdx:86, :94 (match the command-bar grammar, `command_bar.rs`); start/installation.mdx:205 TOML (same table as start/rust.md, which is tested) |
| **F. Not run** | 17 | Installs or builds: development/build-from-source.md:12, :25, :37; guides/bacnet-sc.md:31; start/installation.mdx:106, :113, :117; tui/overview.mdx:19; start/rust.md:10. Another OS: start/installation.mdx:57 (Linux). Changes the system: tui/mstp.mdx:168. PowerShell, no `pwsh` here: start/installation.mdx:77, :84, :170, :173; start/local-lab.mdx:47, :85 |
| **G. Illustration** | 2 | integrate/shared-endpoints.mdx:27, start/local-lab.mdx:27 (directory trees) |

Results: every run and parse check in C and D gave the documented outcome on
both binaries. No sample is wrong. Observations for later PRs:

- `python3` on a current macOS may be 3.15, for which there is no wheel; the
  page's "No matching distribution" section covers it, but the first command
  could name a supported interpreter explicitly.
- The three B/IP read samples on the safe-writes page target `av:1`; the lab
  has no Analog Value, so a lab-backed harness needs one. *Done in the sample
  harness PR: the lab serves a commandable `av:1`.*
- The lab's Python tests should join an existing CI job; otherwise class B
  drifts silently. *Done in the sample harness PR.*

### Coverage after the sample harness

Step 3 of section 8 (2026-10-25; run on macOS arm64, and on Ubuntu 24.04 arm64 in Docker). The pages had grown to 101
blocks after the content PRs: 84 fences (59 shell, 10 text, 7 Python, 5
PowerShell, 2 TOML, 1 Rust) and 17 components (5 TOML, 4 Rust, 4 shell, 3
Python, 1 PowerShell).

`website/tests/python/site_samples.py` lists every block, keyed by page and
first line, with its class; `test_site_samples.py` fails on a block with no
entry or an entry with no block. It runs each class A CLI sample against the
loopback lab in `--serve` mode (the server guide's commands against that
guide's own `server_device.py`), with the page's device address replaced by
the server's and loopback bind and broadcast options added. Before a command
starts, the harness refuses any address off 127.0.0.0/8 or `::1`, any host
name or URL, and the SC, MS/TP, IPv6, foreign-device, multicast and remote
network options. Each command's exit status and output are checked: values,
names and units, the write guard's exit 4 without `--allow-write`, and the
server program's lines after each command. Class D samples run with `--help`
appended. Both files run, with `test_loopback_lab.py`, in the Python bindings
job of `ci.yml`, after `maturin develop` and a debug `cargo build -p
bacnet-cli` (12 seconds in that Docker run, on top of the maturin build, which has already
compiled the crates the CLI shares).

| Status | Count | Blocks (page:line) |
|---|---|---|
| **A. Run or compiled by a CI test** | 41 | Run against a loopback server by `test_site_samples.py` (14): start/first-read.md:14, :25, :37, :45, :54; start/installation.mdx:159; guides/discovery.mdx:14; guides/observe-changes.md:10; guides/safe-writes.md:16, :28, :39, :50; connect/bacnet-ip.md:26; integrate/server.mdx:73. Covered by another CI test (27): the lab, start/local-lab.mdx:40, :56, :78, :117 (`test_loopback_lab.py`, now in CI); start/rust.md:17, :31, :66; start/python.md:14, :65; integrate/shared-endpoints.mdx:34, :36, :42, :48; integrate/server.mdx:42, :44, :48, :56, :60, :66; integrate/router.mdx:105, :107, :111, :115; integrate/security.mdx:115, :117, :123, :129 (the `*_source` and program integration tests, and `test_website_*.py`) |
| **B. Run by a test that no workflow runs** | 0 | |
| **C. Run in the audit only** | 2 | start/installation.mdx:37 (needs the release download); start/embedded.md:72 (CI's all-targets clippy builds both examples) |
| **D. Parse check** | 23 | By CI (22): connect/across-subnets.mdx:29, :40, :49; connect/bacnet-sc.md:37; connect/ipv6.mdx:32; connect/mstp-realtime.mdx:71, :101; connect/multicast.mdx:25, :31; connect/nat.mdx:63; guides/discovery.mdx:22, :34, :54; integrate/router.mdx:38; tui/mstp.mdx:15, :133, :141; tui/overview.mdx:27; tui/profiles.mdx:68, :103; tui/troubleshooting.mdx:33, :43. The Linux-only real-time options are expected to be refused elsewhere, as the pages say. By hand (1): connect/mstp.mdx:32 (argparse `--help`; needs a serial port) |
| **E. Checked against source only** | 5 | As at the audit |
| **F. Not run by CI** | 28 | Installs or builds (11): development/build-from-source.md:12, :25, :37; connect/bacnet-sc.md:31; start/installation.mdx:106, :113, :117, :153; start/rust.md:10; tui/overview.mdx:19; project/upgrading/v0-14.md:20 (installation.mdx:153 and the upgrade line ran in the audit, as class C; CI installs nothing from PyPI). Release download for another OS (1): start/installation.mdx:57. Changes the system (2): tui/mstp.mdx:168; connect/mstp-realtime.mdx:56. Benchmarks (2): connect/mstp-realtime.mdx:113; project/performance.mdx:78. PowerShell, and the job is Linux only (6): start/installation.mdx:77, :84, :170, :173; start/local-lab.mdx:47, :85. Python snippets that need real BBMD, multicast or IPv6 peers (6): connect/across-subnets.mdx:60, :95; connect/ipv6.mdx:42; connect/multicast.mdx:41, :66; connect/nat.mdx:29 |
| **G. Illustration** | 2 | As at the audit |

Mutation checks (each broke one thing, saw the named test fail, and restored
it): a misspelt flag or keyword in a run sample, on its first line or a later
one (`--wiat`, `pvx`, `priority-aray`, `--priorty`); a misspelt flag or
subcommand in a parse sample (`--pasive`, `--renwal`, `whois-routr`); an
off-host device address; `--allow-write` dropped from the write samples; the
server guide writing `bo:2`; a new block with no entry; and the lab without
`av:1`.

Referenced but not displayed: the nine `examples/python/*.py` scripts parse;
`mstp_mini_device.py --help` runs; the Rust examples in `start/embedded.md`
build under CI's all-targets clippy. `reference/api.md` lists five of the nine
Python examples.

## 7. Proposed sitemap by user task

**Approved as written by the maintainer on 2026-10-25.** It follows the
direction approved on #1989: keep Start and the TUI, keep
application guides in `integrate/`, keep `development/` for contributors, and
add connection, server, router, security and performance tasks. Existing URLs
stay where the content stays; a page moves only when its folder would mislead.
**New** pages are marked.

- **Start** (`start/`, unchanged): choose-your-path, installation, local-lab,
  first-read (CLI), python, rust, embedded.
- **Connect to a network** (`connect/`):
  - `connect/overview` **new**: choose a data link and owner (from
    reference/transports "Choose a path" and "Match the link to an owner").
  - `connect/bacnet-ip` ← `guides/configuration`.
  - `connect/across-subnets` **new**: foreign device and BBMD.
  - `connect/nat` **new**: BBMD behind NAT.
  - `connect/multicast` **new**: B/IP multicast.
  - `connect/ipv6` **new**.
  - `connect/bacnet-sc` ← `guides/bacnet-sc`, gaining offline credential
    inspection.
  - `connect/mstp` ← `guides/mstp`.
  - `connect/mstp-realtime` **new**.
- **Discover, read and write** (`guides/`, URLs unchanged): discovery,
  observe-changes, safe-writes.
- **Build applications** (`integrate/`): overview, shared-endpoints,
  network-ports (unchanged); `integrate/server` **new**; `integrate/router`
  **new**; `integrate/security` **new**; `integrate/lifecycle-and-errors`
  **new**, after the error-type PR (#1815); `integrate/resource-limits` **new**;
  `integrate/audit` **new**, optional; `integrate/pics` **new**, per #1496: how to generate a PICS for your own
  configuration.
- **Terminal UI** (`tui/`, unchanged): the eleven pages.
- **Reference** (`reference/`): `reference/cli` (generated from clap plus
  authored notes); `reference/api` stays as the hub, with
  `reference/rust` **new** (crate map, versioned docs.rs links) and
  `reference/python` **new** (generated from the stub);
  `reference/transports` keeps the evidence tables.
- **Evaluate and get help** (`project/`, `help/`): help/troubleshooting;
  project/support; `project/conformance` **new**, per #1496: the generated capability tables and
  the sample PICS;
  `project/performance` **new**; `project/upgrading` becomes a hub whose
  current guide is 0.14 → 0.15, with the 0.13 → 0.14 guide at
  `project/upgrading/v0-14` **moved**.
- **Contribute** (`development/`): build-from-source, documentation;
  `development/architecture` **new** (links `docs/architecture.md`);
  `development/checks` **new** (links `docs/ci.md`).

### Old → new URL map

| Old route | New route | Mechanism |
|---|---|---|
| `guides/configuration/` | `connect/bacnet-ip/` | New `legacy-routes.json` entry with every section anchor |
| `guides/bacnet-sc/` | `connect/bacnet-sc/` | Same |
| `guides/mstp/` | `connect/mstp/` | Same |
| `development/bacnet-sc/` (already a compatibility page) | `connect/bacnet-sc/` | Retarget the existing entry so a saved link takes one hop |
| `development/transports/` (compatibility page) | sections split between `connect/overview/` and `reference/transports/` | Retarget per section. `legacy-routes.json` already gives each section its own `target`, and `development/overview` already mixes targets in two sections (`integrate/overview/…` and `development/build-from-source/…`), so the split needs no generator change |
| `project/upgrading/` sections of the 0.13 → 0.14 guide | `project/upgrading/v0-14/#…` | The hub keeps each old heading as an anchor with a link, the pattern the compatibility pages use; the hub itself is a real page, so it cannot be a `legacy-routes` entry |
| `raw/<old>.md` exports | `raw/<new>.md` | Generated by the same entries |
| Every other route | unchanged | — |

**Two pages for conformance, on purpose.** `project/conformance` answers an
evaluator's question (what does the stack support, shown by generated tables
and one sample PICS); `integrate/pics` is a developer task (generate the PICS
of your own configuration with `generate_pics`). They link to each other; the
sample PICS lives only on `project/conformance`.

**The release test must follow the upgrade move.** `website/tests/unit/release.test.mjs`
lets only the pages in its `history` set (line 20) name the previous release,
and checks the 0.13 → 0.14 guide's title at `project/upgrading.md`. The IA PR
adds the new `project/upgrading/v0-14` page to `history`, points the title
check at it, and keeps the hub under the ordinary `{release}` rule; at the
0.15 release, `previous` becomes 0.14.0 and the hub's current guide takes
the title check.

Inbound links to update in the same PR: the repository `README.md`
(`guides/bacnet-sc`, `guides/mstp`, `project/contributing`),
`CONTRIBUTING.md` (`project/contributing`), and internal links on the site,
including `help/troubleshooting` → `#diagnose-the-stage-that-failed`. The
README links to `project/contributing/` keep working through the compatibility
page; switch them to `development/documentation/` only when the restructured
site is published, because the live site still serves the 0.14 tree.

## 8. Proposed PR sequence

Each PR keeps `npm run verify` green, runs spec_overlap over the diff, and adds
a changelog fragment.

1. **IA (navigation and moves).** Apply the map above: `navigation.json`
   groups, the three moves, retargeted compatibility pages, the upgrade hub,
   and inbound links. No prose changes beyond breadcrumbs and next-step
   links. *Acceptance:* every old route, section anchor and raw export
   resolves (the legacy-routes test); the search index holds each canonical
   page once; screenshots of the changed navigation at phone and desktop width
   in both themes.
2. **Content, one PR per area,** in this order: (a) security page; (b)
   across-subnets, NAT, multicast and IPv6, after the B/IP follow-ups PR lands; (c) MS/TP
   real-time; (d) server and router tasks, each with a complete program kept in
   `website/examples/` and run by an integration test, as #2014 did; (e)
   performance page drawn from `Benchmarks.md`; (f) the 0.14 → 0.15 upgrade
   guide, written last from the migration fragments. *Acceptance per PR:*
   every new sample sits in class A or C of section 6, with the evidence in the
   PR; no support claim without a test; facts link to rustdoc or `docs/`
   rather than restating them.
3. **Sample coverage.** A harness that runs the CLI samples against the
   loopback lab (class C becomes A), and `website/tests/python/test_loopback_lab.py`
   run in the existing Python bindings job of `ci.yml` (maintainer decision,
   2026-10-25; no new workflow), which moves class B to A. *Acceptance:*
   breaking a sample's flag or keyword fails a test, and the lab tests appear
   in the Python bindings job's log.
4. **Generated reference.** CLI reference generated from the clap definitions;
   crate map with docs.rs links pinned to the release; Python reference
   generated from the stub; the conformance tables decided in #1496. A unit test fails
   when a generated file is stale. *Acceptance:* no hand-maintained option or
   signature table remains on the site.
5. **Presentation.** Landing page with evaluator, Python, Rust, operator and
   embedded paths; Rust/Python/CLI tabs where a task exists in more than one;
   screenshots refreshed through `capture.py`; mobile, contrast and alt-text
   passes; search tuning. Reduce e2e assertions that pin whole sentences in
   favour of facts tied to code. *Acceptance:* axe clean at four widths in
   both themes; e2e still covers each route, link and export.
6. **Release.** Move `site.mjs` to 0.15.0, apply section 4, and publish from
   the release SHA through `docs-pages.yml`.

## 9. Fixes made in this PR

- `start/local-lab.mdx` said its recorded runs had not been repeated with
  v0.14.0. The lab's tests and script have since passed with the 0.14.0 wheel
  (#1996, and again in this audit), so the evidence paragraph now says so and
  keeps the older Linux run labelled as an earlier release.
- `docs/README.md` said the site's `development/` guides lead to the
  contracts; since #1995 those are the `integrate/` guides.

## 10. Method

- Inventory: every file under `website/src/content/docs`, the navigation and
  `legacy-routes.json`.
- Links: each `github.com/jscott3201/rusty-bacnet/(blob|tree)/<ref>/<path>#<anchor>`
  link checked with `git cat-file` at its ref and against the heading slugs of
  the target Markdown: 74 links at `d7df2c954`, none broken. Site-internal links are covered
  by the existing e2e suite.
- Names: Python keywords and classes checked against
  `crates/rusty-bacnet/rusty_bacnet.pyi`; Rust items against the crate
  sources on `dev`.
- Tests run: `first_read_source`, `first_read`, `shared_endpoint_source`,
  `shared_endpoint` (39 passed); `test_website_first_read.py` (9 passed) and
  `test_loopback_lab.py` (8 passed) on the 0.14.0 wheel.
