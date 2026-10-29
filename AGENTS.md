# Rusty BACnet agent guidance

Rusty BACnet is a BACnet protocol stack (ASHRAE 135-2020) in Rust, with Python (PyO3) bindings, a CLI, a TUI and an MCP server. This file is the entry point for coding agents.

## Start here

1. **Read the local handoff.** It lives at `_spec/roadmap-0.15/README.md` (gitignored, local-only). It holds:
   - project state after 0.14.0;
   - the 0.15.0 roadmap;
   - the open-issue map;
   - pending and taken decisions;
   - gates, gotchas, working patterns and the release process.
2. **Refresh live state before acting on any snapshot:** `git fetch`, `gh run list --branch dev`, `gh issue list --milestone 0.15.0`.
3. **Confirm this session's permissions with the maintainer** (jscott3201). Merge permission is per session. It never authorizes tagging or publishing a release.

The older `_spec/rusty-bacnet-compliance-execution-plan/` documents are historical evidence, not current instructions.


## Repository hosting and CI

- **Hosting:** `origin` is [GitHub](https://github.com/jscott3201/rusty-bacnet), the home for branches, issues, PRs, CI and the project board (users/jscott3201/projects/4).
  - Branch from `dev` and target PRs at `dev`; `main` takes only release merges.
  - The old Forgejo is retired, and its remote is fetch-only.
  - Historical `GitLab #NNN` references refer to a retired tracker.
- **CI on PRs and pushes to `dev`/`main`:**
  - `.github/workflows/ci.yml` runs Linux; its aggregate check is `CI OK`.
  - `.github/workflows/native-tests.yml` runs macOS and Windows; its aggregate check is `Native OK`.
  - **Merge evidence:** both checks green on the PR's exact head SHA ([docs/ci.md](docs/ci.md)).
  - **Skipped dev runs:** on a dev merge that could have been a fast-forward, with no cache-key change, a Plan job skips the heavy jobs (#1466).
- **Checks that run only for `main`:** `Cargo Audit + Deny` and `MSRV` run only on PRs into `main` and on `main` pushes. Run `cargo audit` and `cargo deny check` locally before any release.
- **Tests:** use cargo-nextest 0.9.145 or later (`cargo nextest run`, configured in `.config/nextest.toml`), plus `cargo test --doc`, since nextest skips doctests. `scripts/ci/local-macos.sh` is optional local verification, not merge evidence.

## Project rules that are easy to miss

- **Licensed standard.** The ASHRAE 135 PDFs (standard, addendum, errata) are in `_spec/`.
  - Find pages with `.claude/skills/bacnet-spec-check/scripts/spec_overlap.py find <phrase>`.
  - Paraphrase only, and cite clause or table numbers. Never copy text into code, docs, commits, PRs or issues.
  - Before posting any diff or body, run `spec_overlap.py check --diff <range>`, `check --words 6 …` and `sentences …`. File arguments are positional.
  - Never infer a missing source.
- **Changelog.** Every change adds a `changelog.d/<issue>-<slug>.md` fragment (format in `changelog.d/README.md`). Never edit `CHANGELOG.md`'s `[Unreleased]`. Breaking changes carry a label and a migration fragment.
- **Merge order.** Wait for dev's post-merge `CI OK` and `Native OK` on the previous merge before merging the next PR. Twice in 0.14, dev failed after a green PR. A dev failure is a bug to triage and fix forward, not a re-run.
- **Closing keywords.** "Closes/Fixes #N" anywhere in a PR body closes the issue. Write "Part of #N" for partial work.
- **Hard limits.**
  - Every `.rs` file stays under 700 lines (`scripts/ci/check-file-size.sh`).
  - `missing_docs` is deny, and clippy warnings are errors.
  - `bacnet-server` in-crate tests share one `TestTransport`.
- **Timing tests.** Use Tokio's paused clock, not real sleeps or short real-time budgets.
- **Async review checklist.**
  - Server lock order: `ObjectDatabase` before `COVTable`.
  - No lock held across an await.
  - No unpolled sibling future holding a queued lock waiter (#1494).
  - Futures used in `select!` must be cancel-safe.
- **Releases.** Tagging publishes irreversibly to crates.io, PyPI and GitHub Releases, and needs the maintainer's explicit approval for each release. The flow is in `_spec/roadmap-0.15/09-release-process.md`.
- **Local-only files.** `_spec/` and `CLAUDE.md` are gitignored. Never commit them.

## Default behavior

- Use read-only exploration first.
- **Delegation:** run independent research or verification questions in parallel when that materially improves speed or confidence and useful root work can proceed. Handle small, coupled tasks locally; the available roles are a menu, not a required panel. Keep subagent prompts scoped and evidence-led.
- **Reviews:** reuse a completed review for its exact immutable target. If the head changes, follow `pr-gate-loop`:
  - both required reviewers acknowledge the new target in the permitted final cycle;
  - unchanged evidence is reused, and inspection focuses on the repair's impact.

  A clean informational review does not reopen a stopped delivery gate.
- Synthesize findings into a single decision or plan, and record the commands, files, symbols and sources used as evidence.
- **PR size:** keep PRs bounded. Group closely related issues (one BACnet layer, state-machine family or measured hotspot) when one review can cover them.
- **CI and claims:** don't add new CI unless asked, and don't broaden README or public support claims without tests and evidence.

## Pre-1.0 design and cleanup

- **APIs are unfrozen before 1.0.0.**
  - Prefer a coherent greenfield design, and remove obsolete paths. Don't add compatibility shims, deprecated aliases, duplicate configuration hooks or speculative extension layers just to preserve pre-1.0 callers.
  - Update every in-repository Rust and Python caller, test, example and doc affected by an API change together, and describe the break in the PR.
  - Protocol correctness, required platform support, persisted-data integrity and security boundaries still apply.
- **Scope of each PR:**
  - Include related cleanup in a PR when it strengthens the same bounded outcome. Avoid unrelated rewrites.
  - File deduplicated issues for follow-ups that don't fit, with evidence, dependencies and testable acceptance.
  - Link them from the relevant `_spec/roadmap-0.15/` file.
- **At 1.0.0,** freeze public APIs, and preserve compatibility after that under the release policy. Historical pre-1.0 compatibility decisions in roadmap snapshots don't override this.

## Resources on the shared machine

- **Disk.** A full workspace gate builds 30–80 GB of `target/`.
  - Run one full workspace gate at a time, and keep at least 50 GB free.
  - Use `CARGO_INCREMENTAL=0` for gate runs.
  - Never share a target directory across worktrees.
- **`cargo clean`.** Run it between PRs. The root owns this step: finish validation and review, make sure no Cargo or rustc job is using the target, then clean it before the next PR's build. Keep the registry and download caches, and never clean during another agent's build or tests.
- **Measurements.** Run performance measurements with no competing workload, one benchmark suite at a time.
- **Linux-only code** (MS/TP real-time, AF_PACKET Ethernet, kernel RS-485) is checked in local Docker. See `_spec/roadmap-0.15/06-gates-ci-and-commands.md`.

## Skills

Repository skills live in `.claude/skills/` (gitignored, local-only) and read fine as plain docs:
- `group-pr-workflow` has templates for implementer, reviewer and fix briefs and PR bodies;
- `bacnet-spec-check` has the spec_overlap checker.

**Review lenses.** If your harness defines BACnet specialist reviewer agents, use them by name. Otherwise apply the same lenses with general reviewers:
- reference research;
- B/IP and BVLL;
- BACnet/SC security;
- TSM and the network layer;
- services and objects;
- data links;
- performance;
- safety and interoperability;
- PR packaging.
