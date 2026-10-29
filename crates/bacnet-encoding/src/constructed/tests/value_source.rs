//! Independent Clause 21 ValueSource CHOICE vectors.
use crate::constructed::{decode_value_source, encode_value_source};
use bacnet_types::constructed::BACnetValueSource;
use bytes::BytesMut;

#[test]
fn value_source_none_golden_vector() {
    let source = BACnetValueSource::None;
    let mut bytes = BytesMut::new();
    encode_value_source(&mut bytes, &source).unwrap();
    assert_eq!(&bytes[..], &[0x08]);
    assert_eq!(decode_value_source(&[0x08], 0).unwrap(), (source, 1));
}

use bacnet_types::{
    MacAddr,
    constructed::{BACnetAddress, BACnetDeviceObjectReference},
    enums::ObjectType,
    error::Error,
    primitives::ObjectIdentifier,
};

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

/// Fixed bytes derived directly from Clause21 CHOICE tags and Clause20.2
/// OID fields (10-bit type / 22-bit instance), not produced by this codec.
fn vectors() -> Vec<(BACnetValueSource, &'static [u8])> {
    vec![
        (BACnetValueSource::None, &[0x08]),
        (
            BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: oid(ObjectType::ANALOG_VALUE, 7),
            }),
            &[0x1e, 0x1c, 0x00, 0x80, 0x00, 0x07, 0x1f],
        ),
        (
            BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: Some(oid(ObjectType::DEVICE, 1)),
                object_identifier: oid(ObjectType::ANALOG_VALUE, 7),
            }),
            &[
                0x1e, 0x0c, 0x02, 0x00, 0x00, 0x01, 0x1c, 0x00, 0x80, 0x00, 0x07, 0x1f,
            ],
        ),
        (
            BACnetValueSource::Address(BACnetAddress {
                network_number: 7,
                mac_address: MacAddr::from_slice(&[0x11, 0x22]),
            }),
            &[0x2e, 0x21, 0x07, 0x62, 0x11, 0x22, 0x2f],
        ),
        (
            BACnetValueSource::Address(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::new(),
            }),
            &[0x2e, 0x21, 0x00, 0x60, 0x2f],
        ),
        (
            BACnetValueSource::Address(BACnetAddress {
                network_number: u16::MAX,
                mac_address: MacAddr::from_slice(&[192, 168, 1, 10, 0xba, 0xc0]),
            }),
            &[
                0x2e, 0x22, 0xff, 0xff, 0x65, 0x06, 192, 168, 1, 10, 0xba, 0xc0, 0x2f,
            ],
        ),
        // The datatype does not narrow either OID to Device or concrete instances.
        (
            BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: Some(oid(
                    ObjectType::ANALOG_INPUT,
                    ObjectIdentifier::MAX_INSTANCE,
                )),
                object_identifier: oid(ObjectType::from_raw(1023), ObjectIdentifier::MAX_INSTANCE),
            }),
            &[
                0x1e, 0x0c, 0x00, 0x3f, 0xff, 0xff, 0x1c, 0xff, 0xff, 0xff, 0xff, 0x1f,
            ],
        ),
    ]
}

#[test]
fn value_source_all_choices_match_independent_wire_vectors() {
    for (source, golden) in vectors() {
        let mut encoded = BytesMut::new();
        encode_value_source(&mut encoded, &source).unwrap();
        assert_eq!(&encoded[..], golden, "{source:?}");
        assert_eq!(
            decode_value_source(golden, 0).unwrap(),
            (source, golden.len())
        );
    }
}

#[test]
fn value_source_stream_offsets_and_encoder_append_preserve_surrounding_bytes() {
    for (source, golden) in vectors() {
        let mut encoded = BytesMut::from(&[0xaa, 0xbb][..]);
        encode_value_source(&mut encoded, &source).unwrap();
        encoded.extend_from_slice(&[0x21, 0x05]); // unrelated application Unsigned(5)
        assert_eq!(&encoded[..2], &[0xaa, 0xbb]);
        assert_eq!(&encoded[2..2 + golden.len()], golden);
        let (decoded, end) = decode_value_source(&encoded, 2).unwrap();
        assert_eq!(decoded, source);
        assert_eq!(end, 2 + golden.len());
        assert_eq!(&encoded[end..], &[0x21, 0x05]);
    }
    // Array callers may decode adjacent choices; no whole-buffer restriction.
    let mut encoded = BytesMut::new();
    for (source, _) in vectors() {
        encode_value_source(&mut encoded, &source).unwrap();
    }
    let mut offset = 0;
    for (source, _) in vectors() {
        let (decoded, end) = decode_value_source(&encoded, offset).unwrap();
        assert_eq!(decoded, source);
        assert!(end > offset);
        offset = end;
    }
    assert_eq!(offset, encoded.len());
}

#[test]
fn value_source_rejects_every_truncated_vector_and_invalid_start_without_panic() {
    for (_, golden) in vectors() {
        for end in 0..golden.len() {
            assert!(
                decode_value_source(&golden[..end], 0).is_err(),
                "{golden:x?}, end={end}"
            );
        }
        for offset in [golden.len(), golden.len() + 1, usize::MAX] {
            assert!(decode_value_source(golden, offset).is_err());
        }
    }
}

#[test]
fn value_source_rejects_malformed_choices_members_and_framing() {
    let malformed: &[&[u8]] = &[
        &[0x00],       // application NULL is not none[0]
        &[0x09, 0x00], // context NULL with contents
        &[0x0e, 0x0f],
        &[0x0f], // none cannot be constructed/closing
        &[0x18],
        &[0x28],
        &[0x38], // wrong primitive or unknown alternative
        &[0x3e, 0x3f],
        &[0xf8],                                           // unknown / truncated extended tag
        &[0x1e, 0x1f],                                     // missing required object
        &[0x1e, 0x0c, 2, 0, 0, 1, 0x1f],                   // Device alone is insufficient
        &[0x1e, 0x1b, 0, 0, 7, 0x1f],                      // wrong object-id width
        &[0x1e, 0x0b, 0, 0, 1, 0x1c, 0, 0x80, 0, 7, 0x1f], // bad optional width
        &[0x1e, 0xc4, 0, 0x80, 0, 7, 0x1f],                // required OID is context[1]
        &[0x1e, 0x1e, 0x1f, 0x1f],                         // required field is primitive
        &[0x1e, 0x1c, 0, 0x80, 0, 7, 0x2f],                // mismatched close
        &[0x1e, 0x1c, 0, 0x80, 0, 7, 0x08, 0x1f],          // extra member inside object
        &[0x1e, 0x1c, 0, 0x80, 0, 7, 0x0c, 2, 0, 0, 1, 0x1f], // reordered fields
        &[0x2e, 0x2f],                                     // missing both address members
        &[0x2e, 0x21, 7, 0x2f],                            // missing MAC
        &[0x2e, 0x31, 7, 0x60, 0x2f],                      // network must be application Unsigned
        &[0x2e, 0x29, 7, 0x60, 0x2f],                      // not context Unsigned
        &[0x2e, 0x20, 0x60, 0x2f],                         // zero-octet Unsigned
        &[0x2e, 0x23, 1, 0, 0, 0x60, 0x2f],                // network 65536
        &[0x2e, 0x25, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x60, 0x2f], // Unsigned >8 bytes
        &[0x2e, 0x21, 7, 0x70, 0x2f],                      // MAC must be OCTET STRING
        &[0x2e, 0x21, 7, 0x68, 0x2f],                      // not context MAC
        &[0x2e, 0x21, 7, 0x60, 0x1f],                      // wrong closing tag
        &[0x2e, 0x21, 7, 0x60, 0x08, 0x2f],                // extra inner field
        &[0x2e, 0x21, 7, 0x65, 0xff, 0xff, 0xff, 0xff, 0xff], // unavailable large MAC
    ];
    for data in malformed {
        let error = decode_value_source(data, 0).expect_err(&format!("{data:x?}"));
        assert!(
            error.decode_fault().is_some() || matches!(error, Error::BufferTooShort { .. }),
            "{error:?}"
        );
    }
    let error = decode_value_source(&[0xff, 0xff, 0x00], 2).unwrap_err();
    assert_eq!(error.decode_fault().map(|(offset, _)| offset), Some(2));
}

#[test]
fn value_source_address_mac_holds_to_the_bacnet_address_bound() {
    // #1156: the address [2] alternative is a BACnetAddress, whose MAC is at
    // most BACnetAddress::MAX_MAC_LEN (18) octets in both directions.
    let wire = |len: usize| {
        let mut wire = vec![0x2e, 0x21, 0x07, 0x65, len as u8];
        wire.extend(std::iter::repeat_n(0xA5, len));
        wire.push(0x2f);
        wire
    };
    let address = |len: usize| {
        BACnetValueSource::Address(BACnetAddress {
            network_number: 7,
            mac_address: MacAddr::from_slice(&vec![0xA5; len]),
        })
    };
    let longest = BACnetAddress::MAX_MAC_LEN;
    let mut encoded = BytesMut::new();
    encode_value_source(&mut encoded, &address(longest)).unwrap();
    assert_eq!(&encoded[..], &wire(longest)[..]);
    assert_eq!(
        decode_value_source(&wire(longest), 0).unwrap(),
        (address(longest), wire(longest).len())
    );
    for len in [longest + 1, 255] {
        assert!(
            decode_value_source(&wire(len), 0)
                .unwrap_err()
                .decode_fault()
                .is_some_and(|(offset, _)| offset == 3),
            "{len}-octet MAC"
        );
        let mut encoded = BytesMut::from(&[0xaa][..]);
        assert!(matches!(
            encode_value_source(&mut encoded, &address(len)),
            Err(Error::Encode(_))
        ));
        assert_eq!(&encoded[..], &[0xaa], "{len}-octet MAC left output behind");
    }
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    for (_, wire) in vectors() {
        super::assert_members_cut_short("BACnetValueSource", wire, |data| {
            decode_value_source(data, 0)
        });
    }
}
