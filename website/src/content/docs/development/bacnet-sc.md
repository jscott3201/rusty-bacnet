---
title: "BACnet/SC setup"
description: "Connect an SC node with explicit trust, operational credentials and caller-owned device identity."
---

[Build and integrate](/rusty-bacnet/development/overview/) / BACnet/SC setup

This page configures BACnet/SC for the CLI, Python and Rust. The [BACnet/SC guide](/rusty-bacnet/guides/bacnet-sc/) explains the trust roles. Every surface requires explicit trust and identity. The [upgrade guide's BACnet/SC section](/rusty-bacnet/project/upgrading/#bacnetsc-security-serverpolicy-and-principals) lists what changed for SC in this release.

## Prepare identities and credentials before connecting

Obtain these from the installation owner:

- The hub's `wss://` URL and the CA PEM used to verify it, including the expected hostname.
- This node's operational certificate and matching private key.
- A nonreserved local VMAC and a nonzero 16-byte device UUID, distinct from other devices.
- A known remote VMAC, object and property for a bounded initial read.

The caller provisions and durably stores the device UUID before first deployment, then loads the same identity across restarts. Do not generate a new UUID in startup code. Distinct devices must not share it; the hub's same-UUID replacement behavior is intentional. Credentials and UUIDs are different roles: a syntactically valid UUID does not establish certificate ownership.

## Connect with the CLI

Every release CLI executable includes BACnet/SC. For a source build, enable `sc-tls` without adding packet capture:

```sh
cargo install --path crates/bacnet-cli --locked --features sc-tls
```

After substituting your approved paths and identities:

```sh
bacnet --sc \
  --sc-url wss://hub.example.com/bacnet \
  --sc-ca site-ca.pem \
  --sc-cert node-cert.pem \
  --sc-key node-key.pem \
  --sc-vmac 22:01:02:03:04:05 \
  --sc-device-uuid 00112233-4455-6677-8899-aabbccddeeff \
  read 00:01:02:03:04:05 ai:1 pv
```

`--sc-vmac` identifies the local node; the final VMAC identifies the remote target. CLI trust requires `--sc-ca`: there is no system-root fallback. The read sends traffic to the named peer and does not perform discovery. Check the [CLI transport reference](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/CLI.md#transport-variants) for exact parser and startup failures.

## Use the same prerequisites in Python and Rust

Python `BACnetClient` and `BACnetServer` with `transport="sc"` require nonempty `sc_ca_cert`, `sc_client_cert`, `sc_client_key` and keyword-only `sc_device_uuid`. Load the latter as 16 bytes from caller-owned durable storage. Missing or invalid identity/credential presence fails construction; file loading and TLS configuration happen at async startup. A trusted hub's verification policy remains a separate fact.

The [Python credential example](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md#required-operational-credentials) and [UUID migration](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md#sc-device-uuid-migration) are the signature authority. Local TLS configuration errors can be repaired at the same paths and retried within the documented boundary; this does not promise rollback after every later peer/dial failure.

Rust uses `ScNodeTlsConfig` and the transport's configured device identity. Raw `ScTransport::new(ws, vmac)` is initially unconfigured; set a valid UUID before start. Because the caller can dial `ws` first, transport validation cannot undo that earlier connection. A shared endpoint uses `ScEndpointBuilder` with a caller-dialed TLS WebSocket; see [shared endpoints](/rusty-bacnet/development/shared-endpoints/).

## Diagnose the stage that failed

| Stage | Useful check |
|---|---|
| Local configuration | CA/certificate/key file readability and PEM validity, matching key, required UUID and VMAC |
| TLS connection | URL, hostname, certificate validity and issuer trust; operational credentials on both peers |
| SC Connect | Nonzero peer UUID and nonzero advertised limits; hub admission/replacement policy |
| BACnet request | Remote VMAC, requested object/property, request limits and response/error |
| Shutdown | Await the owning client/server/session cleanup; separately stop and join any caller-owned direct listener |

Zero-limit rejection is a bounded local policy, not a claim that all positive limit combinations satisfy the full Standard. TLS 1.3 and explicit node credentials are local policy; successful connection does not prove that an arbitrary remote hub requested and verified the node certificate.

## Keep transport identity separate from operation authority

Accepted direct TLS connections can carry a verified leaf identity and original-socket response authority in the native Rust path. A Hub-relayed NPDU does not become an authenticated originating leaf. Outgoing client transactions retain their documented correlation/routing rules, not a universal same-leaf continuity guarantee. Python does not expose a direct-listener entry point or a principal authorizer.

Passive Number replies, including replies to direct queries, use the Hub broadcast path. These controls do not add configured SC Network Port authority or control-origin authorization. Keep application mutation authorization distinct from TLS trust.

## Next steps

[SC Rust contracts](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/rust-api.md#bacnetsc-client-transport) · [Transport evidence](/rusty-bacnet/development/transports/) · [Network Number controls](/rusty-bacnet/development/network-number/)
