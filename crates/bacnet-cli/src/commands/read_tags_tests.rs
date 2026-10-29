use super::*;

#[test]
fn tags_formatter_preserves_names_values_and_shapes() {
    let semantic = [0x0a, 0, b'a'];
    let null = [0x0a, 0, b'a', 0];
    assert_eq!(
        format_read_value(
            &formatter(AI),
            PropertyIdentifier::TAGS,
            None,
            &[semantic.as_slice(), &null].concat()
        ),
        "[\"a\", \"a\"=null]"
    );
    assert_eq!(
        format_read_value(&formatter(AI), PropertyIdentifier::TAGS, Some(1), &null),
        "\"a\"=null"
    );
    assert_eq!(
        format_read_value(&formatter(AI), PropertyIdentifier::TAGS, None, &[]),
        "[]"
    );
    assert_eq!(
        format_read_value(
            &formatter(AI),
            PropertyIdentifier::TAGS,
            Some(0),
            &[0x21, 0]
        ),
        "0"
    );
    assert_eq!(
        format_read_value(
            &formatter(AI),
            PropertyIdentifier::TAGS,
            Some(1),
            &[0x0a, 0, b'\n', 0x72, 0, b'"']
        ),
        "\"\\n\"=\"\\\"\""
    );
    assert_eq!(
        format_read_value(
            &formatter(AI),
            PropertyIdentifier::TAGS,
            Some(1),
            &[0x0a, 0, b'd', 0xa4, 126, 10, 8, 4]
        ),
        "\"d\"=2026-10-08"
    );
    assert_eq!(
        format_read_value(
            &formatter(AI),
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
            format_read_value(&formatter(AI), PropertyIdentifier::TAGS, index, &bytes),
            format!("[invalid Tags] [raw {}]", hex(&bytes))
        );
    }
    // Any other property goes through the shared formatter.
    let value = [0x21, 42];
    assert_eq!(
        format_read_value(
            &formatter(AI),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &value
        ),
        "42"
    );
}

/// Read values are named for their object and property (#2078): an
/// enumeration by name, bits by name, a binary state, and every element of
/// a long list, which the compact places would cut short.
#[test]
fn read_values_go_through_the_shared_formatter() {
    let event_state = [0x91, 0x02];
    assert_eq!(
        format_read_value(
            &formatter(AI),
            PropertyIdentifier::EVENT_STATE,
            None,
            &event_state
        ),
        "offnormal"
    );
    let flags = [0x82, 0x04, 0x40];
    assert_eq!(
        format_read_value(
            &formatter(AI),
            PropertyIdentifier::STATUS_FLAGS,
            None,
            &flags
        ),
        "fault"
    );
    assert_eq!(
        format_read_value(
            &formatter(ObjectType::BINARY_INPUT),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &[0x91, 0x01]
        ),
        "active"
    );
    let list: Vec<u8> = (0..10u8).flat_map(|n| [0x21, n]).collect();
    assert_eq!(
        format_read_value(&formatter(AI), PropertyIdentifier::DESCRIPTION, None, &list),
        "[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]"
    );
    // A REAL keeps every digit.
    let real = [0x44, 0x42, 0x55, 0x22, 0x22];
    assert_eq!(
        format_read_value(
            &formatter(AI),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            &real
        ),
        "53.283333"
    );
}

/// List items split as the property's constructed elements, or as
/// application values with the undecodable tail kept as hex.
#[test]
fn list_items_split_by_element_or_value() {
    let format = formatter(ObjectType::DEVICE);
    let stamps = [0x19, 0x07, 0x19, 0x09];
    assert_eq!(
        format_items(
            &formatter(AI),
            PropertyIdentifier::EVENT_TIME_STAMPS,
            &stamps
        ),
        (vec!["seq 7".to_string(), "seq 9".to_string()], None)
    );
    assert_eq!(
        format_items(
            &format,
            PropertyIdentifier::DESCRIPTION,
            &[0x21, 0x05, 0xFF]
        ),
        (vec!["5".to_string()], Some("[raw ff]".to_string()))
    );
}

const AI: ObjectType = ObjectType::ANALOG_INPUT;
