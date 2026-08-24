# Rusty BACnet

A BACnet protocol stack (ASHRAE 135-2020) written in Rust, with Python bindings.

[![CI](https://github.com/jscott3201/rusty-bacnet/actions/workflows/ci.yml/badge.svg)](https://github.com/jscott3201/rusty-bacnet/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Features

- **BACnet/IP implementation** — async client and server paths with 30+ service modules under conformance review
- **Transport implementations** — BACnet/IP (UDP), BACnet/IPv6 (multicast), BACnet/SC (WebSocket+TLS with hub), MS/TP (serial), Ethernet (BPF); see the conformance ledger for current evidence status
- **BACnet object implementations** — object structs and server helpers for common and extended BACnet object families, with clause-level evidence tracked in the ledger
- **Python bindings** — async client, server, and SC hub bindings via PyO3
- **CLI tool** — interactive shell and scripting for BACnet/IP, IPv6, and SC
- **5,500+ tests** and CI on Linux/macOS/Windows
- **Conformance evidence** — draft Standard 135-2020 ledger and support summaries in [`docs/conformance/`](docs/conformance/standard-135-2020-ledger.md)

## Quick Start (Python)

```bash
pip install rusty-bacnet
```

```python
import asyncio
from rusty_bacnet import (
    BACnetClient, ObjectType, ObjectIdentifier,
    PropertyIdentifier, PropertyValue,
)

async def main():
    async with BACnetClient() as client:
        oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

        # Read a property
        value = await client.read_property(
            "192.168.1.100:47808", oid, PropertyIdentifier.PRESENT_VALUE
        )
        print(f"{value.tag}: {value.value}")  # real: 72.5

        # Write a property
        await client.write_property(
            "192.168.1.100:47808", oid, PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.real(75.0), priority=8,
        )

        # Discover devices
        await client.who_is()
        await asyncio.sleep(2)
        for dev in await client.discovered_devices():
            print(f"Device {dev.object_identifier.instance} vendor={dev.vendor_id}")

        # Read multiple properties at once
        results = await client.read_property_multiple("192.168.1.100:47808", [
            (oid, [
                (PropertyIdentifier.PRESENT_VALUE, None),
                (PropertyIdentifier.OBJECT_NAME, None),
            ]),
        ])

asyncio.run(main())
```

## Quick Start (Rust)

```toml
[dependencies]
bacnet-client = "0.7"
bacnet-types = "0.7"
bacnet-encoding = "0.7"
tokio = { version = "1", features = ["full"] }
```

```rust
use bacnet_client::client::BACnetClient;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_encoding::primitives::decode_application_value;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::UNSPECIFIED)
        .port(0xBAC0)
        .broadcast_address(Ipv4Addr::BROADCAST)
        .build()
        .await?;

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let mac = &[192, 168, 1, 100, 0xBA, 0xC0]; // IP:port as bytes

    let ack = client
        .read_property(mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await?;

    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("Value: {:?}", value);

    Ok(())
}
```

## Companion projects

- **[`rusty-bacnet-mcp`](https://github.com/jscott3201/rusty-bacnet-mcp)** — HTTP REST API + MCP server gateway. 10 MCP tools for AI-driven BACnet interaction, REST endpoints under `/api/v1/`, bearer-token auth, read-only mode, built-in BACnet reference knowledge base.
- **[`rusty-bacnet-btl-harness`](https://github.com/jscott3201/rusty-bacnet-btl-harness)** — external BTL Test Plan 26.1 harness project. Formal support status is tracked separately in the conformance ledger.

Both repos consume the published `bacnet-*` crates from this workspace.

## Running a Server (Python)

```python
import asyncio
from rusty_bacnet import BACnetServer, ObjectType, ObjectIdentifier, PropertyIdentifier, PropertyValue

async def main():
    server = BACnetServer(device_instance=1234, device_name="My Device")
    server.add_analog_input(instance=1, name="Zone Temp", units=62, present_value=72.5)
    server.add_binary_value(instance=1, name="Override")
    await server.start()

    # Read/write local objects at runtime
    value = await server.read_property(
        ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
        PropertyIdentifier.PRESENT_VALUE,
    )
    print(f"Current temp: {value.value}")

    await server.write_property_local(
        ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
        PropertyIdentifier.PRESENT_VALUE,
        PropertyValue.real(73.5),
    )

    await asyncio.sleep(3600)
    await server.stop()

asyncio.run(main())
```

## BACnet/SC with Hub (Python)

```python
import asyncio
from rusty_bacnet import BACnetClient, BACnetServer, ScHub

async def main():
    # Start an SC hub (TLS WebSocket relay)
    hub = ScHub(
        listen="127.0.0.1:0",
        cert="hub-cert.pem", key="hub-key.pem",
        ca_cert="ca-cert.pem",
        vmac=b"\xff\x00\x00\x00\x00\x01",
    )
    await hub.start()
    hub_url = await hub.url()  # "wss://127.0.0.1:<port>"

    # Start a server connected to the hub
    server = BACnetServer(
        device_instance=1000, device_name="SC Device",
        transport="sc", sc_hub=hub_url,
        sc_vmac=b"\x00\x01\x02\x03\x04\x05",
        sc_ca_cert="ca-cert.pem",
        sc_client_cert="server-cert.pem", sc_client_key="server-key.pem",
    )
    server.add_analog_input(instance=1, name="Temp", units=62, present_value=72.5)
    await server.start()

    # Connect a client to the same hub
    async with BACnetClient(
        transport="sc", sc_hub=hub_url,
        sc_vmac=b"\x00\x02\x03\x04\x05\x06",
        sc_ca_cert="ca-cert.pem",
        sc_client_cert="client-cert.pem", sc_client_key="client-key.pem",
    ) as client:
        # Address server by its VMAC (hex-colon notation)
        value = await client.read_property(
            "00:01:02:03:04:05",
            ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
            PropertyIdentifier.PRESENT_VALUE,
        )
        print(f"SC read: {value.value}")

    await server.stop()
    await hub.stop()

asyncio.run(main())
```

For Annex AB production deployments, configure the hub with a trusted issuer CA
(`ca_cert`) and configure every SC node with its own certificate/key pair.
Omitting `ca_cert` leaves the hub in server-auth-only example mode, which is not
claimed as BACnet/SC mTLS conformance evidence.

## CLI Tool

The `bacnet-cli` crate provides an interactive shell and one-shot commands for BACnet diagnostics:

```bash
cargo install bacnet-cli

# Interactive shell
bacnet shell

# Discover devices
bacnet discover
bacnet discover 1000-2000

# Read/write properties (shorthand object and property names)
bacnet read 192.168.1.100 ai:1 pv
bacnet write 192.168.1.100 av:1 pv 72.5 --priority 8

# Read multiple properties
bacnet readm 192.168.1.100 ai:1 pv,object-name ao:1 pv

# Subscribe to COV notifications
bacnet subscribe 192.168.1.100 ai:1 --lifetime 300

# BBMD management
bacnet bdt 192.168.1.1           # Read broadcast distribution table
bacnet fdt 192.168.1.1           # Read foreign device table
bacnet register 192.168.1.1 --ttl 300

# Packet capture and analysis (requires pcap feature)
bacnet capture                              # live capture, summary mode
bacnet capture --device eth0 --decode       # full protocol decode
bacnet capture --save traffic.pcap --quiet  # headless recording
bacnet capture --read traffic.pcap          # offline analysis
bacnet capture --filter "host 10.0.0.1"    # additional BPF filter

# Device management
bacnet time-sync 192.168.1.100 --utc
bacnet create-object 192.168.1.100 av:100
bacnet delete-object 192.168.1.100 av:100

# File transfer
bacnet file-read 192.168.1.100 1 --count 4096 --output data.bin
bacnet file-write 192.168.1.100 1 firmware.bin

# BACnet/IPv6
bacnet --ipv6 discover
bacnet --ipv6 read [fe80::1]:47808 ai:1 pv

# BACnet/SC
bacnet --sc --sc-url wss://hub:443 --sc-cert cert.pem --sc-key key.pem --sc-vmac 22:01:02:03:04:05 --sc-device-uuid 00112233-4455-6677-8899-aabbccddeeff read 00:01:02:03:04:05 ai:1 pv

# Output formats
bacnet --json discover           # JSON output (default when piped)
bacnet -vvv read 192.168.1.100 ai:1 pv  # Debug logging
```

See [CLI Reference](docs/CLI.md) for full documentation, including all commands, shorthand notation, and pre-built binary downloads.

## Workspace Structure

```
crates/
  bacnet-types/       Enums, primitives, errors
  bacnet-encoding/    ASN.1 tags, APDU/NPDU codec, segmentation
  bacnet-services/    30+ services across 24 modules (RP, WP, RPM, WPM, COV, etc.)
  bacnet-transport/   BIP, BIP6, BACnet/SC + Hub, MS/TP, BBMD, Ethernet, Loopback
  bacnet-network/     Network layer routing, router tables
  bacnet-client/      Async client with TSM, segmentation, discovery
  bacnet-objects/     BACnetObject trait, ObjectDatabase, object implementations
  bacnet-server/      Async server (RP/WP/RPM/WPM/COV/Events/DCC/CreateObject/TimeSynchronization)
  rusty-bacnet/       Python bindings via PyO3 (client, server, hub)
  bacnet-cli/         CLI tool with interactive shell
benchmarks/           Criterion benchmarks (9 suites) + Docker stress topology
examples/             Rust, Python, and Docker examples
docs/                 API documentation and design plans
```

## Supported Services

| Service | Client | Server |
|---------|--------|--------|
| ReadProperty | ✓ | ✓ |
| WriteProperty | ✓ | ✓ |
| ReadPropertyMultiple | ✓ | ✓ |
| WritePropertyMultiple | ✓ | ✓ |
| SubscribeCOV / UnsubscribeCOV | ✓ | ✓ |
| SubscribeCOVProperty | ✓ | ✓ |
| SubscribeCOVPropertyMultiple | ✓ | ✓ |
| COV Notifications (confirmed + unconfirmed) | ✓ | ✓ |
| WhoIs / IAm | ✓ | ✓ |
| WhoHas / IHave | ✓ | ✓ |
| WhoAmI | ✓ | — |
| CreateObject | ✓ | ✓ |
| DeleteObject | ✓ | ✓ |
| DeviceCommunicationControl | ✓ | ✓ |
| ReinitializeDevice | ✓ | ✓ |
| AcknowledgeAlarm | ✓ | ✓ |
| GetAlarmSummary | ✓ | ✓ |
| GetEnrollmentSummary | ✓ | ✓ |
| GetEventInformation | ✓ | ✓ |
| LifeSafetyOperation | ✓ | Silence/unsilence (authorization policy required) |
| ReadRange | ✓ | ✓ |
| AtomicReadFile / AtomicWriteFile | ✓ | ✓ |
| AddListElement / RemoveListElement | ✓ | ✓ |
| ConfirmedPrivateTransfer / UnconfirmedPrivateTransfer | ✓ | — |
| ConfirmedTextMessage / UnconfirmedTextMessage | ✓ | ✓ |
| WriteGroup | ✓ | — |
| VTOpen / VTClose / VTData | ✓ | — |
| AuditNotification (confirmed + unconfirmed) | ✓ | — |
| AuditLogQuery | ✓ | — |
| TimeSynchronization / UTCTimeSynchronization | ✓ | ✓ |

## Transports

The table below lists implemented transport code paths. Clause-level support evidence is tracked in the draft [conformance ledger](docs/conformance/standard-135-2020-ledger.md).

| Transport | Platforms | Feature Flag |
|-----------|-----------|-------------|
| BACnet/IP (UDP/IPv4) | All | default |
| BACnet/IPv6 (UDP multicast) | All | `ipv6` |
| BACnet/SC (WebSocket + TLS) | All | `sc-tls` |
| BACnet/SC Hub (TLS relay) | All | `sc-tls` |
| MS/TP (serial token-passing) | Linux | `serial` |
| Ethernet (802.3 via BPF) | Linux | `ethernet` |

Annex J NAT traversal and IPv4 BACnet/IP multicast (B/IP-M) are not claimed by the current BACnet/IP transport; their support direction is tracked in the conformance ledger.

## Python Bindings

The `rusty-bacnet` crate provides Python bindings for the core client, server, and hub APIs:

- **11 enum types** with named constants: `ObjectType`, `PropertyIdentifier`, `ErrorClass`, `ErrorCode`, `EnableDisable`, `ReinitializedState`, `Segmentation`, `LifeSafetyOperation`, `EventState`, `EventType`, `MessagePriority`
- **42 client methods** covering all services above (plus context manager and lifecycle)
- **6 server runtime methods**: `start`, `stop`, `local_address`, `read_property`, `write_property_local`, `comm_state`
- **Server object helpers** via `add_*` methods
- **SC hub management**: `ScHub` class for running a BACnet/SC hub
- **COV async iterator**: `async for notif in client.cov_notifications()`
- **Typed exceptions**: `BacnetError`, `BacnetProtocolError`, `BacnetTimeoutError`, `BacnetRejectError`, `BacnetAbortError`

## Development

```bash
# Run workspace tests (1,800+ tests)
cargo test --workspace --exclude rusty-bacnet

# Check formatting
cargo fmt --all --check

# Lint (deny-level lints set in [workspace.lints]; missing_docs is warn-level)
cargo clippy --workspace --exclude rusty-bacnet --all-targets --locked

# Check Python bindings compile
cargo check -p rusty-bacnet --tests

# License/advisory checks
cargo deny check
```

Minimum Rust version: 1.93

## Documentation

- [Rust API Reference](docs/rust-api.md) — all 8 published crates with examples
- [Python API Reference](docs/python-api.md) — async client, server, object helper, and SC hub bindings
- [CLI Reference](docs/CLI.md) — interactive shell and one-shot commands
- [Benchmark Results](Benchmarks.md) — 9 suites with throughput, latency, and memory
- [Conformance Ledger](docs/conformance/standard-135-2020-ledger.md) — draft Standard 135-2020 evidence map
- [Architecture Guide](docs/architecture.md) — crate graph, packet flow, concurrency model
- [Changelog](CHANGELOG.md)
- [Examples](examples/)

## License

MIT
