//! Writes of a Channel's three arrays (Clauses 12.53.11, 12.53.12 and
//! 12.53.15).
use super::tests::{
    assert_error, assert_property_error, configured, device, encoded, member, oid, read,
    unsigned_list,
};
use super::*;
use PropertyIdentifier as P;
use bacnet_types::enums::{ErrorClass, ErrorCode};

#[test]
fn channel_member_writes_keep_execution_delay_the_same_size() {
    let mut channel = configured();
    let list = P::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    // A whole write, as the server hands it over: the members' octets back
    // to back. Two members: the delays keep their first two.
    channel
        .write_property(
            list,
            None,
            PropertyValue::ApplicationData(encoded(&[member(8), member(9)])),
            None,
        )
        .unwrap();
    assert_eq!(read(&channel, P::EXECUTION_DELAY), unsigned_list(&[0, 100]));
    // One element.
    channel
        .write_property(
            list,
            Some(2),
            PropertyValue::ApplicationData(encoded(&[member(4)])),
            None,
        )
        .unwrap();
    assert_eq!(
        read(&channel, list),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(encoded(&[member(8)])),
            PropertyValue::ApplicationData(encoded(&[member(4)])),
        ])
    );
    // Index 0 grows both arrays: empty references and zero delays.
    channel
        .write_property(list, Some(0), PropertyValue::Unsigned(4), None)
        .unwrap();
    assert_eq!(
        channel.read_property(list, Some(4)).unwrap(),
        PropertyValue::ApplicationData(encoded(&[arrays::empty_reference()]))
    );
    assert_eq!(
        read(&channel, P::EXECUTION_DELAY),
        unsigned_list(&[0, 100, 0, 0])
    );
    // Index 0 of Execution_Delay grows the members too, and shrinking
    // either shrinks both from the end.
    for size in [5, 2] {
        channel
            .write_property(
                P::EXECUTION_DELAY,
                Some(0),
                PropertyValue::Unsigned(size),
                None,
            )
            .unwrap();
        assert_eq!(
            channel.read_property(list, Some(0)).unwrap(),
            PropertyValue::Unsigned(size)
        );
    }
    // A whole write of delays gives one per member and never resizes; any
    // other length is refused, as set_execution_delay refuses it.
    for delays in [unsigned_list(&[5]), unsigned_list(&[5, 6, 7])] {
        assert_property_error(
            channel.write_property(P::EXECUTION_DELAY, None, delays, None),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(read(&channel, P::EXECUTION_DELAY), unsigned_list(&[0, 100]));
    channel
        .write_property(P::EXECUTION_DELAY, None, unsigned_list(&[5, 6]), None)
        .unwrap();
    channel
        .write_property(
            P::EXECUTION_DELAY,
            Some(2),
            PropertyValue::Unsigned(250),
            None,
        )
        .unwrap();
    assert_eq!(read(&channel, P::EXECUTION_DELAY), unsigned_list(&[5, 250]));
    // A one-element array decodes to its single value.
    channel
        .write_property(list, Some(0), PropertyValue::Unsigned(1), None)
        .unwrap();
    channel
        .write_property(P::EXECUTION_DELAY, None, PropertyValue::Unsigned(9), None)
        .unwrap();
    assert_eq!(read(&channel, P::EXECUTION_DELAY), unsigned_list(&[9]));
    assert_eq!(
        read(&channel, list),
        PropertyValue::List(vec![PropertyValue::ApplicationData(encoded(&[member(8)])),])
    );
    channel
        .write_property(list, Some(0), PropertyValue::Unsigned(0), None)
        .unwrap();
    assert_eq!(
        read(&channel, P::EXECUTION_DELAY),
        PropertyValue::List(vec![])
    );
    // An empty whole write empties both.
    channel.set_members(vec![member(1)]).unwrap();
    channel
        .write_property(list, None, PropertyValue::ApplicationData(vec![]), None)
        .unwrap();
    assert_eq!(
        read(&channel, P::EXECUTION_DELAY),
        PropertyValue::List(vec![])
    );
}

#[test]
fn channel_member_writes_refuse_bad_indexes_and_oversize_and_take_other_devices() {
    let mut channel = configured();
    let list = P::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    let before = read(&channel, list);
    assert_property_error(
        channel.write_property(
            list,
            Some(4),
            PropertyValue::ApplicationData(encoded(&[member(4)])),
            None,
        ),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    assert_property_error(
        channel.write_property(
            list,
            Some(1),
            PropertyValue::ApplicationData(encoded(&[member(4), member(5)])),
            None,
        ),
        ErrorCode::INVALID_DATA_ENCODING,
    );
    // An application tag where a member's [0] object identifier belongs.
    assert_property_error(
        channel.write_property(
            list,
            None,
            PropertyValue::ApplicationData(vec![0x21, 0x01]),
            None,
        ),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_property_error(
        channel.write_property(list, None, PropertyValue::Real(1.0), None),
        ErrorCode::INVALID_DATA_TYPE,
    );
    let too_many = MAX_CHANNEL_MEMBERS as u64 + 1;
    for property in [list, P::EXECUTION_DELAY] {
        assert_error(
            channel.write_property(property, Some(0), PropertyValue::Unsigned(too_many), None),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        );
    }
    assert_error(
        channel.set_members(vec![member(1); MAX_CHANNEL_MEMBERS + 1]),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
    );
    assert_property_error(
        channel.write_property(
            P::EXECUTION_DELAY,
            Some(1),
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            None,
        ),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_property_error(
        channel.set_execution_delay(vec![1, 2]),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(read(&channel, list), before);
    assert_eq!(
        read(&channel, P::EXECUTION_DELAY),
        unsigned_list(&[0, 100, 200])
    );
}

#[test]
fn channel_number_and_control_groups_writes() {
    let mut channel = configured();
    channel
        .write_property(
            P::CHANNEL_NUMBER,
            None,
            PropertyValue::Unsigned(65_535),
            None,
        )
        .unwrap();
    assert_eq!(
        read(&channel, P::CHANNEL_NUMBER),
        PropertyValue::Unsigned(65_535)
    );
    assert_property_error(
        channel.write_property(
            P::CHANNEL_NUMBER,
            None,
            PropertyValue::Unsigned(65_536),
            None,
        ),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_property_error(
        channel.write_property(P::CHANNEL_NUMBER, None, PropertyValue::Signed(3), None),
        ErrorCode::INVALID_DATA_TYPE,
    );

    let groups = P::CONTROL_GROUPS;
    channel
        .write_property(groups, None, unsigned_list(&[27, 27, 0]), None)
        .unwrap();
    channel
        .write_property(
            groups,
            Some(3),
            PropertyValue::Unsigned(u32::MAX.into()),
            None,
        )
        .unwrap();
    channel
        .write_property(groups, Some(0), PropertyValue::Unsigned(4), None)
        .unwrap();
    assert_eq!(
        read(&channel, groups),
        unsigned_list(&[27, 27, u32::MAX.into(), 0])
    );
    channel
        .write_property(groups, None, PropertyValue::Unsigned(5), None)
        .unwrap();
    assert_eq!(read(&channel, groups), unsigned_list(&[5]));
    // At least one entry, at most MAX_CONTROL_GROUPS.
    assert_property_error(
        channel.write_property(groups, Some(0), PropertyValue::Unsigned(0), None),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_property_error(
        channel.set_control_groups(vec![]),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_error(
        channel.write_property(
            groups,
            Some(0),
            PropertyValue::Unsigned(MAX_CONTROL_GROUPS as u64 + 1),
            None,
        ),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
    );
    assert_property_error(
        channel.write_property(groups, Some(2), PropertyValue::Unsigned(1), None),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    assert_property_error(
        channel.write_property(
            groups,
            Some(1),
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            None,
        ),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(read(&channel, groups), unsigned_list(&[5]));
}

#[test]
fn channel_members_refuse_a_non_device_device_identifier() {
    let mut channel = configured();
    let list = P::LIST_OF_OBJECT_PROPERTY_REFERENCES;
    let before = read(&channel, list);
    // Another object type in the device member is no Device (#1285), so it is
    // out of range, even at the empty instance.
    for not_a_device in [
        oid(ObjectType::ANALOG_VALUE, 9),
        oid(ObjectType::CHANNEL, ObjectIdentifier::WILDCARD_INSTANCE),
    ] {
        let bad = BACnetDeviceObjectPropertyReference {
            device_identifier: Some(not_a_device),
            ..member(1)
        };
        for (index, value) in [
            (None, encoded(&[member(1), bad.clone()])),
            (Some(1), encoded(std::slice::from_ref(&bad))),
        ] {
            assert_property_error(
                channel.write_property(list, index, PropertyValue::ApplicationData(value), None),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        assert_property_error(
            channel.set_members(vec![member(2), bad]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(read(&channel, list), before);
    }
    // A Device identifier at the empty instance is still an empty member.
    let empty = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(ObjectIdentifier::WILDCARD_INSTANCE)),
        ..member(1)
    };
    channel.set_members(vec![member(1), empty]).unwrap();

    // A member in another device is taken (Clause 12.53.11, #1264), whole,
    // by index and through the setter.
    let remote = BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(9)),
        ..member(1)
    };
    let whole = encoded(&[member(1), remote.clone()]);
    channel
        .write_property(list, None, PropertyValue::ApplicationData(whole), None)
        .unwrap();
    let one = encoded(std::slice::from_ref(&remote));
    channel
        .write_property(list, Some(1), PropertyValue::ApplicationData(one), None)
        .unwrap();
    assert_eq!(
        read(&channel, list),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(encoded(std::slice::from_ref(&remote))),
            PropertyValue::ApplicationData(encoded(std::slice::from_ref(&remote))),
        ])
    );
    channel.set_members(vec![remote]).unwrap();
}
