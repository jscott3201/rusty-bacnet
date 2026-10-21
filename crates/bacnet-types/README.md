# bacnet-types

BACnet identifiers, enum values, addresses, property values and errors for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
These are protocol data types; this crate does not open a transport or run a device.

## Quickstart from development source

The dependency blocks below use a checkout named `rusty-bacnet` beside your
application directory. They describe the development source, not the contents
of a published crate version. Adjust the relative path for another layout.

```toml
[dependencies]
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
```

Construct an identifier for a concrete object, check its wire bytes, and decode
it again. `new_addressable` rejects the wildcard instance as well as values
outside the field widths; `new` and `decode` also support the valid wire wildcard.

```rust
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

fn main() -> Result<(), bacnet_types::error::Error> {
    let object = ObjectIdentifier::new_addressable(ObjectType::ANALOG_INPUT, 7)?;
    let wire = object.encode();
    assert_eq!(wire, [0x00, 0x00, 0x00, 0x07]);
    assert_eq!(ObjectIdentifier::decode(&wire)?, object);
    assert_eq!(object.instance_number(), 7);
    assert!(ObjectIdentifier::new(
        ObjectType::DEVICE,
        ObjectIdentifier::MAX_INSTANCE + 1,
    ).is_err());
    Ok(())
}
```

## Features and allocation

The crate has three profiles. With neither `std` nor `alloc` it builds without
an allocator (#1614), for firmware with no heap.

| Profile | Features | What it has |
| --- | --- | --- |
| Hosted | `std` (default) | Everything, including `error::Error::Transport` and `bip_port`. Enables `alloc`. |
| `no_std` with a heap | `alloc` | Everything but those two. Firmware must supply a global allocator. |
| No allocator | neither | The enums, `ObjectIdentifier`, `Date`, `Time`, `BACnetTimeStamp`, the bit-string flag types and the error type. The owned values (`PropertyValue`, the `constructed` types, `MacAddr`, the `calendar` matching) need `alloc`. |
| | `serde` | Deserialization for `enums::ObjectType` and `enums::PropertyIdentifier`; enables `alloc`, without selecting `std`. |

Without `alloc`, `error::Error` keeps only the variants that own nothing. A
decoder's refusal is `Error::Decoding(DecodeError)`, an encoder's
`Error::Encode(EncodeError)`, a checked constructor's `Error::Range(RangeError)`
and a segmenter's `Error::Segmentation(SegmentationError)`: each carries static
labels or numbers and builds its text only when it is displayed (#1646, #1802),
so firmware that never displays one links no formatting for it. Every decoder
and every codec-crate encoder reports these forms (#1776, #1802). With `alloc`,
the owned `Error::Encoding`, `Error::OutOfRange` and `Error::Structured` return.

Enum newtypes support `FromStr` for their named constants in common case styles,
such as `analog-input` and `ANALOG_INPUT`. The normal runtime dependencies are
`bitflags` and `thiserror`, plus `smallvec` with `alloc` and `serde` when
selected.

For an alloc-only dependency build, replace the dependency block with:

```toml
[dependencies]
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types", default-features = false, features = ["alloc"] }
```

For a build without an allocator, leave out `features = ["alloc"]`. The
quickstart runs on a host with any of the three blocks; CI also builds each
profile for ARMv6-M, ARMv7E-M and RISC-V (`scripts/ci/check-no-std.sh`), which
shows the code needs no `std`, and no heap without `alloc`, but doesn't run it
there. A firmware application supplies its runtime, and with `alloc` its
allocator. Cargo features are additive: another dependency can enable `alloc`
or `std` again, so inspect the application's resolved feature graph.

## API documentation

This README is also the crate's Rust documentation landing page. The docs.rs
profile explicitly enables `std` and `serde`, disables implicit defaults, and
renders only the default `x86_64-unknown-linux-gnu` target. Documented items
follow their real feature gates; this profile does not establish firmware
qualification. The library's module pages describe the individual types.
