# Rusty BACnet

A BACnet protocol stack written in Rust, with Python bindings, a command-line
tool with a terminal UI, and an MCP server for AI agent hosts. Use it to build
BACnet clients, model devices and serve their objects, or explore a network in a
local lab. It targets ASHRAE Standard 135-2020 and isn't BTL certified.

[![crates.io](https://img.shields.io/crates/v/bacnet-client.svg)](https://crates.io/crates/bacnet-client)
[![PyPI](https://img.shields.io/pypi/v/rusty-bacnet.svg)](https://pypi.org/project/rusty-bacnet/)
[![docs.rs](https://img.shields.io/docsrs/bacnet-client)](https://docs.rs/bacnet-client)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**[Documentation](https://jscott3201.github.io/rusty-bacnet/)** ·
**[Rust API](https://docs.rs/bacnet-client)** ·
**[Python API](docs/python-api.md)**

> [!IMPORTANT]
> The packages, the website and docs.rs describe release **0.15.0**; this README
> and [`docs/`](docs/) follow `dev`. It is pre-1.0, so APIs change between minor
> releases. Use it only on networks you are authorized to access: discovery
> sends traffic, writes change real equipment, and it makes no physical-safety
> guarantee.

## What you can build

| Surface | What it gives you | Start here |
|---|---|---|
| Rust client and server | Tokio roles: discovery, segmentation, COV, alarms, and an object database with the standard object types | [Rust guide](https://jscott3201.github.io/rusty-bacnet/start/rust/) |
| Python (asyncio) | Client, server and SC hub, with typed values and COV streams | [Python guide](https://jscott3201.github.io/rusty-bacnet/start/python/) |
| `bacnet` CLI and terminal UI | One-shot commands with JSON output; a full-screen browser, watch list and guarded writes | [CLI reference](docs/CLI.md), [terminal UI](https://jscott3201.github.io/rusty-bacnet/tui/overview/) |
| MCP server (new in 0.15.0) | Tools for AI agent hosts, live only on a profile a person authorized; read-only unless that profile offers writes | [`docs/mcp.md`](docs/mcp.md) |
| Shared endpoints | One transport that both sends requests and answers a few | [Shared endpoints](https://jscott3201.github.io/rusty-bacnet/development/shared-endpoints/) |
| Routing and BBMD | A router between networks and transports; BBMDs and foreign devices | [`bacnet-network`](docs/rust-api.md#bacnet-network) |
| Embedded `no_std` cores | MS/TP, BACnet/IP and routing without an allocator | [Embedded guide](https://jscott3201.github.io/rusty-bacnet/start/embedded/) |

## Quickstart

On loopback: [install](#install) the Python package, start the server in one
terminal, then read from it in a second.

### 1. Run a local server (Python)

Save this as `local_server.py`, run `python local_server.py`, and stop it with
**Ctrl+C**. Off loopback, give each device an instance unique on its network.

```python
# local_server.py: one simulated temperature sensor on 127.0.0.1:47808
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
    # Units 62 = degrees Celsius. Add objects before starting the server.
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

### 2. Read a property

**Python:**

```python
# read_value.py: read Analog Input 1's present value
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
        print(value.value)  # 22.5


asyncio.run(main())
```

**Rust:** create a project with `cargo new`, add the dependencies from
[Install](#rust), replace `src/main.rs` with this, then `cargo run`:

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

    // A BACnet/IP address: four IPv4 octets, then the UDP port (0xBAC0 = 47808).
    let address = [127, 0, 0, 1, 0xBA, 0xC0];
    let response = client
        .read_property(&address, oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    client.stop().await?;

    let ack = response?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("{value:?}");
    Ok(())
}
```

**CLI:**

```bash
bacnet --interface 127.0.0.1 --port 0 read 127.0.0.1:47808 ai:1 pv
bacnet --interface 127.0.0.1 --port 0 --json readm 127.0.0.1:47808 ai:1 pv,object-name
```

`ai:1` is Analog Input 1 and `pv` is Present_Value. Next, try the
[local lab](https://jscott3201.github.io/rusty-bacnet/start/local-lab/) or the
programs in [`examples/`](examples/).

## Tour

### Serve objects from Rust

A device with one analog input, using the dependencies from [Install](#rust)
(more in [`docs/rust-api.md`](docs/rust-api.md#bacnet-server)):

```rust
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_types::enums::EngineeringUnits;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vendor 0 is a placeholder: use the identifier ASHRAE assigned you.
    let identity = DeviceIdentity::new(1001, 0)?.with_name("Example air handler");
    let mut database = ObjectDatabase::new();
    database.add(Box::new(DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        ..DeviceConfig::default()
    })?))?;
    let mut supply = AnalogInputObject::new(1, "Supply air", EngineeringUnits::DEGREES_CELSIUS)?;
    supply.set_present_value(21.5);
    database.add(Box::new(supply))?;

    let mut server = BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(47809)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(database)
        .build()
        .await?;
    println!("Serving device 1001 on 127.0.0.1:47809; press Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    server.stop().await?;
    Ok(())
}
```

### Watch a value from Python

Subscribe to change-of-value (COV) notifications from the quickstart server,
which reports the current value at once, then each change
([observing changes](https://jscott3201.github.io/rusty-bacnet/guides/observe-changes/)):

```python
# watch_value.py: subscribe to Analog Input 1 and print its first notification
import asyncio
from rusty_bacnet import BACnetClient, ObjectIdentifier, ObjectType


async def main():
    sensor = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
    async with BACnetClient(
        interface="127.0.0.1", port=0, broadcast_address="127.0.0.1"
    ) as client:
        notifications = await client.cov_notifications()
        await client.subscribe_cov(
            "127.0.0.1:47808",
            subscriber_process_identifier=1,
            monitored_object_identifier=sensor,
            confirmed=False,
            lifetime=60,
        )
        async for notification in notifications:
            for change in notification.values:
                print(change["property_id"], change["value"].value)
            break


asyncio.run(main())
```

### Browse in the terminal UI

Plain `bacnet` on a terminal opens the
[terminal UI](https://jscott3201.github.io/rusty-bacnet/tui/overview/) at its
profile manager; transport flags without a profile connect at once, here on
loopback. It is read-only until you arm a write session.

```sh
bacnet
bacnet tui --interface 127.0.0.1 --port 0 --broadcast 127.0.0.1
```

### Let an AI agent read a site (MCP)

New in 0.15.0: `bacnet mcp serve` is a Model Context Protocol server that an
agent host such as Claude Code starts. It is read-only unless the profile
offers writes, and then each write needs a person's approval.
The prebuilt release downloads include it. A source build
enables it with the `mcp` feature, for example
`cargo install bacnet-cli --locked --features mcp,sc-tls`
(`sc-tls` lets it use BACnet/SC profiles).

Live traffic needs a [TUI profile](https://jscott3201.github.io/rusty-bacnet/tui/profiles/)
that a person has authorized once. Without one, `--passive` sends nothing,
lists no devices and can only decode frames.

```sh
bacnet mcp authorize --profile plant-a
bacnet mcp serve --stdio --passive
```

The agent host's configuration, here Claude Code's `.mcp.json`:

```json
{
  "mcpServers": {
    "bacnet": {
      "command": "bacnet",
      "args": ["mcp", "serve", "--stdio", "--profile", "plant-a"]
    }
  }
}
```

More: [`docs/mcp.md`](docs/mcp.md), including the profile's `[mcp]` limits.

## Install

The [installation guide](https://jscott3201.github.io/rusty-bacnet/start/installation/)
has every platform and checksum step.

- **Python** 3.11 or newer: `python -m pip install rusty-bacnet` (import
  `rusty_bacnet`).
- **CLI:** download the `bacnet-<os>-<arch>` executable from the
  [latest release](https://github.com/jscott3201/rusty-bacnet/releases/latest),
  check it against `SHA256SUMS` and put it on your `PATH` as `bacnet`, or run
  `cargo install bacnet-cli --locked --features sc-tls`.

### Rust

Add only the crates you need. These cover the examples above:

```toml
[dependencies]
bacnet-client = "0.15"
bacnet-encoding = "0.15"
bacnet-objects = "0.15"
bacnet-server = "0.15"
bacnet-types = "0.15"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal"] }
```

The 0.15 release uses edition 2024 and needs Rust 1.95 or newer
([building from source](https://jscott3201.github.io/rusty-bacnet/development/overview/#build-a-source-checkout)).

## Transports

Cargo features of `bacnet-transport`; `bacnet-client` also offers `ipv6` and
`sc-tls`, and `bacnet-server` and `bacnet-endpoint` offer `sc-tls`. The
Python package has all but Ethernet.

| Transport | Feature | Note |
|---|---|---|
| BACnet/IP | none | BBMD, foreign devices, a BBMD behind NAT, multicast |
| BACnet/IPv6 | `ipv6` | Keeps to one concrete interface and address |
| BACnet/SC | `sc-tls` | Nodes, direct connections and a hub over TLS 1.3 |
| MS/TP | `serial` | Standard and extended frames; on-wire timing not qualified |
| Ethernet | `ethernet` | Linux only; needs `CAP_NET_RAW` |

More: the [transport matrix](https://jscott3201.github.io/rusty-bacnet/reference/transports/).

## Crates

Depend on the highest layer that covers your task; the
[API reference](https://jscott3201.github.io/rusty-bacnet/reference/api/) and
[architecture notes](docs/architecture.md) have the details.

- **Codecs and cores** (`no_std`, no allocator): `bacnet-time`, `bacnet-npdu`, `bacnet-mstp-codec`, `bacnet-bip-codec`, `bacnet-mstp-core`, `bacnet-bip-core`, `bacnet-router-core`
- **Protocol** (`no_std`, allocator optional): `bacnet-types`, `bacnet-encoding`, `bacnet-services`
- **Transports and network:** `bacnet-transport`, `bacnet-network`
- **Roles:** `bacnet-objects` (also `no_std` with an allocator), `bacnet-client`, `bacnet-server`, `bacnet-endpoint-core`, `bacnet-endpoint`
- **Tools:** `bacnet-cli`; new in 0.15.0, `bacnet-ops` (shared by the CLI, TUI and MCP server) and `bacnet-mcp`
- **Python:** [`crates/rusty-bacnet`](crates/rusty-bacnet), built with [maturin](https://www.maturin.rs/)

## Find it in the docs

The website covers 0.15.0, as do the
[Python](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/python-api.md)
and [CLI](https://github.com/jscott3201/rusty-bacnet/blob/v0.15.0/docs/CLI.md)
references at the `v0.15.0` tag; [`docs/`](docs/) tracks `dev`.

| Topic | Where to look |
|---|---|
| First steps | [Choose your path](https://jscott3201.github.io/rusty-bacnet/start/choose-your-path/), [local lab](https://jscott3201.github.io/rusty-bacnet/start/local-lab/) |
| Writes | [Safe writes](https://jscott3201.github.io/rusty-bacnet/guides/safe-writes/), [mutation policy](docs/mutation-policy.md) |
| Policies and limits | [Engineering docs index](docs/README.md) |
| Upgrading | [Upgrade guide](https://jscott3201.github.io/rusty-bacnet/project/upgrading/), [changelog](CHANGELOG.md) |
| Problems | [Troubleshooting](https://jscott3201.github.io/rusty-bacnet/help/troubleshooting/) |

## Status and roadmap

Rusty BACnet is **not BTL certified**;
[What's supported](https://jscott3201.github.io/rusty-bacnet/project/support/#whats-supported)
lists what it implements. Planned work is on the
**[roadmap board](https://github.com/users/jscott3201/projects/4)**; releases
have no dates and ship when their milestone is done.

## Contributing

Patches, tests and bug reports are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md)
(pull requests, Python binding setup), [what an issue needs](https://jscott3201.github.io/rusty-bacnet/project/support/#report-a-problem)
and [CI's checks](docs/ci.md).

```bash
git clone https://github.com/jscott3201/rusty-bacnet.git
cd rusty-bacnet
cargo install cargo-nextest --locked
cargo build --locked
cargo nextest run --workspace --exclude rusty-bacnet --locked
cargo test --doc --workspace --exclude rusty-bacnet --locked
```

nextest (0.9.145 or newer) skips doctests. Report vulnerabilities privately
under the [security policy](.github/SECURITY.md).

## License

[MIT](LICENSE)
