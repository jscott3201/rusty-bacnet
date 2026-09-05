---
name: external-source-research
description: Use when implementation or review depends on external documentation, standards, protocols, APIs, SDKs, changelogs, or version-specific behavior. Produces a concise source dossier and implementation implications.
---
# External Source Research

## Trigger

When the task mentions current docs, specifications, protocol behavior, framework/library APIs, cloud provider behavior, compliance text, or anything that may have changed recently.

## Non-goals

Do not edit code. Do not quote long copyrighted passages. Do not treat blog posts as authoritative when primary sources exist.

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

- `external-researcher` — Find primary sources, versions, changelogs, migration notes, and authoritative examples.
- `spec-tracer` — Convert normative or contractual requirements into testable assertions.
- `codebase-mapper` — Map where the external behavior is consumed or implemented in the codebase.
- `correctness-reviewer` — Check whether current implementation assumptions match the researched sources.

## Workflow

1. Clarify the exact external dependency, version, standard section, API endpoint, or behavioral claim being verified.
2. Prefer primary sources: official docs, standards, RFCs, release notes, code references, or vendor APIs.
3. Extract only actionable facts: version constraints, required behavior, deprecated behavior, edge cases, examples, and test obligations.
4. Map facts to repository usage points and identify mismatches or stale assumptions.
5. Return a source dossier with confidence and citations/links where the environment supports them.

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
# Source dossier

## Research question

## Sources consulted
| Source | Version/date | Authority | Relevant finding |
|---|---:|---|---|

## Requirements / facts
| ID | Fact or requirement | Source | Applicability | Confidence |
|---|---|---|---|---|

## Codebase implications
| Requirement ID | Files / symbols | Current behavior | Gap / action |
|---|---|---|---|

## Unknowns

## Recommended next steps
```

## Local references

Use [output contracts](references/output-contracts.md) and the [severity rubric](references/severity-rubric.md) when useful. For delivery review, the active `pr-gate-loop` target, severity vocabulary, and stopping rules take precedence.
