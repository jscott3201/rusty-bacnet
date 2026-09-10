---
title: "API and example library"
description: "Use the website for tasks and the versioned APIs for exact signatures."
---

The website explains how to complete a task. The source-level references explain the exact methods, parameters, types, and feature gates. Both are needed, but they should not become two competing handwritten API inventories.

## Rust

[The v0.11.0 Rust API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/rust-api.md) covers the crate-level surfaces and examples. The workspace manifest is the release version and Rust-minimum reference.

For a local source checkout, generate API documentation with the selected features appropriate to your use case. Verify any published rustdoc link and version before adding it to the site; do not assume every internal workspace crate has a published documentation page.

## Python

[The Python API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/python-api.md) explains asynchronous clients, servers, object values, exceptions, and SC hubs. [The distributed type stub](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/rusty-bacnet/rusty_bacnet.pyi) provides editor-facing signatures.

The release's Python documentation has an abbreviated transport constructor summary that omits MS/TP; use the mini-device example and implementation for that path. A type constant alone does not prove that the bundled server implements that object family.

## Examples

| Example | Read it for | Review before running |
|---|---|---|
| `bip_client_server.py` | Client/server lifecycle, reads and RPM | Socket exposure and demonstration writes |
| `cov_subscriptions.py` | Subscriptions and consumption | Remote subscription state and cleanup |
| `sc_secure_connect.py` | Hub and node configuration | Certificate roles and example trust settings |
| `mstp_mini_device.py` | Standalone serial device | Adapter, MAC, baud, and token participation |
| `device_management.py` | Management services and errors | Control-changing operations |

[Browse the versioned Python examples](https://github.com/jscott3201/rusty-bacnet/tree/v0.11.0/examples/python).

## Companion projects

The repository points to separate HTTP/MCP gateway and external test-harness projects. They are not installed by this website, and their authentication, APIs, deployment, and certification status must be documented in their own context. Keep ecosystem links separate from the core CLI/Python/Rust installation flow.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[Rust API](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/rust-api.md) · [Python API](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/python-api.md) · [Example index](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/examples/python/README.md) · [Companion project boundaries](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/README.md).
