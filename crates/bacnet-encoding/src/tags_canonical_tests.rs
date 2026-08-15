use crate::tags::{self, TagClass};

#[test]
fn rejects_invalid_extended_tag_numbers() {
    for initial in [0xF1, 0xF9, 0xFE, 0xFF] {
        for tag_number in [0, 1, 14, 255] {
            assert!(tags::decode_tag(&[initial, tag_number], 0).is_err());
        }
    }
}

#[test]
fn rejects_noncanonical_extended_lengths() {
    for length in 0..=4 {
        assert!(tags::decode_tag(&[0x0D, length], 0).is_err());
    }

    assert!(tags::decode_tag(&[0x0D, 254, 0x00, 0xFD], 0).is_err());
    assert!(tags::decode_tag(&[0x0D, 255, 0x00, 0x00, 0xFF, 0xFF], 0).is_err());
}

#[test]
fn accepts_canonical_extended_length_boundaries() {
    let cases: &[(&[u8], u32)] = &[
        (&[0x0D, 5], 5),
        (&[0x0D, 253], 253),
        (&[0x0D, 254, 0x00, 0xFE], 254),
        (&[0x0D, 254, 0xFF, 0xFF], 65_535),
        (&[0x0D, 255, 0x00, 0x01, 0x00, 0x00], 65_536),
    ];

    for &(encoded, expected_length) in cases {
        let (tag, pos) = tags::decode_tag(encoded, 0).unwrap();
        assert_eq!(tag.number, 0);
        assert_eq!(tag.class, TagClass::Context);
        assert_eq!(tag.length, expected_length);
        assert_eq!(pos, encoded.len());
    }
}

#[test]
fn rejects_reserved_application_lvt_forms() {
    for lvt in [6, 7] {
        let encoded = [0x20 | lvt, 5, 0, 0, 0, 0, 1];
        assert!(tags::decode_tag(&encoded, 0).is_err());
        assert!(crate::primitives::decode_application_value(&encoded, 0).is_err());
    }
}
