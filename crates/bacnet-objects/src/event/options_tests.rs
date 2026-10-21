//! Reads and writes of Event_Message_Texts_Config and the
//! Event_Algorithm_Inhibit pair (#1329).

use super::*;
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

fn code(result: Option<Result<(), Error>>) -> ErrorCode {
    match result {
        Some(Err(Error::Protocol { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            code
        }
        other => panic!("expected a PROPERTY refusal, got {other:?}"),
    }
}

fn text(text: &str) -> PropertyValue {
    PropertyValue::CharacterString(text.into())
}

fn texts(texts: &[&str]) -> PropertyValue {
    PropertyValue::List(texts.iter().map(|t| text(t)).collect())
}

fn encoded(reference: &BACnetObjectPropertyReference) -> PropertyValue {
    let mut octets = BytesMut::new();
    encode_object_property_reference(&mut octets, reference);
    PropertyValue::ApplicationData(octets.to_vec())
}

fn reference_read(options: &ReportingOptions) -> PropertyValue {
    options
        .read(P::EVENT_ALGORITHM_INHIBIT_REF, None)
        .unwrap()
        .unwrap()
}

fn binary_value(instance: u32) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::BINARY_VALUE, instance).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    )
}

#[test]
fn message_texts_config_reads_and_writes_as_a_three_element_array() {
    let mut options = ReportingOptions::default();
    let read = |options: &ReportingOptions, index| {
        options
            .read(P::EVENT_MESSAGE_TEXTS_CONFIG, index)
            .unwrap()
            .unwrap()
    };
    assert_eq!(read(&options, None), texts(&["", "", ""]));
    assert_eq!(read(&options, Some(0)), PropertyValue::Unsigned(3));
    assert_eq!(options.message_text(EventTransition::ToFault), None);

    let configured = texts(&["High", "Broken", "Back"]);
    options
        .write(P::EVENT_MESSAGE_TEXTS_CONFIG, None, &configured, true)
        .unwrap()
        .unwrap();
    assert_eq!(read(&options, None), configured);
    options
        .write(
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            Some(2),
            &text("Sensor"),
            true,
        )
        .unwrap()
        .unwrap();
    assert_eq!(read(&options, Some(2)), text("Sensor"));
    assert_eq!(
        options.message_text(EventTransition::ToOffnormal),
        Some("High")
    );
    assert_eq!(
        options.message_text(EventTransition::ToFault),
        Some("Sensor")
    );
    assert_eq!(
        options.message_text(EventTransition::ToNormal),
        Some("Back")
    );

    let before = options.clone();
    for (index, value, expected) in [
        (None, texts(&["a", "b"]), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            None,
            texts(&["a", "b", "c", "d"]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (None, text("alone"), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            None,
            PropertyValue::List(vec![text("a"), PropertyValue::Unsigned(1), text("c")]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            Some(0),
            PropertyValue::Unsigned(4),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (Some(4), text("x"), ErrorCode::INVALID_ARRAY_INDEX),
        (
            Some(1),
            PropertyValue::Boolean(true),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        assert_eq!(
            code(options.write(P::EVENT_MESSAGE_TEXTS_CONFIG, index, &value, true)),
            expected,
            "{index:?} {value:?}"
        );
        assert_eq!(options, before);
    }
    assert!(matches!(
        options.read(P::EVENT_MESSAGE_TEXTS_CONFIG, Some(4)),
        Some(Err(_))
    ));
}

#[test]
fn the_inhibit_is_writable_only_without_a_reference_and_with_detection_on() {
    let mut options = ReportingOptions::default();
    let on = PropertyValue::Boolean(true);
    // Unset, the reference reads as the reserved instance.
    assert_eq!(
        reference_read(&options),
        encoded(&BACnetObjectPropertyReference::new(
            ObjectIdentifier::new(ObjectType::BINARY_VALUE, 4_194_303).unwrap(),
            P::PRESENT_VALUE.to_raw(),
        ))
    );
    options
        .write(P::EVENT_ALGORITHM_INHIBIT, None, &on, true)
        .unwrap()
        .unwrap();
    assert!(options.inhibited());
    assert_eq!(
        code(options.write(
            P::EVENT_ALGORITHM_INHIBIT,
            None,
            &PropertyValue::Boolean(false),
            false
        )),
        ErrorCode::WRITE_ACCESS_DENIED,
        "Event_Detection_Enable FALSE"
    );
    assert_eq!(
        code(options.write(
            P::EVENT_ALGORITHM_INHIBIT,
            None,
            &PropertyValue::Unsigned(0),
            true
        )),
        ErrorCode::INVALID_DATA_TYPE
    );
    assert_eq!(
        code(options.write(P::EVENT_ALGORITHM_INHIBIT, Some(1), &on, true)),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY
    );
    // Without a reference there is nothing to follow.
    assert!(!options.follow(false));
    assert!(options.inhibited());

    // With one, the inhibit follows it and a write is refused.
    let reference = binary_value(7);
    options
        .write(
            P::EVENT_ALGORITHM_INHIBIT_REF,
            None,
            &encoded(&reference),
            true,
        )
        .unwrap()
        .unwrap();
    assert_eq!(options.inhibit_reference(), Some(&reference));
    assert_eq!(reference_read(&options), encoded(&reference));
    assert_eq!(
        code(options.write(P::EVENT_ALGORITHM_INHIBIT, None, &on, true)),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    assert!(options.follow(false));
    assert!(!options.inhibited());
    assert!(!options.follow(false), "no change");

    // Writing the reserved instance clears the reference again, and the
    // inhibit it left TRUE goes back to FALSE.
    assert!(options.follow(true));
    assert!(options.inhibited());
    options
        .write(
            P::EVENT_ALGORITHM_INHIBIT_REF,
            None,
            &encoded(&binary_value(4_194_303)),
            true,
        )
        .unwrap()
        .unwrap();
    assert_eq!(options.inhibit_reference(), None);
    assert!(!options.inhibited());
    // Clearing a reference that isn't there leaves a written inhibit alone.
    options
        .write(P::EVENT_ALGORITHM_INHIBIT, None, &on, true)
        .unwrap()
        .unwrap();
    options
        .write(
            P::EVENT_ALGORITHM_INHIBIT_REF,
            None,
            &encoded(&binary_value(4_194_303)),
            true,
        )
        .unwrap()
        .unwrap();
    assert!(options.inhibited());
    options
        .write(P::EVENT_ALGORITHM_INHIBIT, None, &on, true)
        .unwrap()
        .unwrap();
    assert_eq!(
        code(options.write(P::EVENT_ALGORITHM_INHIBIT_REF, None, &on, true)),
        ErrorCode::INVALID_DATA_TYPE
    );
}

#[test]
fn other_properties_are_left_to_the_object() {
    let mut options = ReportingOptions::default();
    assert!(options.read(P::EVENT_MESSAGE_TEXTS, None).is_none());
    assert!(options
        .write(P::DESCRIPTION, None, &text("d"), true)
        .is_none());
    for row in REPORTING_OPTION_METADATA {
        assert!(!row.is_required());
        assert!(row.write_capability.is_writable());
    }
}
