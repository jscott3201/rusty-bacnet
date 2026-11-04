//! Event_Parameters values in the pre-framing `[255]` form (#2376): the
//! check every property value encoder runs, the decoder's first match, and
//! the fuzz input that found the gap.

use super::*;
use bacnet_types::constructed::{BACnetPropertyValue, NotificationParameters};
use bacnet_types::enums::PropertyIdentifier;

const EVENT_PARAMETERS: PropertyIdentifier = PropertyIdentifier::EVENT_PARAMETERS;

fn value_of(property_identifier: PropertyIdentifier, value: &[u8]) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier,
        property_array_index: None,
        value: value.to_vec(),
        priority: None,
    }
}

fn legacy(value: &[u8]) -> BACnetPropertyValue {
    value_of(EVENT_PARAMETERS, value)
}

fn encoded(values: &[BACnetPropertyValue]) -> BytesMut {
    let mut buf = BytesMut::new();
    for value in values {
        encode_bacnet_property_value(value, &mut buf).unwrap();
    }
    buf
}

/// What the encoding crate's fuzz targets check for a codec (their
/// `round_trip`): when `decode` accepts `input` and the encoder takes the
/// value, its encoding decodes whole to the same value. `None` when the
/// encoder refuses the value, which ends the check.
fn fuzz_round_trip(input: &[u8]) -> Option<Vec<u8>> {
    let (value, end) = decode_bacnet_property_value(input, 0).ok()?;
    assert!(end <= input.len());
    let mut first = BytesMut::new();
    encode_bacnet_property_value(&value, &mut first).ok()?;
    let (again, end) = decode_bacnet_property_value(&first, 0)
        .unwrap_or_else(|error| panic!("{first:02x?} doesn't decode: {error:?}"));
    assert_eq!(end, first.len());
    assert_eq!(again, value);
    Some(first.to_vec())
}

#[test]
fn the_minimized_fuzz_input_no_longer_breaks_the_round_trip() {
    // The scheduled Fuzz run's `constructed_decode` find, without its
    // selector octet (0x3e, BACnetPropertyValue).
    let input = [
        0x09, 0x53, 0x19, 0x42, 0x2e, 0xfe, 0xff, 0xff, 0xff, 0x2f, 0x3c, 0x4e, 0xff, 0xff, 0x2f,
        0x0d, 0x0c, 0x00, 0x2f,
    ];
    // The first `ff ff 2f` is followed by a four-octet [3], neither a
    // one-octet priority nor one that runs to the end, so the value ends at
    // the second, before the next value's [0].
    let (value, end) = decode_bacnet_property_value(&input, 0).unwrap();
    assert_eq!(
        value,
        BACnetPropertyValue {
            property_array_index: Some(0x42),
            ..legacy(&[0xfe, 0xff, 0xff, 0xff, 0x2f, 0x3c, 0x4e, 0xff, 0xff])
        }
    );
    assert_eq!(end, 15);

    // Alone, the first pair would end it: the encoder refuses the value, which
    // ends the fuzzer's round trip.
    assert_eq!(fuzz_round_trip(&input), None);
    let mut buf = BytesMut::new();
    let refused = encode_bacnet_property_value(&value, &mut buf).unwrap_err();
    assert!(buf.is_empty());
    assert_eq!(
        refused.to_string(),
        "encoding error: BACnetPropertyValue: legacy EventParameters value holds its outer \
         closing tag before its end"
    );
}

#[test]
fn the_check_takes_any_other_value() {
    for (property, value) in [
        (
            PropertyIdentifier::PRESENT_VALUE,
            &[0xfe, 0xff, 1, 0xff, 0xff, 0x2f][..],
        ),
        (PropertyIdentifier::PRESENT_VALUE, &[0xfe, 0xff][..]),
        (EVENT_PARAMETERS, &[][..]),
        (EVENT_PARAMETERS, &[0x62, 0xff, 0xff][..]),
        (EVENT_PARAMETERS, &[0xff, 0xfe, 0xff, 0x2f][..]),
    ] {
        for tag in [2, 3, 4] {
            assert_eq!(
                check_property_value(property, value, tag),
                Ok(()),
                "{value:02x?}"
            );
        }
    }
}

#[test]
fn the_check_takes_a_legacy_value_that_decodes_back() {
    for value in [
        &[0xfe, 0xff, 0xff, 0xff][..],
        &[0xfe, 0xff, 1, 2, 0xff, 0xff],
        &[0xfe, 0xff, 1, 0xff, 0xff, 0x3f, 2, 0xff, 0xff],
        // A pair ending at the third octet is before the decoder looks.
        &[0xfe, 0xff, 0xff, 0x2f, 0xff, 0xff],
    ] {
        assert_eq!(check_property_value(EVENT_PARAMETERS, value, 2), Ok(()));
        let buf = encoded(&[legacy(value)]);
        let (decoded, end) = decode_bacnet_property_value(&buf, 0).unwrap();
        assert_eq!(decoded.value, value);
        assert_eq!(end, buf.len());
    }
}

#[test]
fn the_check_refuses_a_legacy_value_without_its_closing_tags() {
    for value in [
        &[0xfe, 0xff][..],
        &[0xfe, 0xff, 0xff],
        &[0xfe, 0xff, 1, 2],
        &[0xfe, 0xff, 1, 0xff],
        &[0xfe, 0xff, 0xff, 0xff, 1],
    ] {
        for tag in [2, 3, 4] {
            let refused = check_property_value(EVENT_PARAMETERS, value, tag).unwrap_err();
            assert_eq!(
                refused.label, "legacy EventParameters value is missing its closing tags",
                "{value:02x?}"
            );
        }
    }
}

#[test]
fn the_check_refuses_the_outer_closing_tag_after_a_pair_before_the_end() {
    for tag in [2u8, 3, 4] {
        let close = (tag << 4) | 0x0f;
        for value in [
            [0xfe, 0xff, 0xff, 0xff, close, 0xff, 0xff].as_slice(),
            &[0xfe, 0xff, 1, 0xff, 0xff, close, 2, 0xff, 0xff],
            &[0xfe, 0xff, 1, 0xff, 0xff, close, 0xff, 0xff],
        ] {
            let refused = check_property_value(EVENT_PARAMETERS, value, tag).unwrap_err();
            assert_eq!(
                refused.label,
                "legacy EventParameters value holds its outer closing tag before its end"
            );
            // Another production's closing tag is payload.
            let other = if tag == 2 { 3 } else { 2 };
            assert_eq!(check_property_value(EVENT_PARAMETERS, value, other), Ok(()));
        }
    }
}

#[test]
fn concatenated_legacy_values_decode_one_by_one() {
    let first = legacy(&[0xfe, 0xff, 1, 0xff, 0xff]);
    let octet_string = value_of(PropertyIdentifier::DESCRIPTION, &[0x62, 0xff, 0xff]);
    let mut with_priority = legacy(&[0xfe, 0xff, 2, 0xff, 0xff]);
    with_priority.priority = Some(8);
    for values in [
        vec![first.clone(), first.clone()],
        vec![first.clone(), octet_string.clone()],
        vec![with_priority, first.clone(), first.clone()],
    ] {
        let buf = encoded(&values);
        let mut offset = 0;
        for value in &values {
            let (decoded, next) = decode_bacnet_property_value(&buf, offset)
                .unwrap_or_else(|error| panic!("{buf:02x?} at {offset}: {error:?}"));
            assert_eq!(&decoded, value);
            offset = next;
        }
        assert_eq!(offset, buf.len());
    }
}

#[test]
fn listed_legacy_values_decode_one_by_one() {
    let mut with_priority = legacy(&[0xfe, 0xff, 2, 0xff, 0xff]);
    with_priority.priority = Some(16);
    let values = [
        legacy(&[0xfe, 0xff, 1, 0xff, 0xff]),
        with_priority,
        legacy(&[0xfe, 0xff, 3, 0xff, 0xff]),
        value_of(PropertyIdentifier::DESCRIPTION, &[0x62, 0xff, 0xff]),
    ];
    for closing_tag in [1, 4, 6] {
        let mut buf = encoded(&values);
        tags::encode_closing_tag(&mut buf, closing_tag);
        let mut offset = 0;
        for value in &values {
            let (decoded, next) =
                decode_bacnet_property_value_in_list(&buf, offset, closing_tag).unwrap();
            assert_eq!(&decoded, value);
            offset = next;
        }
        assert_eq!(offset, buf.len() - 1);
    }
}

#[test]
fn an_older_peers_value_with_two_possible_ends_is_read_to_the_first() {
    // The value holds `ff ff 2f` and then what looks like a next value, so
    // either pair could end it; the first does.
    let wire = [
        0x09, 0x53, 0x2e, 0xfe, 0xff, 0xaa, 0xff, 0xff, 0x2f, 0x09, 0x53, 0x2e, 0xfe, 0xff, 0xbb,
        0xff, 0xff, 0x2f,
    ];
    let (first, next) = decode_bacnet_property_value(&wire, 0).unwrap();
    assert_eq!(first, legacy(&[0xfe, 0xff, 0xaa, 0xff, 0xff]));
    let (second, end) = decode_bacnet_property_value(&wire, next).unwrap();
    assert_eq!(second, legacy(&[0xfe, 0xff, 0xbb, 0xff, 0xff]));
    assert_eq!(end, wire.len());
}

#[test]
fn a_one_octet_priority_then_the_next_value_ends_a_legacy_value() {
    // What the encoder writes for a legacy value with priority 5 followed
    // by another value: two values, the first with its priority.
    let wire = [
        0x09, 0x53, 0x2e, 0xfe, 0xff, 0x01, 0xff, 0xff, 0x2f, 0x39, 0x05, 0x09, 0x53, 0x2e, 0xfe,
        0xff, 0x02, 0xff, 0xff, 0x2f,
    ];
    let mut first = legacy(&[0xfe, 0xff, 1, 0xff, 0xff]);
    first.priority = Some(5);
    assert_eq!(
        encoded(&[first.clone(), legacy(&[0xfe, 0xff, 2, 0xff, 0xff])]).as_ref(),
        wire
    );
    let (decoded, next) = decode_bacnet_property_value(&wire, 0).unwrap();
    assert_eq!((decoded, next), (first, 11));

    // A [3] of two octets, or one followed by anything but the end or a [0],
    // is payload: the value runs on to the last pair.
    for after in [
        &[0x3a, 0x00, 0x05, 0x09, 0x53][..],
        &[0x3a, 0x05, 0x09, 0x53],
        &[0x39, 0x05, 0x19, 0x53],
    ] {
        let mut wire = vec![0x09, 0x53, 0x2e, 0xfe, 0xff, 0x01, 0xff, 0xff, 0x2f];
        wire.extend_from_slice(after);
        wire.extend_from_slice(&[0xff, 0xff, 0x2f]);
        let (decoded, end) = decode_bacnet_property_value(&wire, 0).unwrap();
        assert_eq!(decoded.value, wire[3..wire.len() - 1]);
        assert_eq!(end, wire.len());
    }
}

#[test]
fn complex_event_type_refuses_a_legacy_value() {
    // The `[255]` tag isn't a valid tag, so the list's own value check
    // refuses the value before the legacy check sees it; its decoder
    // refuses one the same way.
    let params = NotificationParameters::ComplexEventType {
        property_values: vec![legacy(&[0xfe, 0xff, 1, 0xff, 0xff])],
    };
    let mut buf = BytesMut::from(&[0xaa][..]);
    assert!(encode_notification_parameters(&params, &mut buf).is_err());
    assert_eq!(buf.as_ref(), [0xaa]);
    let wire = [
        0x6e, 0x09, 0x53, 0x2e, 0xfe, 0xff, 1, 0xff, 0xff, 0x2f, 0x6f,
    ];
    assert!(decode_notification_parameters(&wire, 0).is_err());
}
