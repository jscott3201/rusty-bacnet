use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::ObjectIdentifier;

const LIST: PropertyIdentifier = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn member(object_type: ObjectType, instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        oid(object_type, instance),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )
}

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn is_error(error: &Error, class: ErrorClass, code: ErrorCode) -> bool {
    matches!(error, Error::Remote(RemoteError::Error { class: c, code: e })
        if *c == class && *e == code)
}

#[test]
fn python_add_channel_settings_reach_the_channel() {
    let indexed = member(ObjectType::ANALOG_VALUE, 1).with_index(3);
    let settings = ChannelSettings {
        members: Some(vec![member(ObjectType::ANALOG_OUTPUT, 1), indexed.clone()]),
        execution_delay: Some(vec![0, 250]),
        control_groups: Some(vec![5, 7]),
        allow_group_delay_inhibit: true,
    };
    let configured = channel(1, "CH-1", 65_535, settings).unwrap();

    let mut expected = ChannelObject::new(1, "CH-1", 65_535).unwrap();
    expected
        .set_members(vec![member(ObjectType::ANALOG_OUTPUT, 1), indexed])
        .unwrap();
    assert_eq!(read(&configured, LIST), read(&expected, LIST));
    assert_eq!(
        read(&configured, PropertyIdentifier::EXECUTION_DELAY),
        PropertyValue::List(vec![
            PropertyValue::Unsigned(0),
            PropertyValue::Unsigned(250)
        ])
    );
    assert_eq!(
        read(&configured, PropertyIdentifier::CONTROL_GROUPS),
        PropertyValue::List(vec![PropertyValue::Unsigned(5), PropertyValue::Unsigned(7)])
    );
    assert_eq!(
        read(&configured, PropertyIdentifier::CHANNEL_NUMBER),
        PropertyValue::Unsigned(65_535)
    );
    assert_eq!(
        read(&configured, PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT),
        PropertyValue::Boolean(true)
    );

    // Omitted arguments keep the Channel's defaults; members alone get
    // zero delays.
    let settings = ChannelSettings {
        members: Some(vec![member(ObjectType::ANALOG_OUTPUT, 2)]),
        ..ChannelSettings::default()
    };
    let defaults = channel(2, "CH-2", 0, settings).unwrap();
    assert_eq!(
        read(&defaults, PropertyIdentifier::EXECUTION_DELAY),
        PropertyValue::List(vec![PropertyValue::Unsigned(0)])
    );
    assert_eq!(
        read(&defaults, PropertyIdentifier::CONTROL_GROUPS),
        PropertyValue::List(vec![PropertyValue::Unsigned(0)])
    );
    assert_eq!(
        read(&defaults, PropertyIdentifier::ALLOW_GROUP_DELAY_INHIBIT),
        PropertyValue::Boolean(false)
    );

    // A member naming another Device keeps it.
    let mut remote = member(ObjectType::ANALOG_OUTPUT, 1);
    remote.device_identifier = Some(oid(ObjectType::DEVICE, 99));
    let settings = ChannelSettings {
        members: Some(vec![remote.clone()]),
        ..ChannelSettings::default()
    };
    let elsewhere = channel(3, "CH-3", 0, settings).unwrap();
    let mut expected = ChannelObject::new(3, "CH-3", 0).unwrap();
    expected.set_members(vec![remote]).unwrap();
    assert_eq!(read(&elsewhere, LIST), read(&expected, LIST));
    let mut local = ChannelObject::new(3, "CH-3", 0).unwrap();
    local
        .set_members(vec![member(ObjectType::ANALOG_OUTPUT, 1)])
        .unwrap();
    assert_ne!(read(&elsewhere, LIST), read(&local, LIST));
}

#[test]
fn python_add_channel_refusals_come_from_the_channel() {
    let out_of_range = |settings: ChannelSettings, number: u16| {
        let error = channel(3, "CH-3", number, settings).err().unwrap();
        assert!(
            is_error(&error, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE),
            "{error:?}"
        );
    };
    // A channel number past Unsigned16 never gets here: the parameter is a
    // u16, so the binding raises OverflowError for it (#1360).
    // A delay count that isn't the member count, members given or not.
    out_of_range(
        ChannelSettings {
            members: Some(vec![member(ObjectType::ANALOG_OUTPUT, 1)]),
            execution_delay: Some(vec![0, 100]),
            ..ChannelSettings::default()
        },
        1,
    );
    out_of_range(
        ChannelSettings {
            execution_delay: Some(vec![100]),
            ..ChannelSettings::default()
        },
        1,
    );
    // No control group at all.
    out_of_range(
        ChannelSettings {
            control_groups: Some(vec![]),
            ..ChannelSettings::default()
        },
        1,
    );

    let error = channel(
        4,
        "CH-4",
        1,
        ChannelSettings {
            control_groups: Some((1..=65).collect()),
            ..ChannelSettings::default()
        },
    )
    .err()
    .unwrap();
    assert!(
        is_error(
            &error,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
        ),
        "{error:?}"
    );
}
