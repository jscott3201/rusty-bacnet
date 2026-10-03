---
title: "Support status and evidence"
description: "Read release, development, platform and protocol evidence at the scope actually tested."
---

Rusty BACnet is pre-1.0, with changing APIs and partial conformance coverage. Neither the release nor current development makes a BTL certification or full BACnet conformance claim.

## Choose the version and owner first

The [installation and local tutorials](/rusty-bacnet/start/installation/) describe **v0.12.0**; the operating guides still describe **v0.11.0**. The [development section](/rusty-bacnet/development/overview/) describes **unreleased source**, including shared endpoints, current SC requirements and local Network Number controls. A checkout may still report version 0.11.0; use its commit to identify behavior.

Standalone client, full server, shared endpoint and language binding are different surfaces. The shared endpoint's bounded responder does not acquire the full server's service set. Use the [current transport matrix](/rusty-bacnet/development/transports/) to choose a starting point, then follow its evidence links.

## Keep the evidence source authoritative

The current [machine-readable conformance ledger](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/conformance/bacnet-135-2020.json) owns clause status, notes and test links. The [support summary](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/conformance/support-summary.md), [detailed ledger](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/conformance/standard-135-2020-ledger.md), draft PICS and BIBBs are generated views. This site helps readers navigate those records; it does not maintain a second support-status database.

For released behavior, read the [v0.11 ledger](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/conformance/standard-135-2020-ledger.md). Current evidence must not be projected backwards onto a release artifact.

## Understand what a result proves

| Result | Useful evidence | Still separate |
|---|---|---|
| Build or cross-compile passes | Selected code compiles on that target | Runtime behavior and native-library availability |
| Unit or simulated transport tests pass | Exercised state transitions and failure cases | Actual sockets, frames or physical timing |
| Loopback or isolated wire tests pass | Captured framing and the tested owner/lifecycle | Building-network interoperability and every platform |
| Installed Python tests pass | The tested interpreter and native artifact work together | Other wheels, interpreters and free-threaded builds |
| A ledger row has scoped evidence | The stated clause behavior and controls | Complete object/profile conformance or BTL listing |

A screenshot, test count or single-device demonstration does not establish all-device interoperability or fitness for an operating building. No green badge replaces the stated limits. External ignored fixtures, hardware tests and ordinary CI have different execution requirements.

## Report a current problem

Start with [troubleshooting](/rusty-bacnet/help/troubleshooting/) for release tasks or the relevant development guide. Open a [GitHub issue](https://github.com/jscott3201/rusty-bacnet/issues/new) with revision/artifact, transport, feature set, platform, minimal reproduction and sanitized evidence. Include the exact conflicting documentation link.

Never attach private keys, customer identifiers or unredacted operational captures. Use the project's security-reporting guidance for sensitive findings.
