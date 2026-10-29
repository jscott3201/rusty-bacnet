//! A local write's array index is checked as WriteProperty checks it, before
//! the object sees the value (#1426): `write_local`, a Command's writes and a
//! Channel's member writes. An index on a property that isn't an array is
//! PROPERTY_IS_NOT_AN_ARRAY, one on a property the object doesn't hold
//! UNKNOWN_PROPERTY, and one past the end of a real array the object's own
//! INVALID_ARRAY_INDEX. A valid index still writes its element.
//!
//! AI-1's Description and BV-1's Active_Text are single values that the
//! objects would take whole whatever the index. MSV-1 has three states.
//! CMD-1's one list writes AI-1's Description[1], MSV-1's State_Text[2] and
//! State_Text[9]. CH-1's members are AI-1's Description[1] and MSV-1's
//! State_Text[3]. The clock is paused.
use super::super::channel_wire_tests::{ch, channel, settled};
use super::super::command_action_wire_tests::{cmd, idle, read_db};
use super::super::cov_wire_test_support::*;
use super::super::*;
use bacnet_encoding::constructed::decode_action_list;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::command::CommandObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetDeviceObjectPropertyReference,
};
use bacnet_types::enums::{EngineeringUnits, ObjectType, Reliability, WriteStatus};
use bacnet_types::error::RemoteError;

use PropertyIdentifier as P;

fn ai() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn bv() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap()
}

fn msv() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap()
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::CharacterString(value.into())
}

/// A write of `value` to `object`'s `property` at `index`.
fn indexed(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: u32,
    value: &str,
) -> BACnetActionCommand {
    BACnetActionCommand {
        device_identifier: None,
        object_identifier: object,
        property_identifier: property,
        property_array_index: Some(index),
        property_value: text(value),
        priority: None,
        post_delay: None,
        quit_on_failure: false,
        write_successful: true,
    }
}

fn member(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: u32,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        property_array_index: Some(index),
        ..BACnetDeviceObjectPropertyReference::new_local(object, property.to_raw())
    }
}

fn objects(db: &mut ObjectDatabase) {
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    db.add(Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()))
        .unwrap();
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![
                indexed(ai(), P::DESCRIPTION, 1, "command"),
                indexed(msv(), P::STATE_TEXT, 2, "command"),
                indexed(msv(), P::STATE_TEXT, 9, "command"),
            ],
        }])
        .unwrap();
    db.add(Box::new(command)).unwrap();
    db.add(Box::new(channel(
        1,
        1,
        vec![
            (member(ai(), P::DESCRIPTION, 1), 0),
            (member(msv(), P::STATE_TEXT, 3), 0),
        ],
    )))
    .unwrap();
}

async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), objects).await
}

/// The values a refused write must leave alone.
async fn snapshot(h: &Harness) -> Vec<PropertyValue> {
    let mut values = Vec::new();
    for (object, property) in [
        (ai(), P::DESCRIPTION),
        (ai(), P::OBJECT_NAME),
        (bv(), P::ACTIVE_TEXT),
        (bv(), P::PRESENT_VALUE),
        (msv(), P::STATE_TEXT),
    ] {
        values.push(read_db(h, object, property, None).await);
    }
    values
}

#[tokio::test(start_paused = true)]
async fn write_local_checks_an_array_index_before_the_object_sees_the_value() {
    let h = start().await;
    let before = snapshot(&h).await;
    let missing = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 9).unwrap();
    for (object, property, index, value, class, code) in [
        (
            ai(),
            P::DESCRIPTION,
            Some(1),
            text("indexed"),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            bv(),
            P::ACTIVE_TEXT,
            Some(1),
            text("indexed"),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            bv(),
            P::PRESENT_VALUE,
            Some(1),
            PropertyValue::Enumerated(1),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ),
        (
            ai(),
            P::STATE_TEXT,
            Some(1),
            text("indexed"),
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ),
        (
            msv(),
            P::STATE_TEXT,
            Some(9),
            text("indexed"),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
        ),
        // The checks made beside the index keep their answers.
        (
            ai(),
            P::OBJECT_NAME,
            None,
            text("BV-1"),
            ErrorClass::PROPERTY,
            ErrorCode::DUPLICATE_NAME,
        ),
        (
            missing,
            P::DESCRIPTION,
            None,
            text("indexed"),
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ),
    ] {
        let error = h
            .server
            .write_local(
                &object,
                property,
                index,
                value,
                None,
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class: c, code: e })
                if c == class && e == code),
            "{object} {property:?} {index:?}: {error:?}"
        );
    }
    assert_eq!(snapshot(&h).await, before);

    // A valid index writes that element alone.
    h.server
        .write_local(
            &msv(),
            P::STATE_TEXT,
            Some(2),
            text("Two"),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    let states = [
        read_db(&h, msv(), P::STATE_TEXT, Some(1)).await,
        read_db(&h, msv(), P::STATE_TEXT, Some(2)).await,
        read_db(&h, msv(), P::STATE_TEXT, Some(3)).await,
    ];
    let PropertyValue::List(old) = &before[4] else {
        panic!("State_Text read {:?}", before[4]);
    };
    assert_eq!(states, [old[0].clone(), text("Two"), old[2].clone()]);
}

/// Whether each command of CMD-1's first list succeeded.
async fn flags(h: &Harness) -> Vec<bool> {
    let PropertyValue::ApplicationData(element) = read_db(h, cmd(1), P::ACTION, Some(1)).await
    else {
        panic!("Action[1] is one encoded list");
    };
    let (list, _) = decode_action_list(&element, 0).unwrap();
    list.commands
        .iter()
        .map(|command| command.write_successful)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_command_write_whose_index_the_property_refuses_fails() {
    let h = start().await;
    let description = read_db(&h, ai(), P::DESCRIPTION, None).await;
    h.server
        .write_local(
            &cmd(1),
            P::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(1),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    idle(&h, 1).await;
    // Description[1] refused, State_Text[2] written, State_Text[9] past the
    // end of the array.
    assert_eq!(flags(&h).await, [false, true, false]);
    assert_eq!(
        read_db(&h, cmd(1), P::ALL_WRITES_SUCCESSFUL, None).await,
        PropertyValue::Boolean(false)
    );
    assert_eq!(read_db(&h, ai(), P::DESCRIPTION, None).await, description);
    assert_eq!(
        read_db(&h, msv(), P::STATE_TEXT, Some(2)).await,
        text("command")
    );
}

#[tokio::test(start_paused = true)]
async fn a_channel_member_whose_index_the_property_refuses_is_a_configuration_failure() {
    let mut h = start().await;
    let description = read_db(&h, ai(), P::DESCRIPTION, None).await;
    h.server
        .write_local(
            &ch(1),
            P::PRESENT_VALUE,
            None,
            text("channel"),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(settled(&mut h, 1).await, WriteStatus::FAILED);
    assert_eq!(
        read_db(&h, ch(1), P::RELIABILITY, None).await,
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    assert_eq!(read_db(&h, ai(), P::DESCRIPTION, None).await, description);
    // The member after it is still written.
    assert_eq!(
        read_db(&h, msv(), P::STATE_TEXT, Some(3)).await,
        text("channel")
    );
}
