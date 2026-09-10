---
title: "Configure your connection"
description: "Separate local binds, remote addresses, protocol identities, and timeouts."
---

Most configuration mistakes come from mixing settings that sound similar but act at different layers. Configure the local program, the remote target, and the BACnet network identity separately.

## BACnet/IP settings

| Purpose | CLI | Python | Meaning |
|---|---|---|---|
| Local interface selection | `--interface` | `interface` | Local interface/bind input; not a remote device address |
| Local UDP port | `--port` | `port` | The port used by this local client |
| Discovery broadcast destination | `--broadcast` | `broadcast_address` | Where broadcast discovery is directed |
| APDU timeout | `--timeout` | `apdu_timeout_ms` | Request timeout in milliseconds |
| Remote endpoint | Command target | Method `address` | Target address and, where needed, remote port |

The reviewed CLI defaults to local UDP port `47808`, broadcast address `255.255.255.255`, and APDU timeout `6000` ms. One-shot commands without an interface use `0.0.0.0`; the interactive shell may prompt for an interface.

Do not infer receive isolation or multi-interface behavior from the word “bind.” Broadcast reception can involve wildcard socket behavior. Verify the actual host/interface arrangement, especially when multiple network adapters, VPNs, or local BACnet applications are present.

## A remote port is not a local port

```sh
bacnet --port 47810 read 192.168.1.100:47809 ai:1 pv
```

This example chooses local port `47810` and remote port `47809`. It is a directed-read illustration, not a prescription for changing the ports used by an operational BACnet network.

For standard same-subnet discovery, start with the intended BACnet network's port and address plan. Changing the local port just to avoid a conflict can change discovery and response behavior; first identify the competing process.

## Configuration lives in the interface you use

The reviewed CLI exposes command-line options and interactive session settings. These guides do not invent a global `bacnet.toml`, a `--config` flag, or a universal environment-variable layer.

In Python, construct the client or server with the documented keyword arguments. In Rust, configure the corresponding builder and transport. An application's own JSON/TOML configuration can map to those APIs, but that is an application convention—not a built-in CLI contract.

## Keep units visible

Use `--timeout 6000` for a 6000-millisecond APDU timeout. Discovery's `--wait` and foreign-device registration's `--ttl` are measured in seconds. MS/TP baud is a line rate, and BACnet/SC heartbeat fields have their own documented units and bounds.

Do not copy a numeric value between settings merely because the names resemble one another.

## Network-specific prerequisites

On multiple subnets, review BACnet routing and authorized BBMD/foreign-device arrangements; broad UDP port forwarding is not a substitute for a documented topology. In containers or virtual machines, a working unicast read does not prove broadcast reception. For BACnet/SC, follow [certificate and identity setup](/rusty-bacnet/guides/bacnet-sc/). For a serial trunk, follow [MS/TP commissioning](/rusty-bacnet/guides/mstp/).

## Keep configuration out of published evidence

Documentation, GitHub Pages, public issues, and screenshots must not contain real private keys, credentials, unredacted building addresses, or customer network captures. Use representative examples and privately retained operational records.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[CLI definitions](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/args.rs) · [CLI transport construction](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/transport.rs) · [Python constructor](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/python-api.md).
