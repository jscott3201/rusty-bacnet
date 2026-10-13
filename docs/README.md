# Engineering documentation

These documents follow the `dev` branch. The latest release is **0.13.0**, and the [v0.13.0 documentation tree](https://github.com/jscott3201/rusty-bacnet/tree/v0.13.0/docs) is the copy that matches it. A `dev` checkout may hold later changes while its version still reads 0.13.0, so record the source revision when using these APIs. The [website](https://jscott3201.github.io/rusty-bacnet/) gives a shorter task-oriented path for the release: its `start/` pages install and make a first read, and its `development/` guides lead to these contracts.

## Start with your question

| Question | Canonical document |
|---|---|
| Which crates, owners and packet paths compose the stack? | [Architecture](architecture.md) |
| What is the exact Rust API or feature boundary? | [Rust API](rust-api.md) |
| How do I configure and close a Python owner? | [Python API](python-api.md) |
| Which CLI arguments does the current checkout accept? | [CLI reference](CLI.md) |
| Should I use a shared endpoint or a standalone owner? | [Endpoint roles and scope](rust-api.md#bacnet-endpoint) |
| Is a Network Port registered, or is its number passively learned? | [Registration and local Number controls](rust-api.md#registered-bip-network-port) |
| What does the stack support? | [What's supported](https://jscott3201.github.io/rusty-bacnet/project/support/#whats-supported), on the website's support page |
| Which checks are required before merge? | [CI and merge evidence](ci.md) |

## Policy and resource contracts

- [Mutation authorization](mutation-policy.md), [Device Communication Control](dcc-policy.md) and [time synchronization](time-sync-policy.md).
- [Request admission](request-admission.md), [ReadPropertyMultiple budgets](rpm-budget.md), [ReadRange budgets](read-range-budget.md).
- [AtomicReadFile](atomic-read-file-budget.md), [AtomicWriteFile](atomic-write-file-budget.md), [alarm summary](alarm-summary-budget.md), [enrollment summary](enrollment-summary-budget.md) and [event information](event-information-budget.md) response budgets.
- [Target Audit Reporters](target-audit-reporters.md), [Device Audit recipient](device-audit-recipient.md), [delayed target Audit](delayed-target-audit.md) and [Audit Log forwarding](audit-log-forwarding.md).
- [MS/TP qualification](mstp-qualification.md) separates simulator evidence from serial hardware acceptance.

## Website and engineering docs

The website explains tasks and summarizes what is supported; these references define the engineering contracts. Keep related updates together without duplicating full API tables. Examples and source inspection establish their stated scope, not hardware interoperability or certification.

## Source, issues and contribution

Source, releases and issues are on [GitHub](https://github.com/jscott3201/rusty-bacnet). Report issues on [GitHub](https://github.com/jscott3201/rusty-bacnet/issues) with a revision, transport, platform and sanitized reproduction.

The [website maintenance guide](../website/README.md) covers Astro content, exports and local browser checks. Website validation, Rust runtime evidence, installed Python tests and publication are separate operations.
