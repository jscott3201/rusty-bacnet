//! `BACnetShedLevel` CHOICE vectors (Clause 21).
use super::*;
use bacnet_types::constructed::BACnetShedLevel;

fn encode(value: &BACnetShedLevel) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_shed_level(&mut buf, value);
    buf.to_vec()
}

/// Bytes written out by hand from the CHOICE's context tags, not produced by
/// the codec: percent `[0]` and level `[1]` hold an Unsigned in the fewest
/// octets, amount `[2]` a four-octet REAL.
fn golden_vectors() -> Vec<(BACnetShedLevel, Vec<u8>)> {
    vec![
        (BACnetShedLevel::Percent(0), vec![0x09, 0x00]),
        (BACnetShedLevel::Percent(100), vec![0x09, 0x64]),
        (BACnetShedLevel::Level(0), vec![0x19, 0x00]),
        (BACnetShedLevel::Level(300), vec![0x1A, 0x01, 0x2C]),
        // Eight octets take the extended length octet.
        (
            BACnetShedLevel::Level(u64::MAX),
            vec![0x1D, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        ),
        (
            BACnetShedLevel::Amount(0.0),
            vec![0x2C, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            BACnetShedLevel::Amount(12.5),
            vec![0x2C, 0x41, 0x48, 0x00, 0x00],
        ),
    ]
}

#[test]
fn shed_level_golden_vectors_round_trip() {
    for (value, bytes) in golden_vectors() {
        assert_eq!(encode(&value), bytes, "{value:?}");
        assert_eq!(
            decode_shed_level(&bytes, 0).unwrap(),
            (value, bytes.len()),
            "{bytes:02X?}"
        );
    }
}

#[test]
fn shed_level_decodes_one_choice_and_leaves_what_follows() {
    // A level, then an amount, back to back at an offset.
    let data = [0xAA, 0x19, 0x02, 0x2C, 0x41, 0x48, 0x00, 0x00];
    let (first, next) = decode_shed_level(&data, 1).unwrap();
    assert_eq!((first, next), (BACnetShedLevel::Level(2), 3));
    assert_eq!(
        decode_shed_level(&data, next).unwrap(),
        (BACnetShedLevel::Amount(12.5), data.len())
    );
    // Leading zero octets are a valid Unsigned.
    assert_eq!(
        decode_shed_level(&[0x0A, 0x00, 0x32], 0).unwrap(),
        (BACnetShedLevel::Percent(50), 3)
    );
}

#[test]
fn shed_level_rejects_other_tags_and_malformed_contents() {
    let malformed: &[(&str, &[u8])] = &[
        ("application Unsigned", &[0x21, 0x32]),
        ("application REAL", &[0x44, 0x41, 0x48, 0x00, 0x00]),
        ("context tag 3", &[0x39, 0x01]),
        ("percent framed", &[0x0E, 0x21, 0x32, 0x0F]),
        ("closing tag", &[0x1F]),
        ("empty percent", &[0x08]),
        ("empty level", &[0x18]),
        (
            "nine-octet level",
            &[0x1D, 0x09, 0x01, 0, 0, 0, 0, 0, 0, 0, 0],
        ),
        ("three-octet amount", &[0x2B, 0x41, 0x48, 0x00]),
        ("eight-octet amount", &[0x2D, 0x08, 0, 0, 0, 0, 0, 0, 0, 0]),
    ];
    for (name, bytes) in malformed {
        assert!(
            matches!(decode_shed_level(bytes, 0), Err(Error::Decoding(_))),
            "{name}: {:?}",
            decode_shed_level(bytes, 0)
        );
    }
    for truncated in [&[][..], &[0x1A, 0x01], &[0x2C, 0x41, 0x48]] {
        assert!(decode_shed_level(truncated, 0).is_err(), "{truncated:02X?}");
    }
    assert!(matches!(
        decode_shed_level(&[0x1A, 0x01], 0),
        Err(Error::BufferTooShort { .. })
    ));
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    for (_, wire) in golden_vectors() {
        super::assert_members_cut_short("BACnetShedLevel", &wire, |data| {
            decode_shed_level(data, 0)
        });
    }
}
