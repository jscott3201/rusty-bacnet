use super::*;
use bacnet_types::primitives::{Date, Time};

// Independently authored context-0 names and application values.
const SEMANTIC: &[u8] = &[0x0a, 0, b'a'];
const NULL: &[u8] = &[0x0a, 0, b'a', 0];
const DATE: &[u8] = &[0x0a, 0, b'd', 0xa4, 126, 10, 8, 4];
const TIME: &[u8] = &[0x0a, 0, b't', 0xb4, 12, 34, 56, 7];

#[test]
fn tags_read_whole_element_size_and_primitive_identity() {
    let vectors = [
        (SEMANTIC, BACnetNameValue::semantic("a")),
        (NULL, BACnetNameValue::valued("a", PropertyValue::Null)),
        (&[0x09, 0][..], BACnetNameValue::semantic("")),
        (
            &[0x0a, 0, b'n', 0x21, 3][..],
            BACnetNameValue::valued("n", PropertyValue::Unsigned(3)),
        ),
        (
            &[0x0a, 0, b's', 0x72, 0, b'x'][..],
            BACnetNameValue::valued("s", PropertyValue::CharacterString("x".into())),
        ),
        (
            &[0x0a, 0, b'r', 0x44, 0x3f, 0xc0, 0, 0][..],
            BACnetNameValue::valued("r", PropertyValue::Real(1.5)),
        ),
        (
            DATE,
            BACnetNameValue::valued(
                "d",
                PropertyValue::Date(Date {
                    year: 126,
                    month: 10,
                    day: 8,
                    day_of_week: 4,
                }),
            ),
        ),
        (
            TIME,
            BACnetNameValue::valued(
                "t",
                PropertyValue::Time(Time {
                    hour: 12,
                    minute: 34,
                    second: 56,
                    hundredths: 7,
                }),
            ),
        ),
    ];
    let whole = vectors
        .iter()
        .flat_map(|(bytes, _)| bytes.iter().copied())
        .collect::<Vec<_>>();
    let expected = vectors
        .iter()
        .map(|(_, tag)| tag.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        decode_tags_read(None, &whole).unwrap(),
        TagsRead::Whole(expected)
    );
    for (bytes, tag) in vectors {
        assert_eq!(
            decode_tags_read(Some(1), bytes).unwrap(),
            TagsRead::Element(tag.clone())
        );
        assert_eq!(
            decode_tags_read(Some(u32::MAX), bytes).unwrap(),
            TagsRead::Element(tag)
        );
    }
    assert_eq!(
        decode_tags_read(None, &[]).unwrap(),
        TagsRead::Whole(vec![])
    );
    for (bytes, size) in [
        (&[0x21, 0][..], 0),
        (&[0x21, 8][..], 8),
        (&[0x24, 255, 255, 255, 255][..], u32::MAX),
    ] {
        assert_eq!(
            decode_tags_read(Some(0), bytes).unwrap(),
            TagsRead::Size(size)
        );
    }
}

#[test]
fn tags_read_rejects_bad_elements_and_never_returns_a_prefix() {
    let bad: &[&[u8]] = &[
        &[0x0d],
        &[0x0b, 0, b'a'],
        &[0x0a, 0, 255],
        &[0x0a, 3, b'a'],
        &[0x0a, 0, b'a', 0xd0],
        &[0x0a, 0, b'a', 0x44, 0],
        &[0x0a, 0, b'a', 0xa4, 126, 10, 8, 4, 0xb4, 12, 34, 56, 7],
        &[0x0a, 0, b'a', 0xa4, 126, 10, 8, 4, 0xb4, 12],
        &[0x1a, 0, b'a'],
    ];
    for bytes in bad {
        assert!(decode_tags_read(Some(1), bytes).is_err(), "{bytes:?}");
        assert!(decode_tags_read(None, bytes).is_err(), "{bytes:?}");
        assert!(decode_tags_read(None, &[SEMANTIC, bytes].concat()).is_err());
    }
    assert!(decode_tags_read(Some(1), &[]).is_err());
    assert!(decode_tags_read(Some(1), &[SEMANTIC, SEMANTIC].concat()).is_err());
    assert!(decode_tags_read(Some(1), &[SEMANTIC, &[0x1a, 0, b'x']].concat()).is_err());
    assert!(decode_tags_read(None, &[SEMANTIC, &[0x1a, 0, b'x']].concat()).is_err());
}

#[test]
fn tags_read_count_is_one_u32_unsigned_with_no_tail() {
    for bytes in [
        &[][..],
        &[0],
        &[0x91, 3],
        &[0x21],
        &[0x21, 3, 0x21, 4],
        &[0x25, 5, 1, 0, 0, 0, 0],
        SEMANTIC,
    ] {
        assert!(decode_tags_read(Some(0), bytes).is_err(), "{bytes:?}");
    }
}
