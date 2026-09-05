---
name: release-readiness-review
description: Use before merging, shipping, publishing, or handing off work. Runs subagents for verification, regression risk, operational readiness, docs, compatibility, and unresolved follow-ups.
---
# Release Readiness Review

## Trigger

When the user asks if work is ready to merge/release/ship, prepare a release note, check readiness, or assess final risk after implementation.

## Non-goals

Do not claim readiness without verification evidence. Do not hide unresolved risks in prose.

## Operating principles

- Stay evidence-led. Every non-obvious conclusion must be backed by file paths, symbols, command output, issue/PR links, or source references.
- Keep subagents narrow. Give each subagent one question, a bounded search area, and an explicit output contract.
- Prefer read-only exploration until the parent task explicitly asks for edits.
- Separate facts, inferences, risks, and recommendations.
- Do not rely on repository claims alone. Verify against source code, tests, CI, docs, generated artifacts, runtime behavior, and external specifications when available.
- Treat commands as evidence only when their exact command line, environment assumptions, and result are recorded.
- Preserve user intent. Do not widen scope without stating the proposed scope change.
- Be portable across stacks. Infer language, framework, build system, package manager, test runner, and CI from the repository rather than assuming any one ecosystem.

## Subagent orchestration rules

Delegate bounded independent questions in parallel when useful root work can proceed alongside them. Use only the roles needed for the decision; the list below is a menu, not a mandatory panel. Handle small or tightly coupled work locally. Children do not delegate. Use configured models and the running harness's supported collaboration tools.

When spawning subagents, pass this minimum packet:

1. Objective: one sentence describing the question to answer.
2. Scope: exact revision and files/symbols or supplied artifact; include the root-resolved Codebase Memory project/root and coverage notes for repository source work, otherwise mark it not applicable.
3. Constraints: read-only vs write, commands allowed, network/doc lookup allowed, and timeout.
4. Required evidence: exact files/symbols/commands/sources to cite.
5. Output schema: use the schema requested by the skill.

Parent-agent responsibilities:

- Assign non-overlapping scopes when possible.
- Advance independent root work while agents run; collect required results before dependent decisions. Reuse compatible agents and successful exact-target validation; do not duplicate builds or run competing benchmarks.
- Cross-check subagent conclusions against each other.
- Resolve conflicts explicitly. Do not silently choose the more confident subagent.
- Produce one synthesized result with a risk-ranked decision, not a paste of raw subagent notes.

## Recommended subagents

- `test-verifier` — Run or inspect test/build/lint/typecheck/CI gates and summarize what passed, failed, or was skipped.
- `correctness-reviewer` — Review whether implementation meets stated requirements and preserves existing behavior.
- `security-reliability-reviewer` — Review failure modes, secrets, access control, data handling, concurrency, and reliability hazards.
- `release-risk-reviewer` — Assess release notes, migration/rollback plan, observability, config, compatibility, and docs needs.
- `external-researcher` — Check external release/publish requirements or changelog constraints if relevant.

A completed exact-target delivery review and passing gates are reusable evidence. Add a readiness lane only for an unresolved release risk; this skill does not mandate another panel or authorize publication.

## Workflow

1. Clarify the release unit: PR, branch, package, service, version, artifact, or deployment.
2. Collect the intended change summary and acceptance criteria.
3. Run or inspect verification gates; explicitly record skipped gates and why.
4. Assess operational readiness: rollback, migration, observability, compatibility, docs, support impact.
5. Return a release decision with blockers and exact follow-up items.

## Required final output

Return results in this order:

1. **Decision / state** — ready, not ready, inconclusive, or needs follow-up.
2. **Evidence summary** — the smallest set of facts that supports the decision.
3. **Findings** — severity, title, affected area, evidence, impact, and recommendation.
4. **Gaps / unknowns** — what was not verified and why.
5. **Next work items** — ordered, scoped, and testable.

Use severity labels consistently:

- `blocker`: likely correctness, security, data loss, compliance, release, or customer-impact issue.
- `high`: real risk with plausible production impact or major maintainability cost.
- `medium`: important gap, incomplete test, migration risk, or design debt.
- `low`: useful improvement with limited impact.
- `info`: context, observation, or non-actionable note.

## Skill-specific output schema

```markdown
# Release readiness review

## Release unit

## Decision
ready | not ready | ready with risks | inconclusive

## Verification matrix
| Gate | Command/source | Result | Evidence | Notes |
|---|---|---|---|---|

## Blockers
| Severity | Issue | Evidence | Required action |
|---|---|---|---|

## Operational readiness
- Rollback:
- Migration:
- Observability:
- Docs/release notes:
- Compatibility:

## Final checklist
- [ ] ...
```

## Local references

Use [output contracts](references/output-contracts.md) and the [severity rubric](references/severity-rubric.md) when useful. For delivery review, the active `pr-gate-loop` target, severity vocabulary, and stopping rules take precedence.
