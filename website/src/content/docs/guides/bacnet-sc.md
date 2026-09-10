---
title: "Configure BACnet/SC"
description: "Treat certificate trust, local identities, and build features as explicit prerequisites."
---

:::caution[Release guide, not current dev]
This page targets **v0.11.0**. Current `dev` has intentionally stricter SC configuration: the CLI requires explicit `--sc-ca` with no system-root fallback; Python requires explicit CA/operational credentials and device UUIDs. Do not mix those APIs with the release commands below. See the current-dev engineering references for [CLI trust migration](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/CLI.md#transport-variants), [Python credentials](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/python-api.md#required-operational-credentials), and [device UUID migration](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/python-api.md#sc-device-uuid-migration).
:::

BACnet/SC setup combines a WebSocket/TLS connection with BACnet-specific identities and hub behavior. A successful TCP or TLS connection alone does not establish a complete BACnet/SC deployment.

## Before connecting

Obtain the hub URL, your node's certificate and private key, the trust arrangement for the hub's certificate chain, and an allocated VMAC and device UUID where the interface requires them. Ensure the certificate identity matches the hub name you are using and that the system clock is appropriate for certificate validation.

Keep private keys outside the repository and the generated site. Example names below are not deployable credentials.

## CLI feature and identity setup

Build a CLI with the `sc-tls` feature when required:

```sh
cargo install bacnet-cli --version 0.11.0 --locked --features sc-tls
```

The reviewed CLI requires a hub URL, client certificate, private key, local VMAC, and nonzero local device UUID:

```sh
bacnet --sc \
  --sc-url wss://hub.example.com/bacnet \
  --sc-cert node-cert.pem \
  --sc-key node-key.pem \
  --sc-vmac 22:01:02:03:04:05 \
  --sc-device-uuid 00112233-4455-6677-8899-aabbccddeeff \
  read 00:01:02:03:04:05 ai:1 pv
```

Replace every identity and path with an approved value for your network. The final VMAC is the **remote** target; `--sc-vmac` is the **local** node identity. Do not reuse the example VMAC or UUID throughout a deployment.

### The CLI trust store is not the Python trust argument

The v0.11.0 CLI constructs its trust store from native root certificates. Its reviewed parser does not offer a `--sc-ca-cert` flag. A private hub CA therefore needs an appropriate trust path on the actual execution host, following site policy, or a different supported application configuration.

Do not “fix” certificate failures by disabling certificate validation. A client certificate supplied through `--sc-cert` is not the same thing as trusting the issuer of the hub's certificate.

## Python trust configuration

The Python client exposes `sc_ca_cert`, `sc_client_cert`, and `sc_client_key`. The hub exposes `ca_cert` for trusted node issuers. These are different roles: a node trusts the hub's issuer, and the hub validates connecting node identities using its configured trust.

The v0.11.0 repository distinguishes a hub configured with a trusted issuer CA from an example that omits `ca_cert`. The latter is server-auth-only example mode, not claimed BACnet/SC mutual-TLS conformance evidence. **Do not use server-auth-only example mode for deployment.** Configure trusted node issuers and operational credentials; current `dev` has removed this example mode.

Use the versioned Python API and secure-connect example for release constructor details rather than translating CLI flags mechanically into Python names.

## Validate the whole path

Check the package/build feature, hub URL and port, certificate chain and validity, hostname, local VMAC/UUID, hub acceptance, and remote target identity. After joining the hub, perform a bounded read against a known object. Record sanitized failure stages; never attach a private key to an issue.

These instructions explain configuration boundaries. They are not a certificate-authority operating procedure or a BACnet/SC certification claim.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[CLI SC construction and native roots](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/transport.rs) · [CLI flags](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/crates/bacnet-cli/src/args.rs) · [Python trust and hub guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/README.md) · [SC example](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/examples/python/sc_secure_connect.py).
