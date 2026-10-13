# bacnet-mstp-codec

The MS/TP frame codec of
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet): the wire format
of Clause 9 of ASHRAE 135, and nothing above it. Token passing, timers and
line control belong to the layer that uses it, such as `bacnet-mstp-core`.

- `#![no_std]`, no allocator and no dependencies in the default build.
- Standard frames and the two extended data frames (types 32 and 33), whose
  data goes on the wire COBS-encoded with a CRC-32K.
- The header CRC-8, the data CRC-16 and the CRC-32K, in the `crc` module.
- One-shot `Frame::encode_into` and `decode_frame` over caller storage, and
  `decode_frame_in_place` for an extended frame, whose data is decoded over
  its encoded form.
- `StreamDecoder`, an incremental decoder for a serial byte stream, which
  keeps its receive state in a buffer you lend it, skips a frame too long
  for it by count, and reports every candidate it drops with the
  destination a receiver needs to judge it.
- `FrameType` is an open set: a constant for each type the standard
  defines, and `from_raw` for any other octet, so a reserved or
  proprietary type passes through unchanged.
- Decoding is a raw link-level check; encoding is strict. The crate docs
  list the checks in the order each direction runs them.

## Features

None is on by default, and none needs `std`:

- `alloc`: a `FrameSink` for `Vec<u8>`.
- `bytes`: a `FrameSink` for `bytes::BytesMut`; implies `alloc`.
- `heapless`: a `FrameSink` for `heapless::Vec<u8, N>`.
- `crc32k-table`: works out the CRC-32K with a 256-entry table, about 1 KiB
  more flash than the default 16-entry one and half the lookups.

## Example

```rust
use bacnet_mstp_codec::{
    decode_frame, DecodeEvent, Frame, FrameType, StreamDecoder, MAX_STANDARD_FRAME_LENGTH,
};

// Encode a frame into caller storage, and decode it in one shot.
let frame = Frame::new(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0x10, 0x05, &[0x01, 0x00]);
let mut wire = [0u8; MAX_STANDARD_FRAME_LENGTH];
let length = frame.encode_into(&mut wire).unwrap();
assert_eq!(decode_frame(&wire[..length]), Ok((frame, length)));

// Or feed the octets to a stream decoder as a UART delivers them.
let mut storage = [0u8; MAX_STANDARD_FRAME_LENGTH];
let mut decoder = StreamDecoder::new(&mut storage);
let mut received = None;
for &octet in &wire[..length] {
    if let DecodeEvent::Frame { frame, .. } = decoder.push(octet) {
        received = Some((frame.header, frame.data.to_vec()));
    }
}
assert_eq!(received, Some((frame.header, vec![0x01, 0x00])));

// Match frame types on their constants, with a `_` arm for the rest.
let kind = match FrameType::from_raw(0x80) {
    FrameType::TOKEN => "token",
    other if other.is_proprietary() => "proprietary",
    _ => "other",
};
assert_eq!(kind, "proprietary");
```

## License

MIT
