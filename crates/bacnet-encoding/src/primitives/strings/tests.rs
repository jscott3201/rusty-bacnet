//! The string sink encoders: independent wire cases, whole-value refusal, and
//! (with alloc) the same octets as the `BytesMut` encoders.

use super::*;

/// Every encoder under test, by name, appending `payload` to a sink.
type Encode = fn(&mut &mut [u8], &[u8]) -> Result<usize, SinkFull>;

fn text(payload: &[u8]) -> &str {
    core::str::from_utf8(payload).expect("the cases use ASCII")
}

const ENCODERS: [(&str, Encode); 8] = [
    ("app CharacterString", |s, p| {
        encode_app_character_string_to_sink(s, text(p))
    }),
    ("ctx 3 CharacterString", |s, p| {
        encode_ctx_character_string_to_sink(s, 3, text(p))
    }),
    ("CharacterString contents", |s, p| {
        encode_character_string_to_sink(s, text(p))
    }),
    ("app OCTET STRING", |s, p| {
        encode_app_octet_string_to_sink(s, p)
    }),
    ("ctx 20 OCTET STRING", |s, p| {
        encode_ctx_octet_string_to_sink(s, 20, p)
    }),
    ("app BIT STRING", |s, p| {
        encode_app_bit_string_to_sink(s, 3, p)
    }),
    ("ctx 1 BIT STRING", |s, p| {
        encode_ctx_bit_string_to_sink(s, 1, 3, p)
    }),
    ("BIT STRING contents", |s, p| {
        encode_bit_string_to_sink(s, 3, p)
    }),
];

// Tabulated by hand from Clauses 20.2.1, 20.2.8, 20.2.9 and 20.2.10: the tag
// octet, any extended length, the leading octet, then the payload "ab".
const GOLDENS: [&[u8]; 8] = [
    &[0x73, 0x00, b'a', b'b'],
    &[0x3B, 0x00, b'a', b'b'],
    &[0x00, b'a', b'b'],
    &[0x62, b'a', b'b'],
    &[0xFA, 0x14, b'a', b'b'],
    &[0x83, 0x03, b'a', b'b'],
    &[0x1B, 0x03, b'a', b'b'],
    &[0x03, b'a', b'b'],
];

#[test]
fn each_encoder_writes_its_tabulated_octets() {
    for ((name, encode), golden) in ENCODERS.into_iter().zip(GOLDENS) {
        let mut storage = [0xA5; 8];
        let mut tail = &mut storage[..];
        assert_eq!(encode(&mut tail, b"ab"), Ok(golden.len()), "{name}");
        assert_eq!(tail.len(), 8 - golden.len(), "{name}");
        assert_eq!(&storage[..golden.len()], golden, "{name}");
    }
}

#[test]
fn a_longer_value_takes_the_extended_length_form() {
    // 253 contents octets: length 5 with one extended octet, X'FD'.
    let payload = [b'x'; 252];
    let mut storage = [0; 300];
    let mut tail = &mut storage[..];
    let written = encode_app_character_string_to_sink(&mut tail, text(&payload)).unwrap();
    assert_eq!(written, 2 + 253);
    assert_eq!(storage[..3], [0x75, 0xFD, 0x00]);
    assert_eq!(storage[3..255], payload);
}

#[test]
fn a_value_that_does_not_fit_leaves_the_sink_unchanged() {
    for ((name, encode), golden) in ENCODERS.into_iter().zip(GOLDENS) {
        for room in 0..golden.len() {
            let mut storage = [0xA5; 8];
            let mut tail = &mut storage[..room];
            assert_eq!(
                encode(&mut tail, b"ab"),
                Err(SinkFull {
                    needed: golden.len(),
                    available: room
                }),
                "{name}, {room} octets of room"
            );
            assert_eq!(tail.len(), room, "{name}");
            assert_eq!(storage, [0xA5; 8], "{name}");
        }
    }
}

/// The sink encoders write the octets the `BytesMut` encoders write, at each
/// length where the header's length form changes.
#[cfg(feature = "alloc")]
#[test]
fn the_sink_encoders_match_the_bytes_mut_encoders() {
    use bytes::BytesMut;

    use crate::primitives::{
        encode_app_bit_string, encode_app_character_string, encode_app_octet_string,
        encode_bit_string, encode_character_string, encode_ctx_bit_string,
        encode_ctx_character_string, encode_ctx_octet_string,
    };

    type Hosted = fn(&mut BytesMut, &[u8]);
    let hosted: [Hosted; 8] = [
        |b, p| encode_app_character_string(b, text(p)).unwrap(),
        |b, p| encode_ctx_character_string(b, 3, text(p)).unwrap(),
        |b, p| encode_character_string(b, text(p)),
        encode_app_octet_string,
        |b, p| encode_ctx_octet_string(b, 20, p),
        |b, p| encode_app_bit_string(b, 3, p),
        |b, p| encode_ctx_bit_string(b, 1, 3, p),
        |b, p| encode_bit_string(b, 3, p),
    ];
    let payload = std::vec![b'z'; 70_000];
    for length in [
        0, 1, 3, 4, 5, 252, 253, 254, 255, 65_534, 65_535, 65_536, 70_000,
    ] {
        let payload = &payload[..length];
        for ((name, encode), hosted) in ENCODERS.into_iter().zip(hosted) {
            let mut expected = BytesMut::new();
            hosted(&mut expected, payload);
            let mut storage = std::vec![0; expected.len()];
            let mut tail = &mut storage[..];
            assert_eq!(encode(&mut tail, payload), Ok(expected.len()), "{name}");
            assert!(tail.is_empty(), "{name}");
            assert_eq!(storage, expected[..], "{name}, {length} octets");
        }
    }
}

/// A value too long for a header's 32-bit length is refused, with its
/// encoded length and the sink's room, and the longest one that fits is
/// accepted. The check takes the length alone, so no 4 GiB value is built.
/// `write_tagged` runs it before any write, so a refusal leaves the sink as is.
#[test]
fn a_length_past_32_bits_is_refused() {
    let mut storage = [0xA5; 8];
    let tail = &mut storage[..];
    assert_eq!(header_length(&tail, u32::MAX as usize), Ok(u32::MAX));
    // Only a 64-bit host has lengths past 32 bits.
    if let Some(over) = (u32::MAX as usize).checked_add(1) {
        assert_eq!(
            header_length(&tail, over),
            Err(SinkFull {
                needed: over + MAX_HEADER,
                available: 8
            })
        );
        assert_eq!(
            header_length(&tail, usize::MAX),
            Err(SinkFull {
                needed: usize::MAX,
                available: 8
            })
        );
        // A growable sink advertises no room.
        #[cfg(feature = "alloc")]
        assert_eq!(
            header_length(&std::vec::Vec::<u8>::new(), over),
            Err(SinkFull {
                needed: over + MAX_HEADER,
                available: u32::MAX as usize
            })
        );
    }
    // The longest header, an extended tag number with a four-octet length,
    // fills the encoders' staging buffer exactly.
    let mut header = [0; MAX_HEADER];
    let mut header_tail = &mut header[..];
    assert_eq!(
        encode_tag_to_sink(&mut header_tail, 254, TagClass::Context, u32::MAX),
        Ok(MAX_HEADER)
    );
}
