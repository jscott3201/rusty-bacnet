---
title: "Choose a transport and evidence scope"
description: "Compare Rust and Python owners, features, and the runtime evidence behind each data link."
---

[Build and integrate](/rusty-bacnet/development/overview/) / Transport evidence

Select the application owner, binding and data link separately. This is a navigation map to scoped evidence, not a second conformance database or an all-platform support badge.

## Match the link to an owner

| Data link | Entry points | Setup and important boundary |
|---|---|---|
| B/IP, NORMAL | Rust/Python standalone client and server; shared endpoint | Explicit local IPv4 interface and UDP port; selected Network Port registration is a separate opt-in |
| B/IP, BBMD or foreign | Rust client, server and endpoint builders (`.mode(BipMode::…)`); Python `bbmd=` and `foreign_device=`; CLI `--foreign-bbmd` | BDT/FDT or configured BBMD; one transport is never both; shared-endpoint Number evidence does not qualify every administrative operation |
| B/IPv6 | Standalone Rust/Python client and server; Rust configured foreign-device mode | Select one usable interface/address; peers are named by 3-octet VMAC (`bip6:<instance>`); no shared endpoint builder or Python foreign-device entry point |
| BACnet/SC | Rust/Python standalone owners and shared endpoint | Operational credentials, explicit CA and durable UUID/VMAC configuration; Rust TLS paths require `sc-tls` |
| MS/TP | Rust transport/server/endpoint; Python standalone and endpoint paths | Serial adapter, station, baud and token participation; do not infer physical timing from simulation |
| Ethernet | Linux Rust standalone client and server | AF_PACKET raw socket permissions; no shared endpoint builder or Python Ethernet API |

The CLI has its own [configuration surface](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/CLI.md). A Rust transport does not imply a matching CLI command or Python constructor. For exact feature gates, use the crate manifests and [Rust API](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/rust-api.md#bacnet-transport); do not enable every feature merely to discover which one you need.

## Read the Number-control evidence at its actual scope

| Evidence | Exercised behavior | Limit |
|---|---|---|
| B/IP loopback | Server/endpoint NORMAL controls; independent standalone-client NORMAL/BBMD/foreign replies; full-server and shared-endpoint BBMD Original-Broadcast and foreign DBTN captures | Linux supplies BBMD broadcast observation; no physical-LAN qualification |
| Constrained local SC TLS | Full server/shared endpoint and standalone client Number bytes and Hub broadcast VMAC; direct query replies use Hub broadcast | Generic controlled tests separately prove queued/pending cancellation; TLS trust does not authenticate a relayed NPDU origin |
| MS/TP LoopbackSerial | Full server, shared endpoint and standalone client standard frames, token opportunity, mode parity and producer cancellation | Simulator evidence, not RS-485 timing, transceiver control or hardware interoperability |
| Isolated Linux Ethernet | Full server/standalone client MAC, LLC, length, payload and padding; raw-FD stop/drop and canceled-stop ownership | Opt-in virtual-link fixture needs raw-socket capability; excluded from ordinary CI |
| Isolated Linux B/IPv6 | Full server/standalone Rust client selected-link OriginalBroadcast and configured-foreign DBTN; independent bytes/address/interface; normal installed-Python evidence | Explicit external ignored tests; no physical LAN, IPv6 endpoint builder or full Annex U claim |

Standalone-client BBMD/foreign cases retain BDT/FDT admission, alternate-sender compatibility and registration NAK/retry behavior, and prove requester progress plus awaited-stop socket release. Other implemented opt-ins are not automatically independently qualified by this table.

## Select IPv6 explicitly when discovery is ambiguous

Normal IPv6 startup chooses one unambiguous usable local link/address. A non-loopback multicast interface and a unique non-link-local address are preferred. If selection is ambiguous, supply a concrete local IPv6 address rather than assuming `::` means all interfaces. There is no silent physical-selection fallback to `::1`.

Incoming destination and interface checks run before VMAC learning/application delivery; outgoing data and controls keep the selected source and bound port. Explicit loopback remains node-local. Windows has compile evidence for the described selection path, not the isolated Linux runtime qualification. See the [Python IPv6 contract](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/python-api.md#bacnetipv6) before migrating an application that relied on `::` choosing an address for it.

## Plan your own validation

Record the source revision, interface, feature set, OS and native artifact. Start with a known peer and bounded read. Add registration, discovery, subscriptions or writes only when the intended operation needs them. Preserve the distinction between a successful build, a loopback test, simulated serial frames, an actual isolated wire capture and a hardware deployment test.

[Support and conformance](/rusty-bacnet/project/support/) summarizes what is supported. [MS/TP qualification](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/mstp-qualification.md) covers the separate hardware path. No page here claims BTL certification.

## Next steps

[Compose a shared endpoint](/rusty-bacnet/development/shared-endpoints/) · [Configure SC](/rusty-bacnet/development/bacnet-sc/) · [Use passive Number controls](/rusty-bacnet/development/network-number/)
