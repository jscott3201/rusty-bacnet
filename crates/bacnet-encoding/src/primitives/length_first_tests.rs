//! The error-kind rule of `constructed::tagged` for the application values
//! and timestamps decoded here (#1333): a fixed-size value whose header gives
//! the wrong length is malformed even when the data also stops early, and
//! contents that run past the end of the data are a short buffer.

use super::*;

/// The need and have of a short-buffer error, failing on any other result.
fn short<T: std::fmt::Debug>(result: Result<T, Error>) -> (usize, usize) {
    match result {
        Err(Error::BufferTooShort { need, have }) => (need, have),
        other => panic!("expected a short buffer, got {other:?}"),
    }
}

/// The message of a decoding error, failing on any other result.
fn malformed<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::Decoding(error)) => error.message().to_string(),
        other => panic!("expected a decoding error, got {other:?}"),
    }
}

#[test]
fn fixed_size_application_values_check_their_length_first() {
    // Each type's right length with its contents cut short, then a wrong
    // length cut short too.
    let cases: [(&str, &[u8], usize, &[u8]); 5] = [
        ("REAL", &[0x44, 0x42, 0x90], 5, &[0x45, 0x05, 0x42]),
        ("Double", &[0x55, 0x08, 1, 2, 3], 10, &[0x54, 1, 2]),
        ("Date", &[0xA4, 1, 2], 5, &[0xA3, 1]),
        ("Time", &[0xB4, 1, 2], 5, &[0xB3, 1]),
        (
            "BACnetObjectIdentifier",
            &[0xC4, 0x00, 0x80],
            5,
            &[0xC5, 0x05, 0x00],
        ),
    ];
    for (kind, cut, need, wrong) in cases {
        assert_eq!(
            short(decode_application_value(cut, 0)),
            (need, cut.len()),
            "{kind}"
        );
        let message = malformed(decode_application_value(wrong, 0));
        assert!(
            message.starts_with(&format!("application {kind} has ")),
            "{kind}: {message}"
        );
    }
    // A wrong length with every contents octet present is malformed as well.
    assert!(malformed(decode_application_value(&[0x43, 1, 2, 3], 0)).contains("expected 4"));
}

#[test]
fn variable_length_application_values_cut_short_are_a_short_buffer() {
    // An Unsigned and a CharacterString announcing more than they hold.
    assert_eq!(short(decode_application_value(&[0x22, 0x01], 0)), (3, 2));
    assert_eq!(
        short(decode_application_value(&[0x75, 0x06, 0x00, b'a'], 0)),
        (8, 4)
    );
}

#[test]
fn timestamp_members_check_their_length_then_their_contents() {
    // A Time under [0] with three of its four octets, and a [0] of three
    // octets with one present.
    assert_eq!(short(decode_timestamp_choice(&[0x0C, 14, 30], 0)), (5, 3));
    assert!(malformed(decode_timestamp_choice(&[0x0B, 14], 0)).contains("Time has 3"));
    // A sequence number that says two octets and holds one.
    assert_eq!(short(decode_timestamp_choice(&[0x1A, 0x00], 0)), (3, 2));
    // A DateTime whose Date, then whose Time, is cut short; and a Date of
    // three octets.
    assert_eq!(
        short(decode_timestamp_choice(&[0x2E, 0xA4, 1, 2], 0)),
        (6, 4)
    );
    assert_eq!(
        short(decode_timestamp_choice(
            &[0x2E, 0xA4, 1, 2, 3, 4, 0xB4, 1],
            0
        )),
        (11, 8)
    );
    assert!(malformed(decode_timestamp_choice(&[0x2E, 0xA3, 1], 0)).contains("Date has 3"));
    // The same inside the enclosing field's frame.
    assert_eq!(short(decode_timestamp(&[0x3E, 0x0C, 14, 30], 0, 3)), (6, 4));
}
