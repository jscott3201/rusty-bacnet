use super::*;
use bacnet_types::constructed::BACnetLiftCarCallList;

fn list(floors: &[u8]) -> BACnetLiftCarCallList {
    BACnetLiftCarCallList {
        floor_numbers: floors.to_vec(),
    }
}

fn encode(value: &BACnetLiftCarCallList) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_lift_car_call_list(&mut buf, value);
    buf.to_vec()
}

/// Independent golden vectors: floor-numbers is opening/closing tag 0 around
/// application-tagged Unsigned (tag 2) floors.
fn golden_vectors() -> Vec<(BACnetLiftCarCallList, Vec<u8>)> {
    vec![
        // No registered calls: the empty frame.
        (list(&[]), vec![0x0E, 0x0F]),
        (list(&[5]), vec![0x0E, 0x21, 0x05, 0x0F]),
        // Floor 0 still takes one content octet; 255 is the Unsigned8 limit.
        (
            list(&[3, 0, 255]),
            vec![0x0E, 0x21, 0x03, 0x21, 0x00, 0x21, 0xFF, 0x0F],
        ),
    ]
}

#[test]
fn lift_car_call_list_golden_vectors_encode_and_decode() {
    for (value, wire) in golden_vectors() {
        assert_eq!(encode(&value), wire, "{value:?}");
        assert_eq!(
            decode_lift_car_call_list(&wire, 0).unwrap(),
            (value, wire.len())
        );
    }
}

#[test]
fn lift_car_call_list_decodes_at_an_offset_and_stops_at_its_closing_tag() {
    let mut data = vec![0xAA, 0xBB];
    let value = list(&[7, 2]);
    data.extend(encode(&value));
    let end = data.len();
    // A second element follows, as in a Registered_Car_Call array.
    data.extend([0x0E, 0x0F]);
    assert_eq!(decode_lift_car_call_list(&data, 2).unwrap(), (value, end));
    assert_eq!(
        decode_lift_car_call_list(&data, end).unwrap(),
        (list(&[]), data.len())
    );
    // A non-minimal but well-formed Unsigned still fits.
    assert_eq!(
        decode_lift_car_call_list(&[0x0E, 0x22, 0x00, 0x09, 0x0F], 0)
            .unwrap()
            .0,
        list(&[9])
    );
}

#[test]
fn lift_car_call_list_rejects_malformed_values() {
    for (wire, context) in [
        (&[][..], "empty input"),
        (&[0x21, 0x05], "no frame"),
        (&[0x1E, 0x21, 0x05, 0x1F], "frame on the wrong tag"),
        (&[0x0E], "no closing tag"),
        (&[0x0E, 0x21, 0x05], "truncated before the closing tag"),
        (&[0x0E, 0x09, 0x05, 0x0F], "context-tagged floor"),
        (&[0x0E, 0x91, 0x05, 0x0F], "Enumerated floor"),
        (&[0x0E, 0x20, 0x0F], "empty Unsigned"),
        (&[0x0E, 0x22, 0x05, 0x0F], "floor overruns the closing tag"),
        (&[0x0E, 0x1E, 0x1F, 0x0F], "nested frame"),
    ] {
        let error = decode_lift_car_call_list(wire, 0).unwrap_err();
        assert!(
            !matches!(error, Error::Range(_)),
            "{context}: {wire:02X?} is malformed, not out of range: {error:?}"
        );
    }
}

#[test]
fn lift_car_call_list_reports_oversized_floors_as_range_errors() {
    for (context, wire) in [
        ("floor 256", &[0x0E, 0x22, 0x01, 0x00, 0x0F][..]),
        (
            "floor 256 after floor 2",
            &[0x0E, 0x21, 0x02, 0x22, 0x01, 0x00, 0x0F],
        ),
        (
            "nine-octet floor",
            &[
                0x0E, 0x25, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F,
            ],
        ),
    ] {
        match decode_lift_car_call_list(wire, 0) {
            Err(Error::Range(refused)) => {
                let message = refused.label;
                assert!(message.contains("floor number"), "{context}: {message}")
            }
            other => panic!("{context}: expected a range error, got {other:?}"),
        }
    }
    // A malformed entry anywhere takes precedence over an oversized one.
    let error =
        decode_lift_car_call_list(&[0x0E, 0x22, 0x01, 0x00, 0x91, 0x01, 0x0F], 0).unwrap_err();
    assert!(!matches!(error, Error::Range(_)), "{error:?}");
}

#[test]
fn lift_car_call_list_rejects_more_than_the_item_limit() {
    let mut wire = vec![0x0E];
    for _ in 0..=MAX_FRAMED_ITEMS {
        wire.extend([0x21, 0x01]);
    }
    wire.push(0x0F);
    assert!(matches!(
        decode_lift_car_call_list(&wire, 0),
        Err(Error::Decoding(_))
    ));
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    let mut framed = 0;
    for (_, wire) in golden_vectors() {
        framed += super::assert_members_cut_short("BACnetLiftCarCallList", &wire, |data| {
            decode_lift_car_call_list(data, 0)
        });
    }
    assert!(framed > 0);
}
