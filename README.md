# Rusty BACnet

A complete BACnet protocol stack (ASHRAE 135-2020) written in Rust, with first-class Python, Kotlin/Java, and WASM/JavaScript bindings.

[![CI](https://github.com/jscott3201/rusty-bacnet/actions/workflows/ci.yml/badge.svg)](https://github.com/jscott3201/rusty-bacnet/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Features

- **Full BACnet/IP stack** — async client and server with 30+ service types
- **5 transports** — BACnet/IP (UDP), BACnet/IPv6 (multicast), BACnet/SC (WebSocket+TLS with hub), MS/TP (serial), Ethernet (BPF)
- **64 object types** — All standard BACnet objects including Analog/Binary/MultiState I/O, Device, Schedule, Calendar, Trend Log, Notification Class, Loop, Access Control, Lighting, Life Safety, Elevator, Color, Color Temperature, and more
- **BTL compliance test harness** — 3,808 tests covering 100% of BTL Test Plan 26.1 across all 13 sections
- **Python bindings** — async client, server, and SC hub with full API parity via PyO3
- **Kotlin/Java bindings** — async client and server via UniFFI, distributed as multi-platform JAR
- **WASM/JavaScript** — BACnet/SC thin client for browsers via wasm-bindgen
- **CLI tool** — interactive shell and scripting for BACnet/IP, IPv6, and SC
- **5,500+ tests**, 0 clippy warnings, CI on Linux/macOS/Windows

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

## Quick Start (Kotlin)

Add the dependency (GitHub Packages):

```kotlin
// settings.gradle.kts
dependencyResolutionManagement {
    repositories {
        maven {
            url = uri("https://maven.pkg.github.com/jscott3201/rusty-bacnet")
            credentials {
                username = providers.gradleProperty("gpr.user").orNull ?: System.getenv("GITHUB_ACTOR")
                password = providers.gradleProperty("gpr.key").orNull ?: System.getenv("GITHUB_TOKEN")
            }
        }
    }
}

// build.gradle.kts
dependencies {
    implementation("io.github.jscott3201:bacnet-java:0.7.0")
}
```

```kotlin
import uniffi.bacnet_java.*
import kotlinx.coroutines.runBlocking

fun main() = runBlocking {
    val client = BacnetClient(
        transportType = "bip",
        address = "0.0.0.0:47808",
        broadcastAddress = "192.168.1.255:47808",
    )
    client.start()

    // Read a property
    val value = client.readProperty(
        address = "192.168.1.100:47808",
        objectType = 0u,      // analog-input
        objectInstance = 1u,
        propertyId = 85u,      // present-value
        arrayIndex = null,
    )
    println("Value: $value")

    // Discover devices
    client.whoIs(lowLimit = null, highLimit = null)
    kotlinx.coroutines.delay(2000)
    val devices = client.discoveredDevices()
    devices.forEach { println("Device ${it.deviceInstance} at ${it.address}") }

    client.stop()
}
```

## Quick Start (JavaScript/WASM)

```bash
npm install @jscott3201/bacnet-wasm
```

```javascript
import init, { BACnetScClient } from '@jscott3201/bacnet-wasm';

await init();

const client = new BACnetScClient("wss://sc-hub.example.com:443");
await client.connect(new Uint8Array([0, 1, 2, 3, 4, 5]));  // VMAC

const value = await client.readProperty(
  new Uint8Array([0, 0, 0, 0, 0, 1]),  // target VMAC
  0,   // analog-input
  1,   // instance
  85,  // present-value
);
console.log('Value:', value);

client.disconnect();
```

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
bacnet --sc --sc-url wss://hub:443 --sc-cert cert.pem --sc-key key.pem read 00:01:02:03:04:05 ai:1 pv

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
  bacnet-transport/   BIP, BIP6, BACnet/SC + Hub, MS/TP, BBMD, Ethernet
  bacnet-network/     Network layer routing, router tables
  bacnet-client/      Async client with TSM, segmentation, discovery
  bacnet-objects/     BACnetObject trait, ObjectDatabase, 64 object types
  bacnet-server/      Async server (RP/WP/RPM/WPM/COV/Events/DCC/CreateObject/TimeSynchronization)
  bacnet-btl/         BTL compliance test harness (BTL Test Plan 26.1, 3808 tests, all 13 sections)
  rusty-bacnet/       Python bindings via PyO3 (client, server, hub)
  bacnet-java/        Kotlin/Java bindings via UniFFI (client, server)
  bacnet-wasm/        WASM/JavaScript BACnet/SC thin client
  bacnet-cli/         CLI tool with interactive shell
java/                 Gradle build for multi-platform JAR
benchmarks/           Criterion benchmarks (9 suites) + Docker stress topology
examples/             Rust, Python, Kotlin, and Docker examples
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
| AcknowledgeAlarm | ✓ | — |
| GetAlarmSummary | ✓ | ✓ |
| GetEnrollmentSummary | ✓ | ✓ |
| GetEventInformation | ✓ | ✓ |
| LifeSafetyOperation | ✓ | ✓ |
| ReadRange | ✓ | — |
| AtomicReadFile / AtomicWriteFile | ✓ | ✓ |
| AddListElement / RemoveListElement | ✓ | — |
| ConfirmedPrivateTransfer / UnconfirmedPrivateTransfer | ✓ | — |
| ConfirmedTextMessage / UnconfirmedTextMessage | ✓ | ✓ |
| WriteGroup | ✓ | ✓ |
| VTOpen / VTClose / VTData | ✓ | — |
| AuditNotification (confirmed + unconfirmed) | ✓ | — |
| AuditLogQuery | ✓ | — |
| TimeSynchronization / UTCTimeSynchronization | ✓ | ✓ |

## Transports

| Transport | Platforms | Feature Flag |
|-----------|-----------|-------------|
| BACnet/IP (UDP/IPv4) | All | default |
| BACnet/IPv6 (UDP multicast) | All | `ipv6` |
| BACnet/SC (WebSocket + TLS) | All | `sc-tls` |
| BACnet/SC Hub (TLS relay) | All | `sc-tls` |
| MS/TP (serial token-passing) | Linux | `serial` |
| Ethernet (802.3 via BPF) | Linux | `ethernet` |

## Python Bindings

The `rusty-bacnet` crate provides full Python API parity:

- **11 enum types** with named constants: `ObjectType`, `PropertyIdentifier`, `ErrorClass`, `ErrorCode`, `EnableDisable`, `ReinitializedState`, `Segmentation`, `LifeSafetyOperation`, `EventState`, `EventType`, `MessagePriority`
- **42 client methods** covering all services above (plus context manager and lifecycle)
- **6 server runtime methods**: `start`, `stop`, `local_address`, `read_property`, `write_property_local`, `comm_state`
- **61 server object types** via `add_*` methods
- **SC hub management**: `ScHub` class for running a BACnet/SC hub
- **COV async iterator**: `async for notif in client.cov_notifications()`
- **Typed exceptions**: `BacnetError`, `BacnetProtocolError`, `BacnetTimeoutError`, `BacnetRejectError`, `BacnetAbortError`

## BTL Compliance Testing

The `bacnet-btl` crate provides a full BTL Test Plan 26.1 compliance test harness:

```bash
# Run all 3,808 BTL tests against in-process server (<1s)
cargo build -p bacnet-btl && ./target/debug/bacnet-test self-test

# Run BTL tests against an external BACnet device
./target/debug/bacnet-test run --target 192.168.1.100:47808

# Run BTL tests over BACnet/SC
./target/debug/bacnet-test run --target aa:bb:cc:dd:ee:ff \
    --sc-hub=wss://hub:47809 --sc-no-verify

# Start a standalone BTL server (all 64 object types)
./target/debug/bacnet-test serve --interface 0.0.0.0 --port 47808

# Docker SC topology (hub + server + tester)
cd examples/docker
docker compose -f docker-compose.btl.yml up btl-self-test
```

Coverage: 100% of all 13 BTL Test Plan sections (Basic BACnet, Objects, Data Sharing, Alarm & Event, Scheduling, Trending, Device Management, Data Link Layer, Network Management, Gateway, Network Security, Audit Reporting, Web Services).

## Development

```bash
# Run workspace tests (1,701 tests)
cargo test --workspace --exclude rusty-bacnet --exclude bacnet-wasm

# Run BTL compliance tests (3,808 tests)
cargo build -p bacnet-btl && ./target/debug/bacnet-test self-test

# Check formatting
cargo fmt --all --check

# Lint (0 warnings required)
RUSTFLAGS="-Dwarnings" cargo clippy --workspace --exclude rusty-bacnet --all-targets

# Check Python bindings compile
cargo check -p rusty-bacnet --tests

# Check WASM bindings compile
cargo check -p bacnet-wasm --target wasm32-unknown-unknown

# Build Java/Kotlin JAR (local platform only)
cd java && ./build-local.sh --release

# License/advisory checks
cargo deny check
```

Minimum Rust version: 1.93

## Documentation

- [CLI Reference](docs/CLI.md)
- [Rust API Reference](docs/rust-api.md)
- [Python API Reference](docs/python-api.md)
- [Benchmark Results](Benchmarks.md)
- [Examples](examples/)

## License

MIT
