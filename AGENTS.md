# Rusty BACnet agent guidance

This repository uses focused skills and subagents for research and review. Prefer the installed skills under `.agents/skills/` and custom agents under `.codex/agents/`.

Use local Codebase Memory as the first structural code-intelligence layer. The canonical project is `Users-justin-Development-rusty-bacnet`, rooted at `/Users/justin/Development/rusty-bacnet`; verify it once with `list_projects` and `index_status` when entering a fresh repository source context, then use focused graph tools and check coverage for cited scopes. Codebase Memory is not a durable work ledger. Long-running compliance continuity lives in the ignored `_spec/rusty-bacnet-compliance-execution-plan/{Handoff.md,CURRENT_STATUS.md,NEXT_HANDOFF.md}` documents, with `codex-overnight-status.md` retained as an additional local historical log. Refresh live Git/GitHub state before acting on those snapshots. No external memory service, agent identity, namespace, or team assertion is a prerequisite.

Default behavior:

- Use read-only exploration first.
- Delegate independent research or verification questions in parallel when they materially improve speed or confidence and useful root work can proceed. Handle small, coupled tasks locally; the available roles are a menu, not a required panel.
- Keep subagent prompts scoped and evidence-led.
- Reuse a completed review for its exact immutable target. If the head changes, follow `pr-gate-loop`: both required reviewers acknowledge the new target in the permitted final cycle, reusing unchanged evidence and focusing inspection on repair impact. A clean informational review does not reopen a stopped delivery gate.
- Synthesize findings into a single decision or plan.
- Record commands, files, symbols, and sources used as evidence.
- Keep PRs small: one BACnet layer, state-machine family, or measured hotspot per PR.
- Do not add new CI unless explicitly requested.
- Do not broaden README or public support claims without conformance ledger rows, tests, and evidence.
- Use `_spec/rusty-bacnet-compliance-execution-plan/` for the current roadmap: read the current block of `NEXT_HANDOFF.md`, then relevant `CURRENT_STATUS.md`, `WORK_QUEUE.md`, `DECISIONS.md`, and packet/gate files. Historical blocks preserve evidence, not current instructions. The older `_spec/rusty_bacnet_compliance_specs_v1/` path is absent in this checkout. Locate the licensed Standard through `STANDARD_NAVIGATION.md` before protocol work; never infer a missing source.


## Codex CLI and Astra

- Model and effort settings belong in `.codex/config.toml` and named-agent TOMLs. Use the advertised model catalog and role settings; do not substitute a model or raise every task to maximum effort. This repository selects Astra for root work and demanding review, with lighter roles for bounded mapping and source lookup.
- Batch independent tool reads; keep dependent operations and Git/GitHub mutations sequential. Use the collaboration tools and context controls actually exposed by the running CLI. Supply a compact task packet instead of full history when supported.
- Each packet names the objective, exact revision, paths/symbols, ownership, constraints, acceptance evidence, and expected result. Children do not delegate or rediscover the Codebase Memory project. The root resolves conflicting evidence and retains delivery authority.
- Keep a small active team, usually two or three independent readers. The configured limit is a ceiling, not a target. Reuse compatible agents; release finished agents with the available lifecycle tool. Coordinate builds and tests to avoid duplicate Cargo work, shared fixtures, and port conflicts. Run performance measurements without competing workloads.
- Run `cargo clean` between PRs. The root owns this boundary step: finish validation/review work, ensure no Cargo or rustc job is using the shared target directory, then clean that target before the next PR's build work. Preserve Cargo registry/download caches and never clean during another agent's build or tests.
- Ordinary product delivery uses one `implementer`. This repository permits up to two product writers only after an explicit `$parallel-portfolio` invocation and that skill's admission gate: disjoint ownership and contracts, isolated worktrees from one base, and root-owned integration. General requests for speed or parallel research do not admit a portfolio. If the skill is unavailable, continue serially.
- Use the existing `pr-gate-loop` for delivery review; specialist reviews supply bounded evidence, not an extra approval cycle. Keep its repair limits and stopped-gate history intact. Standing merge authority applies only after the current delivery gate passes.
- Validate configuration changes against the installed CLI and start a fresh CLI session to load changed settings and agent definitions. Do not restart another session or assume this running thread has reloaded them.

Recommended skills:

- `$codebase-research-pass`
- `$external-source-research`
- `$spec-contract-compliance-review`
- `$architecture-design-review`
- `$multi-agent-pr-review`
- `$performance-ab-benchmark-review`
- `$release-readiness-review`

BACnet compliance reviewer panel:

- `bacnet-reference-researcher`
- `bacnet-ip-bvll-reviewer`
- `bacnet-sc-security-reviewer`
- `bacnet-tsm-network-reviewer`
- `bacnet-services-objects-reviewer`
- `bacnet-data-link-reviewer`
- `bacnet-performance-reviewer`
- `bacnet-safety-interop-reviewer`
- `bacnet-pr-packager`
