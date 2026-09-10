---
title: "Embed the Rust client"
description: "Use a small set of aligned BACnet crates for a directed property read."
---

The v0.11.0 workspace declares a minimum Rust version of **1.93**. Use a compatible stable toolchain; the website's Node tooling is unrelated to the Rust library minimum.

## Start a small client

```sh
cargo new first-read
cd first-read
```

Add these entries to the generated `Cargo.toml` dependency section:

```toml
[dependencies]
bacnet-client = "=0.11.0"
bacnet-types = "=0.11.0"
bacnet-encoding = "=0.11.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The exact BACnet versions make the tutorial's release scope explicit. Your application can adopt its own version policy after migration and compatibility review.

## Read one property

Replace `src/main.rs` with the following. It expects an explicit remote IPv4 address and UDP port; analog input 1 must exist on that device.

```rust
use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target: SocketAddrV4 = std::env::args()
        .nth(1)
        .ok_or("usage: first-read <IPv4:port>")?
        .parse()?;
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::UNSPECIFIED)
        .port(0)
        .broadcast_address(Ipv4Addr::BROADCAST)
        .build()
        .await?;

    let [a, b, c, d] = target.ip().octets();
    let [hi, lo] = target.port().to_be_bytes();
    let address = [a, b, c, d, hi, lo];
    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    let ack = client
        .read_property(&address, object, PropertyIdentifier::PRESENT_VALUE, None)
        .await?;
    let (value, _) = decode_application_value(&ack.property_value, 0)?;
    println!("{value:?}");
    Ok(())
}

```

```sh
cargo run -- 192.168.1.100:47808
```

This example uses an ephemeral local port and directed traffic. It is not a broadcast-discovery or server configuration example. The displayed value is typed BACnet data; consult the object's name and units before interpreting it.

## Know the boundaries

The Rust client returns encoded property data that this example decodes. Do not silently treat every property as a float, and do not assume that every complex BACnet value is interchangeable with a primitive value.

Transport features live on the crates that implement or expose them. Enabling a feature in one crate does not prove that a particular executable exposes it. Consult the manifests and [transport guide](/rusty-bacnet/reference/transports/).

## Build beyond the example

Use the server and object crates for a hosted device model, and the transport/network layers where your application needs them. A library building successfully does not establish a combined client/server endpoint profile or full conformance. Start from the current examples and API documentation rather than copying an older README dependency block.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[Workspace versions and minimum](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/Cargo.toml) · [Rust API guide](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/rust-api.md) · [Existing client example](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/README.md).
