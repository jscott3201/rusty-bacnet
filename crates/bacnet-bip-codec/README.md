# bacnet-bip-codec

The BVLL codec of [rusty-bacnet](https://github.com/jscott3201/rusty-bacnet):
the wire format of BACnet/IP (Annex J of ASHRAE 135) at the crate root and
of BACnet/IPv6 (Annex U) in its `ipv6` module, and nothing above it.
Sockets, BBMD forwarding and foreign-device registration belong to the
layer that uses it, such as `bacnet-bip-core`.

- `#![no_std]`, no allocator and no dependencies in the default build.
- The twelve B/IP messages and the twelve B/IPv6 ones, as `Bvll` and
  `Bvll6` values that borrow their NPDU or table from the datagram.
- Decoding is strict about shape (BVLC type, a Length field equal to the
  datagram, a payload the function's layout allows) and lenient about
  values; encoding refuses what the Length field can't state and a
  BVLC-Result with an undefined code.
- `BvlcFunction`, `BvlcResultCode` and their B/IPv6 counterparts are open
  sets: a constant for each code the standard defines, and `from_raw` for
  any other, so a decoder can report a code it doesn't know.
- BDT and FDT entries, read through iterators straight from the wire.
- A B/IP address is a plain `SocketAddrV4`, since all of it travels; a
  B/IPv6 address is a `Bip6Address`, which keeps the 18 octets that travel
  and none of a `SocketAddrV6`'s flow label or scope ID.

## Features

None is on by default, and none needs `std`. Each adds a `BvllSink`, which
both encoders write to:

- `alloc`: for `Vec<u8>`.
- `bytes`: for `bytes::BytesMut`; implies `alloc`.
- `heapless`: for `heapless::Vec<u8, N>`.

## Example

```rust
use core::net::{Ipv4Addr, SocketAddrV4};
use bacnet_bip_codec::{decode, BvlcFunction, Bvll, BvllFrame};

// Encode an Original-Broadcast-NPDU into caller storage.
let npdu = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF];
let mut wire = [0u8; 64];
let length = Bvll::OriginalBroadcastNpdu(&npdu).encode_into(&mut wire).unwrap();
assert_eq!(wire[..4], [0x81, 0x0B, 0x00, 0x0A]);
assert_eq!(decode(&wire[..length]), Ok(Bvll::OriginalBroadcastNpdu(&npdu)));

// A Forwarded-NPDU carries its originator's address.
let wire = [0x81, 0x04, 0x00, 0x0C, 192, 168, 1, 20, 0xBA, 0xC0, 0x01, 0x00];
let origin = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 20), 47808);
assert_eq!(decode(&wire).unwrap().origin(), Some(origin));

// A function the standard doesn't define still has a frame, so a
// receiver can see what arrived before refusing it.
let frame = BvllFrame::decode(&[0x81, 0x0C, 0x00, 0x04]).unwrap();
assert_eq!(frame.function, BvlcFunction::from_raw(0x0C));
assert!(!frame.function.is_defined());
assert!(frame.message().is_err());
```

## License

MIT
