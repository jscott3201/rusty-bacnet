//! The configuration objects the table builds (#2208): each starts inert,
//! and the properties it serves read-only take a creation-time value
//! through `initialize_property`, which WriteProperty still refuses.

use super::*;
use bacnet_encoding::constructed::{
    encode_device_object_property_reference, encode_device_object_reference,
};
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::trend::MAX_CREATED_BUFFER_SIZE;

fn build(kind: ObjectType) -> Box<dyn BACnetObject> {
    builtin_constructor(kind).expect("in the table")(3, "made".into(), &[]).unwrap()
}

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

#[track_caller]
fn assert_code(result: Result<(), Error>, expected: ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!((class, code), (ErrorClass::PROPERTY, expected));
        }
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

fn unsigned_list(values: &[u64]) -> PropertyValue {
    PropertyValue::List(
        values
            .iter()
            .copied()
            .map(PropertyValue::Unsigned)
            .collect(),
    )
}

fn transitions(byte: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![byte],
    }
}

fn analog_input(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

#[test]
fn each_configuration_type_starts_inert() {
    let calendar = build(ObjectType::CALENDAR);
    assert_eq!(
        read(&*calendar, PropertyIdentifier::DATE_LIST),
        PropertyValue::List(vec![])
    );
    let schedule = build(ObjectType::SCHEDULE);
    assert_eq!(
        read(&*schedule, PropertyIdentifier::SCHEDULE_DEFAULT),
        PropertyValue::Null
    );
    assert_eq!(
        read(&*schedule, PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Null
    );
    let class = build(ObjectType::NOTIFICATION_CLASS);
    assert_eq!(
        read(&*class, PropertyIdentifier::PRIORITY),
        unsigned_list(&[255, 255, 255])
    );
    let log = build(ObjectType::TREND_LOG);
    assert_eq!(
        read(&*log, PropertyIdentifier::BUFFER_SIZE),
        PropertyValue::Unsigned(u64::from(CREATED_BUFFER_SIZE))
    );
    let enrollment = build(ObjectType::EVENT_ENROLLMENT);
    assert_eq!(
        read(&*enrollment, PropertyIdentifier::EVENT_TYPE),
        PropertyValue::Enumerated(EventType::NONE.to_raw())
    );
    let view = build(ObjectType::STRUCTURED_VIEW);
    assert_eq!(
        read(&*view, PropertyIdentifier::SUBORDINATE_LIST),
        PropertyValue::List(vec![])
    );
}

#[test]
fn each_type_whose_name_writes_cant_change_takes_one_at_creation() {
    let name = PropertyIdentifier::OBJECT_NAME;
    for kind in [
        ObjectType::CALENDAR,
        ObjectType::SCHEDULE,
        ObjectType::NOTIFICATION_CLASS,
        ObjectType::TREND_LOG,
        ObjectType::STRUCTURED_VIEW,
    ] {
        let mut object = build(kind);
        let named = PropertyValue::CharacterString("Lobby".into());
        assert_code(
            object.initialize_property(name, PropertyValue::CharacterString(String::new())),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_code(
            object.initialize_property(name, PropertyValue::Unsigned(1)),
            ErrorCode::INVALID_DATA_TYPE,
        );
        object.initialize_property(name, named.clone()).unwrap();
        assert_eq!(object.object_name(), "Lobby", "{kind:?}");
        assert_code(
            object.write_property(name, None, named, None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    // An Event Enrollment's name is writable, so it goes the write route.
    let enrollment = build(ObjectType::EVENT_ENROLLMENT);
    assert!(!enrollment.creation_only_properties().contains(&name));
}

#[test]
fn a_notification_class_takes_its_priorities_and_ack_required_only_at_creation() {
    let mut class = build(ObjectType::NOTIFICATION_CLASS);
    assert_eq!(
        class.creation_only_properties(),
        [
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::PRIORITY,
            PropertyIdentifier::ACK_REQUIRED
        ]
    );
    let priority = PropertyIdentifier::PRIORITY;
    for (value, code) in [
        (unsigned_list(&[1, 2]), ErrorCode::VALUE_OUT_OF_RANGE),
        (unsigned_list(&[1, 2, 3, 4]), ErrorCode::VALUE_OUT_OF_RANGE),
        (unsigned_list(&[1, 256, 3]), ErrorCode::VALUE_OUT_OF_RANGE),
        (PropertyValue::Unsigned(1), ErrorCode::VALUE_OUT_OF_RANGE),
        (PropertyValue::Real(1.0), ErrorCode::INVALID_DATA_TYPE),
        (
            PropertyValue::List(vec![PropertyValue::Real(1.0); 3]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        assert_code(class.initialize_property(priority, value), code);
    }
    // A refusal left the defaults.
    assert_eq!(read(&*class, priority), unsigned_list(&[255, 255, 255]));
    class
        .initialize_property(priority, unsigned_list(&[10, 20, 0]))
        .unwrap();
    assert_eq!(read(&*class, priority), unsigned_list(&[10, 20, 0]));

    let ack = PropertyIdentifier::ACK_REQUIRED;
    assert_code(
        class.initialize_property(
            ack,
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0xA0],
            },
        ),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    class.initialize_property(ack, transitions(0xA0)).unwrap();
    assert_eq!(read(&*class, ack), transitions(0xA0));

    // WriteProperty still can't change either.
    for (property, value) in [
        (priority, unsigned_list(&[1, 1, 1])),
        (ack, transitions(0xE0)),
    ] {
        assert_code(
            class.write_property(property, None, value, None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
}

#[test]
fn a_trend_log_takes_its_buffer_size_only_at_creation_up_to_the_cap() {
    let mut log = build(ObjectType::TREND_LOG);
    let size = PropertyIdentifier::BUFFER_SIZE;
    assert_eq!(
        log.creation_only_properties(),
        [PropertyIdentifier::OBJECT_NAME, size]
    );
    assert_code(
        log.initialize_property(
            size,
            PropertyValue::Unsigned(u64::from(MAX_CREATED_BUFFER_SIZE) + 1),
        ),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_code(
        log.initialize_property(size, PropertyValue::Real(5.0)),
        ErrorCode::INVALID_DATA_TYPE,
    );
    log.initialize_property(
        size,
        PropertyValue::Unsigned(u64::from(MAX_CREATED_BUFFER_SIZE)),
    )
    .unwrap();
    log.initialize_property(size, PropertyValue::Unsigned(0))
        .unwrap();
    log.initialize_property(size, PropertyValue::Unsigned(500))
        .unwrap();
    assert_eq!(read(&*log, size), PropertyValue::Unsigned(500));
    assert_code(
        log.write_property(size, None, PropertyValue::Unsigned(9), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn an_event_enrollment_takes_its_reference_only_at_creation() {
    let mut enrollment = build(ObjectType::EVENT_ENROLLMENT);
    let property = PropertyIdentifier::OBJECT_PROPERTY_REFERENCE;
    assert_eq!(enrollment.creation_only_properties(), [property]);
    let encode = |reference: BACnetDeviceObjectPropertyReference| {
        let mut buf = BytesMut::new();
        encode_device_object_property_reference(&mut buf, &reference);
        PropertyValue::ApplicationData(buf.to_vec())
    };
    let reference = BACnetDeviceObjectPropertyReference::new_local(
        analog_input(4),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    let mut foreign = reference.clone();
    foreign.device_identifier = Some(analog_input(9));
    assert_code(
        enrollment.initialize_property(property, encode(foreign)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_code(
        enrollment.initialize_property(property, PropertyValue::Null),
        ErrorCode::INVALID_DATA_TYPE,
    );
    enrollment
        .initialize_property(property, encode(reference.clone()))
        .unwrap();
    assert_eq!(read(&*enrollment, property), encode(reference.clone()));
    assert_code(
        enrollment.write_property(property, None, encode(reference), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

fn subordinates(instances: &[u32]) -> PropertyValue {
    let mut buf = BytesMut::new();
    for instance in instances {
        encode_device_object_reference(
            &mut buf,
            &BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: analog_input(*instance),
            },
        );
    }
    PropertyValue::ApplicationData(buf.to_vec())
}

fn texts(labels: &[&str]) -> PropertyValue {
    PropertyValue::List(
        labels
            .iter()
            .map(|label| PropertyValue::CharacterString((*label).into()))
            .collect(),
    )
}

fn array_size(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, Some(0)).unwrap()
}

#[test]
fn a_structured_view_takes_its_node_and_subordinates_only_at_creation() {
    let mut view = build(ObjectType::STRUCTURED_VIEW);
    let node_type = PropertyIdentifier::NODE_TYPE;
    assert_code(
        view.initialize_property(node_type, PropertyValue::Enumerated(22)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    view.initialize_property(node_type, PropertyValue::Enumerated(21))
        .unwrap();
    view.initialize_property(
        PropertyIdentifier::NODE_SUBTYPE,
        PropertyValue::CharacterString("AHU".into()),
    )
    .unwrap();
    assert_eq!(read(&*view, node_type), PropertyValue::Enumerated(21));
    assert_code(
        view.write_property(node_type, None, PropertyValue::Enumerated(1), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_code(
        view.initialize_property(
            PropertyIdentifier::SUBORDINATE_ANNOTATIONS,
            PropertyValue::Unsigned(1),
        ),
        ErrorCode::INVALID_DATA_TYPE,
    );
}

#[test]
fn subordinates_and_annotations_keep_one_size_in_either_order() {
    let list = PropertyIdentifier::SUBORDINATE_LIST;
    let notes = PropertyIdentifier::SUBORDINATE_ANNOTATIONS;
    let mut list_first = build(ObjectType::STRUCTURED_VIEW);
    list_first
        .initialize_property(list, subordinates(&[1, 2]))
        .unwrap();
    // The list sizes the annotations, each empty until given.
    assert_eq!(read(&*list_first, notes), texts(&["", ""]));
    list_first
        .initialize_property(notes, texts(&["supply", "return"]))
        .unwrap();
    let mut notes_first = build(ObjectType::STRUCTURED_VIEW);
    notes_first
        .initialize_property(notes, texts(&["supply", "return"]))
        .unwrap();
    assert_eq!(array_size(&*notes_first, list), PropertyValue::Unsigned(2));
    notes_first
        .initialize_property(list, subordinates(&[1, 2]))
        .unwrap();
    for view in [&list_first, &notes_first] {
        assert_eq!(read(&**view, list), read(&*list_first, list));
        assert_eq!(read(&**view, notes), texts(&["supply", "return"]));
    }
    // Once both are given, a size that differs is refused and changes
    // nothing, in either order.
    assert_code(
        list_first.initialize_property(notes, PropertyValue::CharacterString("only".into())),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_code(
        notes_first.initialize_property(list, subordinates(&[1, 2, 3])),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    for view in [&list_first, &notes_first] {
        assert_eq!(array_size(&**view, list), PropertyValue::Unsigned(2));
        assert_eq!(read(&**view, notes), texts(&["supply", "return"]));
    }
}

#[test]
fn either_list_alone_sets_the_size_up_to_the_cap() {
    let list = PropertyIdentifier::SUBORDINATE_LIST;
    let notes = PropertyIdentifier::SUBORDINATE_ANNOTATIONS;
    let mut view = build(ObjectType::STRUCTURED_VIEW);
    // One string alone decodes as itself.
    view.initialize_property(notes, PropertyValue::CharacterString("only".into()))
        .unwrap();
    assert_eq!(array_size(&*view, list), PropertyValue::Unsigned(1));
    let past_cap = crate::group::MAX_SUBORDINATES as u32 + 1;
    let no_space = |result: Result<(), Error>| match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(
                (class, code),
                (ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)
            );
        }
        other => panic!("{other:?}"),
    };
    let mut fresh = build(ObjectType::STRUCTURED_VIEW);
    let many: Vec<u32> = (1..=past_cap).collect();
    no_space(fresh.initialize_property(list, subordinates(&many)));
    let labels: Vec<String> = (0..past_cap).map(|n| n.to_string()).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    no_space(fresh.initialize_property(notes, texts(&labels)));
    assert_eq!(array_size(&*fresh, list), PropertyValue::Unsigned(0));
    let at_cap: Vec<u32> = (1..past_cap).collect();
    fresh
        .initialize_property(list, subordinates(&at_cap))
        .unwrap();
}
