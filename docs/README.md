# Engineering documentation

These documents describe the **current development checkout**, including unreleased changes. The workspace version can still be 0.11.0; record the source revision when using these APIs. For the release, use the [v0.11.0 documentation tree](https://github.com/jscott3201/rusty-bacnet/tree/v0.11.0/docs). The Astro site's authored `development/` guides provide a shorter task-oriented path to these contracts; its `start/` tutorials retain their release scope.

## Start with your question

| Question | Canonical document |
|---|---|
| Which crates, owners and packet paths compose the stack? | [Architecture](architecture.md) |
| What is the exact Rust API or feature boundary? | [Rust API](rust-api.md) |
| How do I configure and close a Python owner? | [Python API](python-api.md) |
| Which CLI arguments does the current checkout accept? | [CLI reference](CLI.md) |
| Should I use a shared endpoint or a standalone owner? | [Endpoint roles and scope](rust-api.md#bacnet-endpoint) |
| Is a Network Port registered, or is its number passively learned? | [Registration and local Number controls](rust-api.md#registered-bip-network-port) |
| Which clauses have bounded evidence? | [Support summary](conformance/support-summary.md), [detailed ledger](conformance/standard-135-2020-ledger.md) |
| Which checks are required before merge? | [CI and merge evidence](ci.md) |

## Policy and resource contracts

- [Mutation authorization](mutation-policy.md) and [Device Communication Control](dcc-policy.md).
- [Request admission](request-admission.md), [ReadPropertyMultiple budgets](rpm-budget.md), [ReadRange budgets](read-range-budget.md).
- [AtomicReadFile](atomic-read-file-budget.md), [AtomicWriteFile](atomic-write-file-budget.md), [alarm summary](alarm-summary-budget.md), [enrollment summary](enrollment-summary-budget.md) and [event information](event-information-budget.md) response budgets.
- [Target Audit Reporters](target-audit-reporters.md), [Device Audit recipient](device-audit-recipient.md), [delayed target Audit](delayed-target-audit.md) and [Audit Log forwarding](audit-log-forwarding.md).
- [MS/TP qualification](mstp-qualification.md) separates simulator evidence from serial hardware acceptance.

## Evidence has a single source

[bacnet-135-2020.json](conformance/bacnet-135-2020.json) is the machine-readable conformance authority. The summary, detailed ledger, [draft PICS](conformance/pics-draft.md) and [draft BIBBs](conformance/bibbs-draft.md) are generated views. Change the JSON and run `python3 scripts/generate-conformance-docs.py` from the repository root; use `--check` to verify consistency. A row's `notes` is an array of entries, one topic per line, read as one text joined with single spaces. Add a topic as a new entry instead of lengthening a neighbour, so parallel PRs change different lines; `python3 scripts/ledger_notes_split.py` splits any note still written as one string. The detailed ledger's row tables are generated between `ledger-rows` markers from the `evidence` array of each row a marker lists; `evidence` is temporary, and the #1208 condense batches fold it into the row and delete it. Do not replace clause scope with a blanket support badge or edit generated views independently.

The website explains tasks; these references define the engineering contracts. Keep related updates together without duplicating full API tables or inventing a second evidence ledger. Examples and source inspection establish their stated scope, not hardware interoperability or certification.

## Source, issues and contribution

Source, releases and issues are on [GitHub](https://github.com/jscott3201/rusty-bacnet). Report issues on [GitHub](https://github.com/jscott3201/rusty-bacnet/issues) with a revision, transport, platform and sanitized reproduction.

The [website maintenance guide](../website/README.md) covers Astro content, exports and local browser checks. Website validation, Rust runtime evidence, installed Python tests and publication are separate operations.
