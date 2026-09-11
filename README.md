# Rusty BACnet

A BACnet protocol stack in Rust, with Python bindings and a command-line tool.
Use it to build BACnet clients, model devices and serve their properties, or
explore protocol behavior in a local lab. The project targets ASHRAE Standard
135-2020 and tracks implementation evidence at the clause level.

[![CI](https://github.com/jscott3201/rusty-bacnet/actions/workflows/ci.yml/badge.svg)](https://github.com/jscott3201/rusty-bacnet/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**[Documentation](https://jscott3201.github.io/rusty-bacnet/)** ·
[Installation](#installation) · [Quickstarts](#read-a-property) ·
[Capabilities](#capabilities-and-conformance) · [Transports](#transports-and-platforms) ·
[Contributing](#development-and-contributing)

> **Choose documentation for your version.** This README on `dev` describes the
> current checkout, including unreleased changes. Published Python and Rust
> packages and the hosted getting-started guides currently target **0.11.0**.
> The workspace still carries that version number; it does not mean all changes
> on `dev` are in the published packages. Use the [0.11.0 release](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.11.0)
> and [versioned Rust API](https://docs.rs/bacnet-client/0.11.0/bacnet_client/)
> for a released deployment. The [SC migration section](#bacnetsc-current-development-checkout)
> below specifically requires a current source build.

## What is included?

- Async Rust client and server APIs, with protocol encoding, network routing,
  transaction handling, and object models in separate crates.
- Python client, server, and SC hub bindings, typed values and exceptions, and
  async COV notification streams. Python and Rust expose different configuration
  surfaces; do not assume feature parity.
- BACnet/IP, BACnet/IPv6, BACnet/SC, MS/TP, and Linux Ethernet code paths, subject
  to the feature, platform, and qualification limits below.
- A `bacnet` CLI for targeted reads, discovery, an interactive shell, and optional
  packet capture. Operations that change devices require separate authorization.

**Work only on networks and devices you are authorized to access.** Start with
the loopback examples below. Discovery creates network traffic; writes, device
management, time synchronization, and file operations can affect real equipment.
This library does not provide a physical-safety guarantee. BACnet/SC credential
validation is not authorization to perform a BACnet operation.

## Installation

### Python: published package

The distribution is [`rusty-bacnet`](https://pypi.org/project/rusty-bacnet/0.11.0/);
the import name is `rusty_bacnet`. Python **3.11 or newer** is required.

```bash
python3 -m venv .venv
source .venv/bin/activate
# Windows PowerShell: .venv\Scripts\Activate.ps1
python -m pip install --only-binary=:all: "rusty-bacnet==0.11.0"
```

The wheel-only command avoids an unexpected native build. If no compatible
wheel exists for your interpreter and platform, use the [source-build steps](#build-from-source)
with the required Rust/native toolchain. A Python version constraint does not
promise wheels for every interpreter, architecture, or free-threaded build.

### Rust: published crates

Add only the crates you need. The read example below uses aligned 0.11.0
dependencies; the declared minimum Rust version is **1.93**.

```toml
[dependencies]
bacnet-client = "=0.11.0"
bacnet-types = "=0.11.0"
bacnet-encoding = "=0.11.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

See the [released client API](https://docs.rs/bacnet-client/0.11.0/bacnet_client/)
or the [checkout's Rust reference](docs/rust-api.md) for lower-level transports,
routed requests, server configuration, and crate-specific features.

### CLI: release binary or source build

Download the appropriate `bacnet` executable from the
[0.11.0 release assets](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.11.0).
Assets are provided for Linux amd64/arm64, macOS amd64/arm64, and Windows amd64.
An available binary is not a guarantee that every optional transport or capture
feature is enabled or qualified on that platform. Check its `--help` and release notes.

Alternatively, from a checkout:

```bash
cargo install --path crates/bacnet-cli --locked
# Include BACnet/SC when needed:
cargo install --path crates/bacnet-cli --locked --features sc-tls
```

The CLI enables IPv6 through its dependencies; it has no separate `ipv6` feature
switch. Packet capture is a separate `pcap` feature and needs the native capture
library and appropriate permissions. Do not confuse capture filters with the
Linux Ethernet transport.

## Read a property

Start the [local Python server](#run-a-local-python-server) in another terminal
before running either example. Both readers use loopback and an ephemeral local
UDP port (`0`), so they do not compete with the server's port `47808`. These
examples send a targeted ReadProperty request, not discovery or a write.

### Python reader

Save as `read_property.py` and run `python read_property.py` in your environment.

```python
import asyncio
from rusty_bacnet import BACnetClient, ObjectIdentifier, ObjectType, PropertyIdentifier


async def main():
    async with BACnetClient(
        interface="127.0.0.1", port=0, broadcast_address="127.0.0.1"
    ) as client:
        value = await client.read_property(
            "127.0.0.1:47808",
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(value.value)  # 22.5 from the local example server


if __name__ == "__main__":
    asyncio.run(main())
```

The context manager starts and stops the client, including when a request fails.
For device discovery, routed addresses, multiple-property reads, COV, and error
handling, continue with the [Python guide](https://jscott3201.github.io/rusty-bacnet/start/python/)
or [checkout API reference](docs/python-api.md).

### Rust reader

With the dependencies above, put this in `src/main.rs` and run `cargo run`:

```rust
use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .build()
        .await?;

    // BACnet/IP MAC: four IPv4 octets, then the two-byte UDP port (47808).
    let address = [127, 0, 0, 1, 0xBA, 0xC0];
    let response = client
        .read_property(&address, oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    client.stop().await?; // also stop before returning a request error

    let ack = response?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("Value: {value:?}");
    Ok(())
}
```

See the [Rust guide](https://jscott3201.github.io/rusty-bacnet/start/rust/)
for a larger application and the [architecture guide](docs/architecture.md) for
the boundary between transport MACs, network routing, and BACnet device identity.

## Run a local Python server

Save as `local_server.py`, then run `python local_server.py`. It exposes a
simulated temperature on loopback only; it does not control physical equipment.
Add objects before starting the server. Stop it with **Ctrl+C**.

```python
import asyncio
from rusty_bacnet import BACnetServer


async def main():
    server = BACnetServer(
        device_instance=1234,
        device_name="Local BACnet lab",
        interface="127.0.0.1",
        port=47808,
        broadcast_address="127.0.0.1",
    )
    server.add_analog_input(1, "Zone temperature", units=62, present_value=22.5)
    try:
        await server.start()
        print(f"Listening at {await server.local_address()}", flush=True)
        await asyncio.Event().wait()
    finally:
        await server.stop()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
```

Units `62` mean degrees Celsius. Keep device identifiers unique when moving
beyond this isolated lab. See the [local-lab guide](https://jscott3201.github.io/rusty-bacnet/start/local-lab/)
and [examples directory](examples/) for other setups; review their network and
credential requirements before running them.

## CLI reads

With the same local server running:

```bash
bacnet --help
bacnet --interface 127.0.0.1 --port 0 read 127.0.0.1:47808 ai:1 pv
bacnet --interface 127.0.0.1 --port 0 --json readm 127.0.0.1:47808 ai:1 pv,object-name
```

`ai:1` is Analog Input instance 1 and `pv` is Present_Value. The
[CLI reference](docs/CLI.md) covers the shell, output formats, discovery,
subscriptions, and optional capture. Treat write, object-management, file,
time-sync, and BBMD-management commands as advanced operations requiring explicit
permission—not as connectivity checks. For SC, use a build with `sc-tls` and the
current reference's complete CA/certificate/key and provisioned-identity options.

## Capabilities and conformance

**Rusty BACnet does not claim BTL certification or full BACnet conformance.**
Implemented code, passing interoperability examples, and support backed by
clause-specific evidence are different things. Use the
[Standard 135-2020 ledger](docs/conformance/standard-135-2020-ledger.md),
[draft PICS](docs/conformance/pics-draft.md), and
[machine-readable evidence](docs/conformance/bacnet-135-2020.json) to check the
particular service, object, direction, and configuration you need.

| Area | Scope and important limits |
|---|---|
| Property access and objects | Client/server paths for single and multiple-property access, with common and extended object models. Served properties, writable behavior, and optional functionality are object-specific; an object constructor is not a declaration of full object conformance. |
| Discovery, routing, and COV | Rust client/network components and Python APIs expose discovery, routed requests, subscriptions, and notifications. Consult the individual API for address forms, notification delivery, and lifecycle ownership. |
| Events, logs, files, and device management | APIs and server handlers exist for selected services, with configuration, authorization, resource, and persistence limits. Client availability does not imply equivalent server execution or Python configuration support. |
| LifeSafetyOperation | The Rust server implements a partial modeled behavior with explicit application authorization; reset needs a configured application executor. This is not whole-object conformance or physical-safety qualification. |
| Audit services | Rust client helpers and Python raw/typed helpers are available. Rust server notification reception requires an explicit Audit Log sink and separate confirmed/unconfirmed authorizers; AuditLogQuery requires retained Audit Log storage. Python server object helpers do not wire the notification receiver's sink/authorizers. |

Audit records contain peer-reported identities, not authenticated provenance.
Notification storage uses the database's write path and depends on synchronous
storage availability. See [Rust Audit configuration and limits](docs/rust-api.md#audit-services)
and [Python Audit APIs](docs/python-api.md#audit-services). The server does not
claim execution of WriteGroup, Virtual Terminal, or PrivateTransfer services
merely because a client can send them.

## Transports and platforms

These are implemented code paths, not an all-platform or hardware-qualification
matrix. Flags in this table belong to **`bacnet-transport`**; other crates have
their own feature sets. Check their Cargo manifests rather than applying one
flag list to the whole workspace.

| Transport | Feature | Scope |
|---|---|---|
| BACnet/IP (UDP/IPv4) | Available without optional features | Client/server transport and BBMD-related paths; see Annex J evidence. |
| BACnet/IPv6 (UDP multicast) | `ipv6` | Cross-platform networking code; interface and multicast behavior need validation on the deployment network. |
| BACnet/SC nodes and hub | `sc-tls` | WebSocket/TLS code for nodes and an accepting relay hub; current-development security and admission limits below. |
| MS/TP | `serial`; `serial-gpio` for GPIO support | Protocol core, serial adapter, and loopback evidence. Kernel RS-485/ioctl and GPIO facilities are Linux-specific; physical timing is not qualified across operating systems or adapters. |
| Ethernet (802.3 LLC) | `ethernet` | Linux `AF_PACKET` transport, not a generic BPF backend. Requires suitable interface permissions and platform qualification. |

Annex J NAT traversal and IPv4 BACnet/IP multicast (B/IP-M) are **not claimed** by
the current BACnet/IP transport. Loopback or protocol tests do not establish
serial hardware timing, deployed-network behavior, or cross-OS support.
Routine CI tests run on Linux; cross-OS and MSRV checks run on selected
main/release paths, not every feature PR. See the [CI configuration](.github/workflows/ci.yml).

## BACnet/SC: current development checkout

**This section describes unreleased source behavior, not the installed 0.11.0
package contract.** Build from current source if you need these changes. Review
the [Python SC migration](docs/python-api.md#bacnetsc-secure-connect),
[Rust node migration](docs/rust-api.md#bacnetsc-client-transport), and
[Rust hub migration](docs/rust-api.md#bacnetsc-hub) before updating an existing deployment.

### Credentials and device identity

- Built-in hub and node paths use TLS 1.3 as local policy. Supply an explicit site
  CA and matching certificate/private key. Python `ScHub` requires `ca_cert`,
  `cert`, and `key`; Python SC clients/servers require `sc_ca_cert`,
  `sc_client_cert`, and `sc_client_key`. There is no insecure hub mode or
  system-root fallback in these paths. Credentials are validated before the hub
  binds or the built-in node dials; omission of required paths is an error.
- Provision a **nonzero 16-byte Device UUID before deployment**, store it durably,
  and reuse it for that device's lifetime. Distinct devices need distinct
  identities; do not generate a new UUID on every start. Python nodes use
  `sc_device_uuid`; the hub's hosting device uses `device_uuid`. Local VMACs must
  be six bytes and neither all-zero nor all-ff.
- Raw Rust `ScTransport::new(ws, vmac)` remains a two-argument, unconfigured
  constructor. Call `.with_device_uuid(...)` before `start()`. Startup rejects
  missing/all-zero UUIDs and all-zero/all-ff local VMACs before **transport-owned
  I/O**, not creation or dialing of the caller's `ws`.
- UUID bytes are not validated for version/variant bits. There is no
  certificate-to-UUID binding, durable identity-change detection, or enforced
  lifetime immutability. Post-start mutation through Rust's public `connection()`
  is outside this startup guard. See the [identity validation scope](docs/conformance/standard-135-2020-ledger.md#device-identity-acceptance-closeout).

CA membership is not BACnet-operation authorization and does not bind a
certificate to a VMAC or Device UUID. These checks are not a claim of the entire
Annex AB security profile or qualification of your credential provisioning.

### Compatibility and forwarding limits

- After TLS/WebSocket establishment, an all-zero peer UUID in Connect-Request is
  rejected before registration/replacement; eligible Requests receive
  `COMMUNICATION/PARAMETER_OUT_OF_RANGE` (7/80), not a duplicate-VMAC error.
  Initiating nodes silently discard Connect-Accept with a zero UUID, preserving
  state, peer limits, and the original Connect deadline. A later valid Accept
  can complete the handshake; otherwise that original wait expires. See
  [peer UUID admission](docs/conformance/standard-135-2020-ledger.md#received-peer-uuid-admission).
- Zero advertised Max-BVLC or Max-NPDU is rejected by a **zero-only local policy**:
  eligible Requests receive 7/80 and nodes silently discard invalid Accepts
  without resetting the original deadline or committing peer limits. Positive
  values, including tiny or inverted pairs, still pass this check; that is not
  a serviceability guarantee or universal minimum-capacity conformance claim.
  Defaults and outgoing budgets are unchanged. See [zero-capacity admission](docs/conformance/standard-135-2020-ledger.md#received-zero-capacity-admission).
- Registered, correctly addressed unknown BVLC functions can transit the hub as
  opaque bytes, with source stamping, no source echo, encoded BVLC limits, and
  a guarded Result return path. Pre-registration frames are not forwarded.
  This does not add general forwarding for other known BVLC functions; see the
  [bounded forwarding and rejection scope](docs/conformance/standard-135-2020-ledger.md#hub-unknown-transit-and-result-return).

## Development and contributing

Bug reports, documentation improvements, test cases, and focused patches are
welcome. Start with a small reproducible example and keep changes scoped to a
protocol boundary or user-visible behavior. Include tests for changed behavior
and update the relevant evidence/docs without broadening support claims.

### Build from source

For **current development**, clone `dev`; use the `v0.11.0` tag instead when you
need that release's source. The checked-in toolchain is 1.97.1, distinct from the
declared minimum Rust version 1.93. Native build requirements depend on the
selected transport, TLS provider, and Python interpreter.

```bash
git clone --branch dev https://github.com/jscott3201/rusty-bacnet.git
cd rusty-bacnet
cargo build --locked
```

For Python source development, activate a virtual environment first:

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install "maturin>=1,<2" pytest pytest-subtests
maturin develop --manifest-path crates/rusty-bacnet/Cargo.toml --locked
python -m pytest --import-mode=append -q crates/rusty-bacnet/tests
```

The Python package builds a native PyO3 extension. Rust compilation alone does
not verify its installed Python API; run the Python tests with the interpreter
and artifact you intend to use. A source build from `dev` may still report 0.11.0
as its version, so record the source revision as well.

### Useful checks

From the repository root, with the selected Python environment active for the
binding check:

```bash
cargo test --workspace --exclude rusty-bacnet --locked
cargo clippy --workspace --exclude rusty-bacnet --all-targets --locked
cargo check -p rusty-bacnet --tests --locked
cargo fmt --all --check
cargo test -p bacnet-integration-tests --test conformance_ledger --locked
python3 scripts/generate-conformance-docs.py --check
```

The Python `cdylib` is excluded from that workspace test command intentionally;
its check and installed-package tests are separate. Optional-feature tests and
system dependencies depend on your platform—consult the manifests and CI jobs
before enabling serial, Ethernet, SC, or capture features.

### Repository map

- `crates/`: types/codecs/services, transports/network/endpoint core, clients,
  object models/server, Python bindings, CLI, and integration tests.
- `examples/`: Rust, Python, and container-based labs.
- `docs/`: checkout API references, architecture, and conformance evidence.
- `website/`: sources for the hosted documentation.
- `benchmarks/`: benchmark workloads. [Benchmarks.md](Benchmarks.md) is a
  **historical 0.8.0 report from March 2026**, not current performance qualification;
  its retired server-auth-only SC setup must not be treated as a current example.

Not every workspace package is published: the Python binding crate is a
non-published Rust `cdylib`, and integration tests are internal test infrastructure.

## Documentation, help, and companion projects

- [Hosted guides](https://jscott3201.github.io/rusty-bacnet/) — currently for 0.11.0;
  [installation](https://jscott3201.github.io/rusty-bacnet/start/installation/) and
  [support scope](https://jscott3201.github.io/rusty-bacnet/project/support/).
- Checkout references: [Rust](docs/rust-api.md), [Python](docs/python-api.md),
  [CLI](docs/CLI.md), [architecture](docs/architecture.md), and [changelog](CHANGELOG.md).
- [Issues](https://github.com/jscott3201/rusty-bacnet/issues) — include the package
  version/source revision, OS, transport, and a sanitized minimal reproduction.
  Issues are public: do not post private keys, credentials, sensitive deployment
  details, or captures from real networks. Prefer synthetic/local-lab fixtures.
- [`rusty-bacnet-mcp`](https://github.com/jscott3201/rusty-bacnet-mcp) — companion MCP gateway.
- [`rusty-bacnet-btl-harness`](https://github.com/jscott3201/rusty-bacnet-btl-harness) — companion test harness;
  its existence is not a certification or a substitute for the project's conformance evidence.

## License

Rusty BACnet is available under the [MIT License](LICENSE).
