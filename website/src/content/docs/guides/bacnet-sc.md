---
title: "Configure BACnet/SC"
description: "Treat certificate trust, local identities, and build features as explicit prerequisites."
---

BACnet/SC setup combines a WebSocket/TLS connection with BACnet-specific identities and hub behavior. A successful TCP or TLS connection alone does not establish a complete BACnet/SC deployment.

This page covers the CLI and the trust roles. For Python and Rust credentials, durable device identity and failure stages, continue with [BACnet/SC setup](/rusty-bacnet/development/bacnet-sc/).

## Before connecting

Obtain the hub URL, the CA certificate that verifies the hub, your node's certificate and private key, and an allocated VMAC and device UUID. Ensure the hub's certificate identity matches the hub name you are using and that the system clock is appropriate for certificate validation.

Keep private keys outside the repository and the generated site. Example names below are not deployable credentials.

## CLI feature and identity setup

Every [release CLI executable](/rusty-bacnet/start/installation/#install-a-cli-executable) includes BACnet/SC. For a [source build](/rusty-bacnet/start/installation/#prefer-a-source-build), enable the `sc-tls` feature:

```sh
cargo install --path crates/bacnet-cli --locked --features sc-tls
```

The CLI requires a hub URL, the hub's CA, a client certificate and private key, a local VMAC, and a nonzero local device UUID:

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

Replace every identity and path with an approved value for your network. The final VMAC is the **remote** target; `--sc-vmac` is the **local** node identity. Do not reuse the example VMAC or UUID throughout a deployment.

### Trust the hub's issuer, not the system roots

`--sc-ca` names the CA PEM that verifies the hub's certificate. The CLI has no fallback to the operating system's root certificates, so a private hub CA needs no change to the host's trust store.

Do not “fix” certificate failures by disabling certificate validation. A client certificate supplied through `--sc-cert` is not the same thing as trusting the issuer of the hub's certificate.

## Python trust configuration

The Python client and server take `sc_ca_cert`, `sc_client_cert`, `sc_client_key` and a keyword-only `sc_device_uuid`; all four are required for `transport="sc"`. The hub (`ScHub`) takes `ca_cert` for trusted node issuers. These are different roles: a node trusts the hub's issuer, and the hub validates connecting node identities using its configured trust.

A hub always requires `ca_cert` and verifies each node's certificate; there is no server-auth-only mode. Use the Python API and secure-connect example linked below for constructor details rather than translating CLI flags mechanically into Python names.

## Validate the whole path

Check the package/build feature, hub URL and port, certificate chain and validity, hostname, local VMAC/UUID, hub acceptance, and remote target identity. After joining the hub, perform a bounded read against a known object. Record sanitized failure stages; never attach a private key to an issue.

These instructions explain configuration boundaries. They are not a certificate-authority operating procedure or a BACnet/SC certification claim.

## Sources and release scope

These instructions target **v0.13.0**. Source review is not a claim of hardware qualification.

[CLI SC transport](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/CLI.md#transport-variants) · [CLI flags](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/crates/bacnet-cli/src/args.rs) · [Python credentials](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/docs/python-api.md#required-operational-credentials) · [SC example](https://github.com/jscott3201/rusty-bacnet/blob/v0.13.0/examples/python/sc_secure_connect.py).
