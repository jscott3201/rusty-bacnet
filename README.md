# Rusty BACnet

A complete BACnet protocol stack (ASHRAE 135-2020) written in Rust, with first-class Python bindings via PyO3.

[![CI](https://github.com/jscott3201/rusty-bacnet/actions/workflows/ci.yml/badge.svg)](https://github.com/jscott3201/rusty-bacnet/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Features

- **Full BACnet/IP stack** — async client and server with 30+ service types
- **5 transports** — BACnet/IP (UDP), BACnet/IPv6 (multicast), BACnet/SC (WebSocket+TLS with hub), MS/TP (serial), Ethernet (BPF)
- **62 object types** — All standard BACnet objects including Analog/Binary/MultiState I/O, Device, Schedule, Calendar, Trend Log, Notification Class, Loop, Access Control, Lighting, Life Safety, Elevator, and more
- **Python bindings** — async client, server, and SC hub with full API parity via PyO3
- **1682 tests**, 0 clippy warnings, CI on Linux/macOS/Windows

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
bacnet-client = "0.1"
bacnet-types = "0.1"
bacnet-encoding = "0.1"
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

## Workspace Structure

```
crates/
  bacnet-types/       Enums, primitives, errors
  bacnet-encoding/    ASN.1 tags, APDU/NPDU codec, segmentation
  bacnet-services/    30+ services across 24 modules (RP, WP, RPM, WPM, COV, etc.)
  bacnet-transport/   BIP, BIP6, BACnet/SC + Hub, MS/TP, BBMD, Ethernet
  bacnet-network/     Network layer routing, router tables
  bacnet-client/      Async client with TSM, segmentation, discovery
  bacnet-objects/     BACnetObject trait, ObjectDatabase, 62 object types
  bacnet-server/      Async server (RP/WP/RPM/WPM/COV/Events/DCC)
  rusty-bacnet/       Python bindings via PyO3 (client, server, hub)
benchmarks/           Criterion benchmarks (9 suites) + Python mixed-mode
examples/             Rust, Python, and Docker examples
docs/                 API documentation
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
| SubscribeCOVPropertyMultiple | ✓ | — |
| COV Notifications (confirmed + unconfirmed) | ✓ | ✓ |
| WhoIs / IAm | ✓ | ✓ |
| WhoHas / IHave | ✓ | ✓ |
| WhoAmI | ✓ | — |
| CreateObject | ✓ | ✓ |
| DeleteObject | ✓ | ✓ |
| DeviceCommunicationControl | ✓ | ✓ |
| ReinitializeDevice | ✓ | ✓ |
| AcknowledgeAlarm | ✓ | — |
| GetAlarmSummary | ✓ | — |
| GetEnrollmentSummary | ✓ | — |
| GetEventInformation | ✓ | ✓ |
| LifeSafetyOperation | ✓ | — |
| ReadRange | ✓ | — |
| AtomicReadFile / AtomicWriteFile | ✓ | — |
| AddListElement / RemoveListElement | ✓ | — |
| ConfirmedPrivateTransfer / UnconfirmedPrivateTransfer | ✓ | — |
| ConfirmedTextMessage / UnconfirmedTextMessage | ✓ | — |
| WriteGroup | ✓ | — |
| VTOpen / VTClose / VTData | ✓ | — |
| AuditNotification (confirmed + unconfirmed) | ✓ | — |
| AuditLogQuery | ✓ | — |
| TimeSynchronization | — | ✓ |

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

## Development

```bash
# Run tests (1682 tests)
cargo test --workspace --exclude rusty-bacnet

# Check formatting
cargo fmt --all --check

# Lint (0 warnings required)
RUSTFLAGS="-Dwarnings" cargo clippy --workspace --exclude rusty-bacnet --all-targets

# Check Python bindings compile
cargo check -p rusty-bacnet --tests

# License/advisory checks
cargo deny check
```

Minimum Rust version: 1.93

## Documentation

- [Rust API Reference](docs/rust-api.md)
- [Python API Reference](docs/python-api.md)
- [Benchmark Results](Benchmarks.md)
- [Examples](examples/)

## License

MIT
