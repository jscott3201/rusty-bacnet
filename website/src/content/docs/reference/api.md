---
title: "API and example library"
description: "Find exact signatures for the release, and where unreleased changes are described."
---

Use the website for tasks and the canonical references for exact methods, parameters, types and feature gates. The references below describe **v0.13.0**.

## Release references

Start with the [integration overview](/rusty-bacnet/development/overview/) and [engineering documentation map](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/README.md).

| Reference | Use it for |
|---|---|
| [Rust API](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/rust-api.md) | Crate APIs, feature gates, shared endpoint contracts, transport ownership and migration details |
| [Python API](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/python-api.md) | Constructors, native Awaitable results, endpoint lifecycle and binding-specific limits |
| [Python type stub](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/crates/rusty-bacnet/rusty_bacnet.pyi) | Editor-facing signatures for the matching native build |
| [CLI reference](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/CLI.md) | Flags and transport prerequisites |
| [Architecture](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/architecture.md) | Crate composition, packet paths and lifecycle ownership |

For source checkout rustdoc, select the features used by your application. Do not assume every workspace crate is published or that a published documentation page matches the current checkout. Use the [shared endpoint guide](/rusty-bacnet/development/shared-endpoints/) to compare requester, responder and source Audit scope before choosing that owner.

A type constant alone does not prove that a bundled server implements an object family.

## Unreleased changes

The same documents on the [`dev` branch](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/README.md) describe changes merged after the release. A `dev` checkout may differ from the release while its version still reads 0.13.0, so record the commit; pending changelog entries are in [`changelog.d/`](https://github.com/jscott3201/rusty-bacnet/tree/dev/changelog.d).

## Examples by task

| Example | Read it for | Review before running |
|---|---|---|
| `bip_client_server.py` | Client/server lifecycle, reads and RPM | Socket exposure and demonstration writes |
| `cov_subscriptions.py` | Subscriptions and consumption | Remote subscription state and cleanup |
| `sc_secure_connect.py` | Hub and node configuration | Credentials and identity rules at the chosen revision |
| `mstp_mini_device.py` | Serial MS/TP device | Adapter, MAC, baud and token participation |
| `device_management.py` | Management services and errors | Control-changing operations |

Browse the [v0.13.0 examples](https://github.com/jscott3201/rusty-bacnet/tree/v0.13.0/examples/python). The site's [local lab](/rusty-bacnet/start/local-lab/) keeps its tested release source and download together.
