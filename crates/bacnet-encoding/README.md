# bacnet-encoding

BACnet tag, primitive, APDU and NPDU codecs for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
Encoding a value produces bytes; it does not send a message or establish that a
peer accepted it.

## Quickstart

The tag-sink API below is new in 0.14.0:

```toml
[dependencies]
bacnet-encoding = "0.14"
```

Write a Real tag header into caller-owned storage, then verify that a header
which cannot fit leaves the existing bytes intact. This writes headers only;
the four Real value bytes announced by the first header are not appended.

```rust
use bacnet_encoding::sink::SinkFull;
use bacnet_encoding::tags::{encode_opening_tag_to_sink, encode_tag_to_sink, TagClass};

fn main() -> Result<(), SinkFull> {
    let mut storage = [0xA5; 2];
    let mut tail = &mut storage[..];
    assert_eq!(encode_tag_to_sink(&mut tail, 4, TagClass::Application, 4)?, 1);
    let refused = encode_opening_tag_to_sink(&mut tail, 15).unwrap_err();
    assert_eq!((refused.needed, refused.available), (2, 1));
    assert_eq!(tail.len(), 1);
    assert_eq!(storage, [0x44, 0xA5]);
    Ok(())
}
```

The tag-sink functions append one complete header or return `SinkFull` without
changing the sink. They do not validate every tag number or datatype-specific
length, and their atomic write guarantee does not span several encoder calls.
A variable-length value, such as a CharacterString with its header, goes out
through `EncodeSink::write_parts`, all of it or none. See
[the sink design](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/encoding-sinks.md).

## Features and allocation

The crate has three profiles. With neither `std` nor `alloc` it builds without
an allocator (#1614), for firmware with no heap.

| Profile | Features | What it has |
| --- | --- | --- |
| Hosted | `std` (default) | Everything; enables `alloc` and the dependencies' standard-library support. Adds tracing for reserved NPDU control bits. |
| `no_std` with a heap | `alloc` | Everything but the tracing. Firmware must supply a global allocator. |
| No allocator | neither | Tag decoding and the `_to_sink` encoders (tags, scalars, timestamps, strings, octet and bit strings), the primitive decoders and the borrowed views (`primitives::CharacterStringRef`, `primitives::ApplicationValueRef`), the tagged-field readers in `constructed::tagged`, and `apdu::header`, which reads and writes the header of an Unconfirmed-Request, an unsegmented Confirmed-Request or ComplexACK and an Error, and a whole Reject or Abort. The owned APDU and NPDU, segmentation, the other `constructed` codecs and every `BytesMut` encoder need `alloc`. |
| | `heapless` | Adds `sink::EncodeSink` for `heapless::Vec<u8, N>`, in any profile. |

`EncodeSink` also supports caller slices, and with `alloc` `Vec<u8>` and
`bytes::BytesMut`. Only the `alloc` profile has `bytes` in its graph.

For an alloc-only dependency build, replace the dependency block with:

```toml
[dependencies]
bacnet-encoding = { path = "../rusty-bacnet/crates/bacnet-encoding", default-features = false, features = ["alloc"] }
```

For a build without an allocator, leave out `features = ["alloc"]`: the
quickstart runs unchanged with any of the three blocks on a host. With
`alloc`, firmware also needs a global allocator and a compatible `bytes`
configuration: on targets without atomic compare-and-swap, such as ARMv6-M,
enable `bytes`'s `extra-platforms` feature and select a `portable-atomic`
backend in the firmware. Without `alloc` there is no `bytes`, so no backend to
choose. CI builds each profile for ARMv6-M, ARMv7E-M and RISC-V
(`scripts/ci/check-no-std.sh`) but doesn't run it there. Cargo features are
additive, so check whether another dependency enables `alloc` or `std`.

## API documentation

This README is included in the crate's Rust documentation. The docs.rs profile
explicitly enables `std` and `heapless`, disables implicit defaults, and renders
only the default `x86_64-unknown-linux-gnu` target. That makes the optional sink
adapter visible through its actual feature gate; it does not qualify a
bare-metal target.
