//! The shared tagged-field decode helpers (#1147): each refusal's kind,
//! offset and wording, on hand-written octets.

use crate::constructed::tagged::*;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

const W: &str = "Thing";

/// The offset and message of a decode error, failing on any other result.
fn decoding<T: std::fmt::Debug>(result: Result<T, Error>) -> (usize, String) {
    match result {
        Err(Error::Decoding(error)) => (error.offset, error.message().to_string()),
        other => panic!("expected a decoding error, got {other:?}"),
    }
}

/// The need and have of a short-buffer error, failing on any other result.
fn short<T: std::fmt::Debug>(result: Result<T, Error>) -> (usize, usize) {
    match result {
        Err(Error::BufferTooShort { need, have }) => (need, have),
        other => panic!("expected a short buffer, got {other:?}"),
    }
}

#[test]
fn peeks_match_only_their_own_tag_and_form() {
    // A primitive [0], an opening [0], a closing [0] and an application tag.
    let data = [0x09, 0x01, 0x0E, 0x0F, 0x21, 0x05];
    assert!(next_is_context(&data, 0, 0).unwrap());
    assert!(!next_is_context(&data, 0, 1).unwrap());
    assert!(!next_is_context(&data, 2, 0).unwrap());
    assert!(!next_is_context(&data, 4, 2).unwrap());
    assert!(next_is_opening(&data, 2, 0).unwrap());
    assert!(!next_is_opening(&data, 0, 0).unwrap());
    assert!(!next_is_opening(&data, 2, 1).unwrap());
    assert!(next_is_closing(&data, 3, 0).unwrap());
    assert!(!next_is_closing(&data, 2, 0).unwrap());
}

#[test]
fn optional_peeks_are_false_at_the_end_but_a_frame_must_close() {
    for offset in [0, 1, 7] {
        let data = &[0x09][..offset.min(1)];
        assert!(!next_is_context(data, offset, 0).unwrap());
        assert!(!next_is_opening(data, offset, 0).unwrap());
    }
    // Running out of data inside a frame is the frame left open.
    let (offset, _) = decoding(next_is_closing(&[0x0E], 1, 0));
    assert_eq!(offset, 1);
    // A malformed tag is an error even for a peek.
    let (offset, _) = decoding(next_is_context(&[0x0D], 0, 0));
    assert_eq!(offset, 1);
}

#[test]
fn frames_refuse_a_wrong_number_or_form_at_the_tag() {
    let data = [0xAA, 0x0E, 0x09, 0x01, 0x0F];
    assert_eq!(expect_opening(&data, 1, 0, W).unwrap(), 2);
    assert_eq!(expect_closing(&data, 4, 0, W).unwrap(), 5);
    // An opening [1] where [0] is due, a primitive [0] and a closing [0].
    for (bytes, expected) in [
        (&[0xAA, 0x1E][..], "Thing: expected opening tag [0]"),
        (&[0xAA, 0x09, 0x01][..], "Thing: expected opening tag [0]"),
        (&[0xAA, 0x0F][..], "Thing: expected opening tag [0]"),
    ] {
        assert_eq!(
            decoding(expect_opening(bytes, 1, 0, W)),
            (1, expected.to_string())
        );
    }
    assert_eq!(
        decoding(expect_closing(&data, 1, 0, W)),
        (1, "Thing: expected closing tag [0]".to_string())
    );
    assert_eq!(
        decoding(expect_closing(&[0xAA, 0x1F], 1, 0, W)),
        (1, "Thing: expected closing tag [0]".to_string())
    );
}

#[test]
fn a_constructed_field_yields_its_balanced_body() {
    // [2] { [0] { 21 05 } 09 01 } then a trailing octet.
    let data = [0x2E, 0x0E, 0x21, 0x05, 0x0F, 0x09, 0x01, 0x2F, 0xAA];
    let (body, end) = decode_ctx_constructed(&data, 0, 2, W).unwrap();
    assert_eq!(body, &data[1..7]);
    assert_eq!(end, 8);
    // The wrong number, and a frame that never closes.
    assert_eq!(decoding(decode_ctx_constructed(&data, 0, 3, W)).0, 0);
    let (_, message) = decoding(decode_ctx_constructed(&data[..7], 0, 2, W));
    assert!(message.contains("missing closing tag 2"), "{message}");
}

#[test]
fn a_primitive_refuses_another_number_class_or_form() {
    for (bytes, number) in [
        (&[0x19, 0x01][..], 0),       // primitive [1] where [0] is due
        (&[0x21, 0x01][..], 2),       // application Unsigned, not context [2]
        (&[0x0E, 0x0F][..], 0),       // opening [0], not primitive
        (&[0xF9, 0x10, 0x01][..], 0), // extended [16] where [0] is due
    ] {
        let mut data = vec![0xAA];
        data.extend_from_slice(bytes);
        assert_eq!(
            decoding(decode_ctx_primitive(&data, 1, number, W)),
            (1, format!("Thing: expected context tag [{number}]"))
        );
    }
    // An extended tag number reads like any other.
    assert_eq!(
        decode_ctx_primitive(&[0xF9, 0x10, 0x07], 0, 16, W).unwrap(),
        (&[0x07][..], 3)
    );
}

#[test]
fn contents_past_the_end_are_a_short_buffer() {
    // [0] says four octets and holds two.
    let data = [0x0C, 0x00, 0x00];
    assert_eq!(short(decode_ctx_primitive(&data, 0, 0, W)), (5, 3));
    assert_eq!(short(decode_ctx_unsigned::<u32>(&data, 0, 0, W)), (5, 3));
    assert_eq!(short(decode_ctx_object_id(&data, 0, 0, W)), (5, 3));
    assert_eq!(short(decode_ctx_real(&data, 0, 0, W)), (5, 3));
    assert_eq!(short(decode_ctx_boolean(&[0x09], 0, 0, W)), (2, 1));
    assert_eq!(short(contents(&data, 1, 4)), (5, 3));
    // A length that would run off the address space saturates.
    assert_eq!(short(contents(&data, usize::MAX - 1, 10)), (usize::MAX, 3));
    // A header cut short is malformed, not short: decode_tag refuses it.
    assert_eq!(decoding(decode_ctx_primitive(&[0x0D], 0, 0, W)).0, 1);
}

#[test]
fn fixed_size_primitives_refuse_any_other_length() {
    for (bytes, expected) in [
        (
            &[0x0B, 1, 2, 3][..],
            "Thing: [0] REAL has 3 contents octets, expected 4",
        ),
        (
            &[0x0D, 5, 1, 2, 3, 4, 5][..],
            "Thing: [0] REAL has 5 contents octets, expected 4",
        ),
    ] {
        assert_eq!(
            decoding(decode_ctx_real(bytes, 0, 0, W)),
            (0, expected.into())
        );
    }
    for (bytes, expected) in [
        (
            &[0x08][..],
            "Thing: [0] BOOLEAN has 0 contents octets, expected 1",
        ),
        (
            &[0x0A, 0, 1][..],
            "Thing: [0] BOOLEAN has 2 contents octets, expected 1",
        ),
    ] {
        assert_eq!(
            decoding(decode_ctx_boolean(bytes, 0, 0, W)),
            (0, expected.into())
        );
    }
    assert_eq!(
        decoding(decode_ctx_object_id(&[0x1B, 0, 0, 1], 0, 1, W)),
        (
            0,
            "Thing: [1] object identifier has 3 contents octets, expected 4".into()
        )
    );
    // A wrong tag names the type expected.
    assert_eq!(
        decoding(decode_ctx_real(&[0x1C, 0, 0, 0, 0], 0, 0, W)),
        (0, "Thing: expected context tag [0] REAL".into())
    );
}

#[test]
fn fixed_size_primitives_decode_their_values() {
    assert_eq!(
        decode_ctx_real(&[0x2C, 0x3F, 0x80, 0, 0], 0, 2, W).unwrap(),
        (1.0, 5)
    );
    assert_eq!(decode_ctx_boolean(&[0x09, 0], 0, 0, W).unwrap(), (false, 2));
    assert_eq!(decode_ctx_boolean(&[0x09, 1], 0, 0, W).unwrap(), (true, 2));
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 5).unwrap();
    assert_eq!(
        decode_ctx_object_id(&[0x3C, 0x02, 0, 0, 5], 0, 3, W).unwrap(),
        (device, 5)
    );
}

#[test]
fn a_boolean_holds_only_zero_or_one() {
    assert_eq!(
        decoding(decode_ctx_boolean(&[0xAA, 0x19, 0x02], 1, 1, W)),
        (2, "Thing: [1] BOOLEAN contents must be 0 or 1".into())
    );
}

#[test]
fn unsigned_fields_narrow_to_their_type() {
    assert_eq!(
        decode_ctx_unsigned::<u8>(&[0x09, 0xFF], 0, 0, W).unwrap(),
        (255, 2)
    );
    assert_eq!(
        decoding(decode_ctx_unsigned::<u8>(&[0x0A, 0x01, 0x00], 0, 0, W)),
        (0, "Thing: [0] value 256 exceeds u8".into())
    );
    assert_eq!(
        decoding(decode_ctx_unsigned::<u16>(
            &[0x1B, 0x01, 0x00, 0x00],
            0,
            1,
            W
        )),
        (0, "Thing: [1] value 65536 exceeds u16".into())
    );
    assert_eq!(
        decoding(decode_ctx_unsigned::<u32>(
            &[0x2D, 5, 1, 0, 0, 0, 0],
            0,
            2,
            W
        )),
        (0, "Thing: [2] value 4294967296 exceeds u32".into())
    );
    let max = [0x0D, 8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    assert_eq!(
        decode_ctx_unsigned::<u64>(&max, 0, 0, W).unwrap(),
        (u64::MAX, 10)
    );
    // Leading zero octets are accepted here.
    assert_eq!(
        decode_ctx_unsigned::<u8>(&[0x0A, 0, 7], 0, 0, W).unwrap(),
        (7, 3)
    );
    // No octets, or more than a u64 holds, are malformed.
    decoding(decode_ctx_unsigned::<u64>(&[0x08], 0, 0, W));
    decoding(decode_ctx_unsigned::<u64>(
        &[0x0D, 9, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        0,
        0,
        W,
    ));
}

#[test]
fn canonical_unsigned_fields_refuse_padding_and_bad_lengths() {
    assert_eq!(
        decode_ctx_canonical_unsigned::<u8>(&[0x09, 0], 0, 0, W).unwrap(),
        (0, 2)
    );
    assert_eq!(
        decoding(decode_ctx_canonical_unsigned::<u8>(
            &[0xAA, 0x0A, 0, 7],
            1,
            0,
            W
        )),
        (
            1,
            "Thing: must use the shortest Unsigned/Enumerated encoding".into()
        )
    );
    assert_eq!(
        decoding(decode_ctx_canonical_unsigned::<u64>(&[0xAA, 0x08], 1, 0, W)),
        (
            1,
            "Thing: has 0 contents octets, expected one to eight".into()
        )
    );
    let nine = [0x0D, 9, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        decoding(decode_ctx_canonical_unsigned::<u64>(&nine, 0, 0, W)),
        (
            0,
            "Thing: has 9 contents octets, expected one to eight".into()
        )
    );
    assert_eq!(
        decoding(decode_ctx_canonical_unsigned::<u8>(&[0x0A, 1, 0], 0, 0, W)),
        (0, "Thing: [0] value 256 exceeds u8".into())
    );
    assert_eq!(
        decode_canonical_unsigned(&[0x12, 0x34], 0, W).unwrap(),
        0x1234
    );
}

#[test]
fn an_optional_field_is_read_only_under_its_own_tag() {
    let data = [0x09, 0x05, 0x19, 0x06];
    let read = |offset, tag| decode_optional_ctx(&data, offset, tag, W, decode_ctx_unsigned::<u8>);
    assert_eq!(read(0, 0).unwrap(), (Some(5), 2));
    assert_eq!(read(0, 1).unwrap(), (None, 0));
    assert_eq!(read(2, 1).unwrap(), (Some(6), 4));
    assert_eq!(read(4, 1).unwrap(), (None, 4));
    // Present but malformed is an error, not an absent field.
    let (offset, message) = decoding(decode_optional_ctx(
        &[0x0A, 1, 0],
        0,
        0,
        W,
        decode_ctx_unsigned::<u8>,
    ));
    assert_eq!(
        (offset, message.as_str()),
        (0, "Thing: [0] value 256 exceeds u8")
    );
    assert_eq!(
        short(decode_optional_ctx(
            &[0x0C, 1],
            0,
            0,
            W,
            decode_ctx_object_id
        )),
        (5, 2)
    );
}

#[test]
fn an_optional_primitive_yields_its_contents_or_nothing() {
    // What tags::decode_optional_context did (#1374): the contents come
    // back borrowed from the input.
    fn read(data: &[u8], tag: u8) -> Result<(Option<&[u8]>, usize), Error> {
        decode_optional_ctx(data, 0, tag, W, decode_ctx_primitive)
    }
    assert_eq!(read(&[0x09, 42], 0).unwrap(), (Some(&[42][..]), 2));
    assert_eq!(read(&[0x19, 42], 0).unwrap(), (None, 0));
    assert_eq!(read(&[], 0).unwrap(), (None, 0));
    assert_eq!(short(read(&[0x0C, 0x01, 0x02], 0)), (5, 3));
}

#[test]
fn an_application_peek_matches_only_its_own_type() {
    // An application Unsigned, a context [2] and an application Date.
    let data = [0x21, 0x05, 0x29, 0x05, 0xA4, 0x7E, 0x0A, 0x03, 0xFF];
    assert!(next_is_application(&data, 0, 2).unwrap());
    assert!(!next_is_application(&data, 0, 3).unwrap());
    assert!(!next_is_application(&data, 2, 2).unwrap());
    assert!(next_is_application(&data, 4, 10).unwrap());
    assert!(!next_is_application(&data, data.len(), 10).unwrap());
    // A reserved application length is a malformed tag, not a mismatch.
    assert!(matches!(
        next_is_application(&[0x26, 0x01], 0, 2),
        Err(Error::Decoding(_))
    ));
}

#[test]
fn an_input_must_end_where_its_value_does() {
    let data = [0x09, 0x01, 0xAA, 0xBB];
    assert!(expect_end(&data, 4, 4, W).is_ok());
    assert_eq!(
        decoding(expect_end(&data, 2, 2, W)),
        (2, "Thing: 2 trailing byte(s)".into())
    );
    // A frame body reports at the frame, given by the caller.
    assert_eq!(decoding(expect_end(&data[2..], 1, 40, W)).0, 40);
}

#[test]
fn bit_and_octet_strings_and_character_strings_read_their_contents() {
    assert_eq!(
        decode_ctx_bit_string(&[0x0A, 0x05, 0xE0], 0, 0, W).unwrap(),
        ((5, vec![0xE0]), 3)
    );
    assert_eq!(
        decoding(decode_ctx_bit_string(&[0x19, 0x00], 0, 0, W)),
        (0, "Thing: expected context tag [0] BIT STRING".into())
    );
    decoding(decode_ctx_bit_string(&[0x09, 0x08], 0, 0, W));
    assert_eq!(
        decode_ctx_octet_string(&[0x2A, 0xDE, 0xAD], 0, 2, W).unwrap(),
        (vec![0xDE, 0xAD], 3)
    );
    assert_eq!(
        decoding(decode_ctx_octet_string(&[0x0A, 0, 0], 0, 2, W)),
        (0, "Thing: expected context tag [2] OCTET STRING".into())
    );
    assert_eq!(
        decode_ctx_character_string(&[0x5B, 0x00, b'o', b'k'], 0, 5, W).unwrap(),
        ("ok".to_string(), 4)
    );
}

#[test]
fn application_items_refuse_another_tag() {
    assert_eq!(
        decode_app_unsigned::<u64>(&[0x21, 0x07], 0, W).unwrap(),
        (7, 2)
    );
    assert_eq!(
        decoding(decode_app_unsigned::<u64>(&[0x09, 0x07], 0, W)),
        (0, "Thing: expected application-tagged Unsigned".into())
    );
    assert_eq!(
        decode_app_enumerated::<u32>(&[0x91, 0x02], 0, W).unwrap(),
        (2, 2)
    );
    assert_eq!(
        decoding(decode_app_enumerated::<u32>(
            &[0x95, 5, 1, 0, 0, 0, 0],
            0,
            W
        )),
        (2, "Thing: ENUMERATED exceeds u32".into())
    );
    assert_eq!(
        decode_app_bit_string(&[0x82, 0x07, 0x80], 0, W).unwrap(),
        ((7, vec![0x80]), 3)
    );
    assert_eq!(
        decoding(decode_app_character_string(&[0x21, 0x00], 0, W)),
        (
            0,
            "Thing: expected application-tagged CharacterString".into()
        )
    );
    assert_eq!(
        short(decode_app_unsigned::<u64>(&[0x22, 0x07], 0, W)),
        (3, 2)
    );
}

#[test]
fn application_primitives_read_any_type_and_name_the_expected_one() {
    use crate::tags::app_tag;
    // An INTEGER, an OCTET STRING and a Date, each read at its own offset.
    let data = [0x31, 0xFF, 0x62, 0xAB, 0xCD, 0xA4, 0x7E, 0x0A, 0x03, 0x05];
    assert_eq!(
        decode_app_primitive(&data, 0, app_tag::SIGNED, W).unwrap(),
        (&[0xFF][..], 2)
    );
    assert_eq!(
        decode_app_primitive(&data, 2, app_tag::OCTET_STRING, W).unwrap(),
        (&[0xAB, 0xCD][..], 5)
    );
    assert_eq!(
        decode_app_primitive(&data, 5, app_tag::DATE, W).unwrap(),
        (&[0x7E, 0x0A, 0x03, 0x05][..], 10)
    );
    // Each refusal names the type it wanted; a context tag of the same
    // number is refused too.
    for (number, kind) in [
        (app_tag::SIGNED, "INTEGER"),
        (app_tag::OCTET_STRING, "OCTET STRING"),
        (app_tag::DATE, "Date"),
        (app_tag::TIME, "Time"),
        (app_tag::OBJECT_IDENTIFIER, "BACnetObjectIdentifier"),
        (app_tag::DOUBLE, "Double"),
        (13, "value of a reserved type"),
    ] {
        assert_eq!(
            decoding(decode_app_primitive(&[0x21, 0x00], 0, number, W)),
            (0, format!("Thing: expected application-tagged {kind}"))
        );
    }
    assert_eq!(
        decoding(decode_app_primitive(&[0x39, 0x01], 0, app_tag::SIGNED, W)),
        (0, "Thing: expected application-tagged INTEGER".into())
    );
    // Contents cut short are a short buffer.
    assert_eq!(
        short(decode_app_primitive(
            &[0x63, 0xAB],
            0,
            app_tag::OCTET_STRING,
            W
        )),
        (4, 2)
    );
}

#[test]
fn an_application_boolean_has_no_contents_to_read() {
    use crate::tags::app_tag;
    // A TRUE BOOLEAN (its value in the length field) and then an Unsigned 5:
    // the BOOLEAN's "contents" would be the Unsigned's tag octet.
    for data in [&[0x11, 0x21, 0x05][..], &[0x10], &[0x21, 0x05]] {
        assert_eq!(
            decoding(decode_app_primitive(data, 0, app_tag::BOOLEAN, W)),
            (
                0,
                "Thing: an application-tagged BOOLEAN has no contents to read".into()
            ),
            "{data:02X?}"
        );
    }
}

#[test]
fn fixed_size_contents_check_their_length_first() {
    // A `[3]` Time with its four octets.
    assert_eq!(
        decode_ctx_fixed(&[0x3C, 0x0C, 0x00, 0x00, 0x00], 0, 3, Fixed::TIME, W).unwrap(),
        (&[0x0C, 0x00, 0x00, 0x00][..], 5)
    );
    // A header announcing five octets is the wrong length, even with three
    // present; one announcing four with three present is cut short.
    assert_eq!(
        decoding(decode_ctx_fixed(
            &[0x3D, 0x05, 0x0C, 0x00, 0x00],
            0,
            3,
            Fixed::TIME,
            W
        )),
        (
            0,
            "Thing: [3] Time has 5 contents octets, expected 4".into()
        )
    );
    assert_eq!(
        short(decode_ctx_fixed(
            &[0x3C, 0x0C, 0x00, 0x00],
            0,
            3,
            Fixed::TIME,
            W
        )),
        (5, 4)
    );
    assert_eq!(
        decoding(decode_ctx_fixed(&[0x2C, 0, 0, 0, 0], 0, 3, Fixed::TIME, W)),
        (0, "Thing: expected context tag [3] Time".into())
    );
    // A REAL is read the same way.
    assert_eq!(
        decoding(decode_ctx_real(&[0x5D, 0x05, 0x42, 0x90, 0x00], 0, 5, W)),
        (
            0,
            "Thing: [5] REAL has 5 contents octets, expected 4".into()
        )
    );
    assert_eq!(
        short(decode_ctx_real(&[0x5C, 0x42, 0x90, 0x00], 0, 5, W)),
        (5, 4)
    );
}

#[test]
fn application_unsigned_values_narrow_to_their_width() {
    assert_eq!(
        decode_app_unsigned::<u8>(&[0x22, 0x00, 0xFF], 0, W).unwrap(),
        (255, 3)
    );
    assert_eq!(
        decoding(decode_app_unsigned::<u8>(&[0xAA, 0x22, 0x01, 0x00], 1, W)),
        (2, "Thing: Unsigned exceeds u8".into())
    );
    assert_eq!(
        decoding(decode_app_unsigned::<u16>(&[0x23, 0x01, 0x00, 0x00], 0, W)),
        (1, "Thing: Unsigned exceeds u16".into())
    );
    assert_eq!(
        decode_app_unsigned::<u32>(&[0x24, 0xFF, 0xFF, 0xFF, 0xFF], 0, W).unwrap(),
        (u32::MAX, 5)
    );
    assert_eq!(
        decoding(decode_app_unsigned::<u32>(&[0x25, 5, 1, 0, 0, 0, 0], 0, W)),
        (2, "Thing: Unsigned exceeds u32".into())
    );
}

#[test]
fn application_enumerated_items_narrow_and_canonical_ones_refuse_padding() {
    // An error class or code fits u16.
    assert_eq!(
        decode_app_enumerated::<u16>(&[0x92, 0xFF, 0xFF], 0, W).unwrap(),
        (0xFFFF, 3)
    );
    assert_eq!(
        decoding(decode_app_enumerated::<u16>(&[0x93, 1, 0, 0], 0, W)),
        (1, "Thing: ENUMERATED exceeds u16".into())
    );
    // Leading zero octets are accepted, unless the codec needs the shortest
    // encoding.
    assert_eq!(
        decode_app_enumerated::<u16>(&[0x92, 0, 7], 0, W).unwrap(),
        (7, 3)
    );
    assert_eq!(
        decode_app_canonical_enumerated::<u16>(&[0x91, 7], 0, W).unwrap(),
        (7, 2)
    );
    assert_eq!(
        decoding(decode_app_canonical_enumerated::<u16>(
            &[0xAA, 0x92, 0, 7],
            1,
            W
        )),
        (
            1,
            "Thing: must use the shortest Unsigned/Enumerated encoding".into()
        )
    );
    assert_eq!(
        decoding(decode_app_canonical_enumerated::<u16>(
            &[0x93, 1, 0, 0],
            0,
            W
        )),
        (1, "Thing: ENUMERATED exceeds u16".into())
    );
    assert_eq!(
        decoding(decode_app_canonical_enumerated::<u16>(&[0x21, 7], 0, W)),
        (0, "Thing: expected application-tagged ENUMERATED".into())
    );
    // Contents cut short are a short buffer for both.
    assert_eq!(
        short(decode_app_enumerated::<u16>(&[0x92, 0], 0, W)),
        (3, 2)
    );
    assert_eq!(
        short(decode_app_canonical_enumerated::<u16>(&[0x92, 0], 0, W)),
        (3, 2)
    );
}

#[test]
fn fixed_size_application_items_check_their_length_first() {
    let device_1 = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    assert_eq!(
        decode_app_object_id(&[0xC4, 0x02, 0x00, 0x00, 0x01], 0, W).unwrap(),
        (device_1, 5)
    );
    // Any other length is malformed, even when the data also stops early.
    assert_eq!(
        decoding(decode_app_object_id(&[0xC3, 0x02, 0x00, 0x00], 0, W)),
        (
            0,
            "Thing: application BACnetObjectIdentifier has 3 contents octets, expected 4".into()
        )
    );
    assert_eq!(
        decoding(decode_app_object_id(&[0xC5, 0x05, 0x02, 0x00], 0, W)),
        (
            0,
            "Thing: application BACnetObjectIdentifier has 5 contents octets, expected 4".into()
        )
    );
    // Four announced and fewer present is a short buffer.
    assert_eq!(
        short(decode_app_object_id(&[0xC4, 0x02, 0x00], 0, W)),
        (5, 3)
    );
    // A context tag, or another application type, is refused at the tag.
    for wrong in [
        [0x0C, 0x02, 0x00, 0x00, 0x01],
        [0x24, 0x02, 0x00, 0x00, 0x01],
    ] {
        assert_eq!(
            decoding(decode_app_object_id(&wrong, 0, W)),
            (
                0,
                "Thing: expected application-tagged BACnetObjectIdentifier".into()
            )
        );
    }
    // A Date is read the same way.
    assert_eq!(
        decode_app_fixed(&[0xA4, 0x7E, 0x0A, 0x03, 0xFF], 0, 10, 4, W).unwrap(),
        (&[0x7E, 0x0A, 0x03, 0xFF][..], 5)
    );
    assert_eq!(
        decoding(decode_app_fixed(&[0xA3, 0x7E, 0x0A, 0x03], 0, 10, 4, W)),
        (
            0,
            "Thing: application Date has 3 contents octets, expected 4".into()
        )
    );
    assert_eq!(
        decoding(decode_app_fixed(&[0x11], 0, 1, 1, W)),
        (
            0,
            "Thing: an application-tagged BOOLEAN has no contents to read".into()
        )
    );
}

/// A closing `[0]` after `depth` opening `[0]`s: whether it closes an open
/// frame, as `misplaced_kind` judges it. The walk keeps 32 frames; past that
/// it says no, so the tag is an invalid one rather than a missing member,
/// which still refuses the input, and nothing panics.
#[test]
fn a_frame_walk_past_its_depth_answers_safely() {
    use crate::tags::{MAX_CONTEXT_NESTING_DEPTH, decode_tag};
    use bacnet_types::error::DecodingKind;

    let kind_at = |depth: usize| {
        let mut data = std::vec![0x0E; depth];
        data.push(0x0F);
        let (closing, _) = decode_tag(&data, depth).unwrap();
        misplaced_kind(&data, depth, &closing, Some(0))
    };
    assert_eq!(MAX_CONTEXT_NESTING_DEPTH, 32);
    assert_eq!(kind_at(1), DecodingKind::Missing);
    assert_eq!(kind_at(32), DecodingKind::Missing);
    assert_eq!(kind_at(33), DecodingKind::InvalidTag);
    assert_eq!(kind_at(300), DecodingKind::InvalidTag);
}
