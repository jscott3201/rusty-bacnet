# bacnet-services

BACnet service request and response parameter codecs for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
Each module provides structures and byte encoders/decoders for a service family.
Property values at constructed boundaries remain raw bytes for the application
to interpret. This crate does not perform network requests or operate a device.

## Quickstart from development source

Use a checkout named `rusty-bacnet` beside your application directory. These
blocks describe development-source dependencies, not the contents of a published
release. The example imports `bacnet-types` and `bytes` directly, so both are
listed as application dependencies too. Adjust the paths for another layout.

```toml
[dependencies]
bacnet-services = { path = "../rusty-bacnet/crates/bacnet-services" }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
bytes = "1"
```

Encode and decode ReadProperty service parameters. These are only the service
body: no confirmed-request APDU, NPDU, transport frame or device interaction is
created by this example.

```rust
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

fn main() -> Result<(), bacnet_types::error::Error> {
    let request = ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new_addressable(ObjectType::ANALOG_INPUT, 7)?,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut storage = [0; 16];
    let mut tail = &mut storage[..];
    let written = request.encode_to_sink(&mut tail).expect("16 octets hold the request");
    let wire = &storage[..written];
    assert_eq!(wire, &[0x0C, 0, 0, 0, 7, 0x19, 85]);
    assert_eq!(ReadPropertyRequest::decode(wire)?, request);
    assert!(ReadPropertyRequest::decode(&wire[..1]).is_err());
    Ok(())
}
```

With `alloc`, `request.encode(&mut wire)` appends the same octets to a
`bytes::BytesMut`.

## Without an allocator

Who-Is, I-Am and ReadProperty (request and ACK) encode into the caller's
storage and decode from borrowed octets with neither `std` nor `alloc`
(#1614). `ReadPropertyAckRef` borrows the property value in place of the owned
ACK's `Vec`:

```rust
use bacnet_services::read_property::ReadPropertyAckRef;
use bacnet_services::who_is::{DeviceInstanceRange, WhoIsRequest};

fn main() -> Result<(), bacnet_types::error::Error> {
    let mut storage = [0; 16];
    let mut tail = &mut storage[..];
    let who_is = WhoIsRequest {
        range: Some(DeviceInstanceRange::new(1000, 2000)?),
    };
    let written = who_is.encode_to_sink(&mut tail).expect("16 octets hold a Who-Is");
    assert_eq!(storage[..written], [0x0A, 0x03, 0xE8, 0x1A, 0x07, 0xD0]);

    // [0] analog input 1, [1] Present_Value, [3] REAL 72.5.
    let ack = [0x0C, 0, 0, 0, 1, 0x19, 0x55, 0x3E, 0x44, 0x42, 0x91, 0, 0, 0x3F];
    let read = ReadPropertyAckRef::decode(&ack)?;
    assert_eq!(read.property_value, &[0x44, 0x42, 0x91, 0, 0]);
    Ok(())
}
```

## Features and allocation

The crate has three profiles. With neither `std` nor `alloc` it builds without
an allocator, with the services above only.

| Profile | Features | What it has |
| --- | --- | --- |
| Hosted | `std` (default) | Every service; enables `alloc` and standard-library support in `bacnet-types`, `bacnet-encoding` and `bytes`. |
| `no_std` with a heap | `alloc` | Every service; enables the foundation crates' `alloc` profiles. Firmware must supply a global allocator. |
| No allocator | neither | `who_is` and `read_property`: their `encode_to_sink` and `decode`, and `ReadPropertyAckRef`. The `BytesMut` encoders, the owned `ReadPropertyACK` and every other service need `alloc`. |

For alloc-only dependencies, replace the entire dependency block, disabling
defaults on every directly listed foundation crate and on `bytes`:

```toml
[dependencies]
bacnet-services = { path = "../rusty-bacnet/crates/bacnet-services", default-features = false, features = ["alloc"] }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types", default-features = false, features = ["alloc"] }
bytes = { version = "1", default-features = false }
```

The same example executes on a host with either block. Firmware additionally
supplies an allocator and a compatible `bytes` atomic backend. A target without
atomic compare-and-swap, such as ARMv6-M, needs `bytes`'s `extra-platforms`
feature and a firmware-selected `portable-atomic` backend. The block above does
not choose that backend. For a build without an allocator, depend on
`bacnet-services` and `bacnet-types` with `default-features = false` and no
features, and leave out `bytes`, which that profile doesn't use. CI builds each
profile for ARMv6-M, ARMv7E-M and RISC-V (`scripts/ci/check-no-std.sh`) but
doesn't run it there. Cargo features are additive; another dependency can still
enable `alloc` or `std`.

## API documentation

This README is also the crate's Rust documentation landing page. The docs.rs
profile explicitly enables `std`, disables implicit defaults, and renders only
the default `x86_64-unknown-linux-gnu` target. It exposes the actual hosted API
without bypassing feature gates or claiming firmware qualification.
