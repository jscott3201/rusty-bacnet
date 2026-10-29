---
title: "Build and integrate"
description: "Choose an application owner and find release API contracts and evidence boundaries."
---

**Integration guides for v0.15.0.** These pages cover the parts of the release that go beyond a first read: serving objects, routing, shared endpoints, Network Port and Number controls, transports and BACnet/SC. Install the release from [v0.15 installation](/rusty-bacnet/start/installation/), or try the [local lab](/rusty-bacnet/start/local-lab/) first.

## Choose the owner for your application

| Your task | Start here |
|---|---|
| Send requests to other devices | Standalone `BACnetClient`; choose a transport and use the API reference |
| Model a device with the full server's service set | [Serve objects from your application](/rusty-bacnet/integrate/server/) with a standalone `BACnetServer` |
| Join BACnet networks and reach devices behind a router | [Route between networks](/rusty-bacnet/integrate/router/) with `BACnetRouter` |
| Request and respond as one device through one transport | [Shared endpoints](/rusty-bacnet/integrate/shared-endpoints/) for B/IP, SC or MS/TP |
| Associate a B/IP socket with a Network Port object | [Network Port registration](/rusty-bacnet/integrate/network-ports/#register-a-bip-receiving-port) |
| Learn and answer local network-number queries | [Passive Number controls](/rusty-bacnet/integrate/network-ports/); no discovery command or startup announcement required |
| Connect through an SC hub | [BACnet/SC setup](/rusty-bacnet/connect/bacnet-sc/), with explicit trust, credentials and durable device identity |
| Decide which peers may write to or control your server | [Restrict who can change a device](/rusty-bacnet/integrate/security/) |
| Describe your device's objects and services in a PICS | [Generate a PICS for your device](/rusty-bacnet/integrate/pics/) |

Shared endpoints deliberately have a narrower responder than the full server. Transport implementation, language binding and actual runtime evidence are separate choices; compare the [transport matrix](/rusty-bacnet/reference/transports/) before selecting a path.

## Working on the repository

For an unreleased change or a different feature set, [build from source](/rusty-bacnet/development/build-from-source/). The contributor guides cover checkout setup and documentation changes separately from release integration.

## Use the reference for exact signatures

The [engineering documentation map](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/README.md) links the canonical [Rust API](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/rust-api.md), [Python API](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/python-api.md) and [CLI reference](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/CLI.md). These task guides summarize those contracts rather than duplicate every method.

Before 1.0, APIs may be removed or changed directly. Keep the source revision, feature selection and installed native artifact together in your integration record. Review [upgrade guidance](/rusty-bacnet/project/upgrading/) when moving an existing application.

## Next steps

- [Serve objects from your application](/rusty-bacnet/integrate/server/).
- [Route between networks](/rusty-bacnet/integrate/router/).
- [Compose a shared endpoint](/rusty-bacnet/integrate/shared-endpoints/).
- [Restrict who can change a device](/rusty-bacnet/integrate/security/).
- [Generate a PICS for your device](/rusty-bacnet/integrate/pics/).
- [Choose a data link and its owner](/rusty-bacnet/connect/overview/).
- [Choose a transport and its evidence scope](/rusty-bacnet/reference/transports/).
- [Understand support and conformance](/rusty-bacnet/project/support/).
- [See what performance to expect](/rusty-bacnet/project/performance/).
