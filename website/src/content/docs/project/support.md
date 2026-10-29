---
title: "Support status and evidence"
description: "What Rusty BACnet supports, and how to read release, development and platform evidence at the scope actually tested."
---

Rusty BACnet is pre-1.0, with changing APIs and partial conformance coverage. It makes no BTL certification or full BACnet conformance claim, and no physical-safety guarantee: writes, device management, time synchronization and file transfers can change real equipment.

## Choose the version and owner first

The [installation and local tutorials](/rusty-bacnet/start/installation/), the [integration guides](/rusty-bacnet/integrate/overview/) (shared endpoints, SC requirements and local Network Number controls) and the operating guides describe **v0.15.0**. A `dev` checkout may hold later changes while it still reports the release's version; use its commit to identify behavior.

Standalone client, full server, shared endpoint and language binding are different surfaces. The shared endpoint's bounded responder does not acquire the full server's service set. Use the [transport matrix](/rusty-bacnet/reference/transports/) to choose a starting point, then follow its evidence links.

## What's supported

The [capability tables](/rusty-bacnet/project/conformance/), generated from the code, list the object types the stack can construct, the services each surface executes, initiates or sends (full server, client, shared endpoint responder and Python client), and the data links each artifact is built with. They track `dev`; the v0.15.0 [release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.15.0) list what that release changed. The tables don't describe behaviour: who may change a device (`ServerPolicy`) is on [Restrict who can change a device](/rusty-bacnet/integrate/security/), the Python classes are in [the Python reference](/rusty-bacnet/reference/python/), the client's segmentation and COV renewal in its rustdoc, the `no_std` crates on [the embedded page](/rusty-bacnet/start/embedded/), and the terminal UI starts at [its overview](/rusty-bacnet/tui/overview/).

**PICS for your device.** A running server can describe itself: `BACnetServer::generate_pics` builds a PICS (`bacnet_server::pics::Pics`, which renders as text or Markdown) from the objects actually in its database and the services the full server executes. `bacnet_server::pics::generate_pics` does the same for a database alone, taking the services from its Device object. Either one describes that configuration; it is not a conformance test result or a BTL listing. [Generate a PICS for your device](/rusty-bacnet/integrate/pics/) shows how and what you fill in yourself, and the [sample PICS](/rusty-bacnet/project/sample-pics/) is the generator's output for one reference configuration, labelled as such.

## Understand what a result proves

| Result | Useful evidence | Still separate |
|---|---|---|
| Build or cross-compile passes | Selected code compiles on that target | Runtime behavior and native-library availability |
| Unit or simulated transport tests pass | Exercised state transitions and failure cases | Actual sockets, frames or physical timing |
| Loopback or isolated wire tests pass | Captured framing and the tested owner/lifecycle | Building-network interoperability and every platform |
| Installed Python tests pass | The tested interpreter and native artifact work together | Other wheels, interpreters and free-threaded builds |
| A generated PICS lists an object or service | What that configured server declares | Complete object/profile conformance or BTL listing |

A screenshot, test count or single-device demonstration does not establish all-device interoperability or fitness for an operating building. For measured latency and throughput, and the conditions behind them, see [performance](/rusty-bacnet/project/performance/). No green badge replaces the stated limits. External ignored fixtures, hardware tests and ordinary CI have different execution requirements.

## Report a problem

Start with [troubleshooting](/rusty-bacnet/help/troubleshooting/) or the relevant integration guide. Open a [GitHub issue](https://github.com/jscott3201/rusty-bacnet/issues/new) with revision/artifact, transport, feature set, platform, minimal reproduction and sanitized evidence. Include the exact conflicting documentation link.

Never attach private keys, customer identifiers or unredacted operational captures. Use the project's security-reporting guidance for sensitive findings.
