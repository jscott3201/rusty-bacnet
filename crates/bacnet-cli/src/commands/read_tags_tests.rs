use super::*;

#[test]
fn tags_formatter_preserves_names_values_and_shapes() {
    let semantic = [0x0a, 0, b'a'];
    let null = [0x0a, 0, b'a', 0];
    assert_eq!(
        format_read_value(
            PropertyIdentifier::TAGS,
            None,
            &[semantic.as_slice(), &null].concat()
        ),
        "[\"a\", \"a\"=null]"
    );
    assert_eq!(
        format_read_value(PropertyIdentifier::TAGS, Some(1), &null),
        "\"a\"=null"
    );
    assert_eq!(format_read_value(PropertyIdentifier::TAGS, None, &[]), "[]");
    assert_eq!(
        format_read_value(PropertyIdentifier::TAGS, Some(0), &[0x21, 0]),
        "0"
    );
    assert_eq!(
        format_read_value(
            PropertyIdentifier::TAGS,
            Some(1),
            &[0x0a, 0, b'\n', 0x72, 0, b'"']
        ),
        "\"\\n\"=\"\\\"\""
    );
    assert_eq!(
        format_read_value(
            PropertyIdentifier::TAGS,
            Some(1),
            &[0x0a, 0, b'd', 0xa4, 126, 10, 8, 4]
        ),
        "\"d\"=2026-10-08"
    );
    assert_eq!(
        format_read_value(
            PropertyIdentifier::TAGS,
            Some(1),
            &[0x0a, 0, b't', 0xb4, 12, 34, 56, 7]
        ),
        "\"t\"=12:34:56.07"
    );
}

#[test]
fn tags_formatter_reports_whole_failure_and_preserves_other_formatters() {
    let valid = [0x0a, 0, b'a'];
    for (index, bytes) in [
        (None, [valid.as_slice(), &[0x0a, 0, 255]].concat()),
        (Some(1), [valid.as_slice(), &valid].concat()),
        (Some(0), vec![0x21, 1, 0]),
        (Some(1), vec![]),
        (
            None,
            vec![0x0a, 0, b'a', 0xa4, 126, 10, 8, 4, 0xb4, 12, 34, 56, 7],
        ),
    ] {
        assert_eq!(
            format_read_value(PropertyIdentifier::TAGS, index, &bytes),
            format!("[invalid Tags; raw: {}]", hex(&bytes))
        );
    }
    let value = [0x21, 42];
    assert_eq!(
        format_read_value(PropertyIdentifier::PRESENT_VALUE, None, &value),
        decode_and_format(&value)
    );
    // Generic ReadRange formatting is still independently available.
    assert_eq!(decode_and_format(&value), "42");
}
