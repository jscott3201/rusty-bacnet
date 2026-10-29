//! Channel's three arrays read one element per index through ReadProperty
//! and ReadPropertyMultiple (Table 12-62, Clause 12.1.5.1; #1151).
use super::group::{ExpectedRead, assert_cases};
use super::*;
use PropertyIdentifier as P;
use bacnet_objects::channel::ChannelObject;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;

/// AO-1's Present_Value: [0] object identifier, [1] property.
const MEMBER_1: &[u8] = &[0x0C, 0x00, 0x40, 0x00, 0x01, 0x19, 0x55];
/// Slot 8 of BV-3's Priority_Array: [2] carries the index.
const MEMBER_2: &[u8] = &[0x0C, 0x01, 0x40, 0x00, 0x03, 0x19, 0x57, 0x29, 0x08];
/// The whole list: the elements' octets back to back.
const BOTH_MEMBERS: &[u8] = &[
    0x0C, 0x00, 0x40, 0x00, 0x01, 0x19, 0x55, 0x0C, 0x01, 0x40, 0x00, 0x03, 0x19, 0x57, 0x29, 0x08,
];

fn channel() -> ChannelObject {
    let mut channel = ChannelObject::new(7, "CH-7", 11).unwrap();
    channel
        .set_members(vec![
            BACnetDeviceObjectPropertyReference::new_local(
                ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
                P::PRESENT_VALUE.to_raw(),
            ),
            BACnetDeviceObjectPropertyReference {
                property_array_index: Some(8),
                ..BACnetDeviceObjectPropertyReference::new_local(
                    ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap(),
                    P::PRIORITY_ARRAY.to_raw(),
                )
            },
        ])
        .unwrap();
    channel.set_execution_delay(vec![0, 300]).unwrap();
    channel.set_control_groups(vec![27, 0]).unwrap();
    channel
}

#[test]
fn rpm_channel_arrays_serve_one_element_per_index() {
    let object = channel();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    assert_eq!(BOTH_MEMBERS, [MEMBER_1, MEMBER_2].concat());
    let cases: &[(P, Option<u32>, ExpectedRead)] = &[
        (
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            None,
            Ok(BOTH_MEMBERS),
        ),
        (
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            Some(0),
            Ok(&[0x21, 2]),
        ),
        (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, Some(1), Ok(MEMBER_1)),
        (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, Some(2), Ok(MEMBER_2)),
        (
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            Some(3),
            Err(ErrorCode::INVALID_ARRAY_INDEX),
        ),
        (
            P::EXECUTION_DELAY,
            None,
            Ok(&[0x21, 0x00, 0x22, 0x01, 0x2C]),
        ),
        (P::EXECUTION_DELAY, Some(0), Ok(&[0x21, 2])),
        (P::EXECUTION_DELAY, Some(1), Ok(&[0x21, 0x00])),
        (P::EXECUTION_DELAY, Some(2), Ok(&[0x22, 0x01, 0x2C])),
        (
            P::EXECUTION_DELAY,
            Some(3),
            Err(ErrorCode::INVALID_ARRAY_INDEX),
        ),
        (P::CONTROL_GROUPS, None, Ok(&[0x21, 27, 0x21, 0])),
        (P::CONTROL_GROUPS, Some(0), Ok(&[0x21, 2])),
        (P::CONTROL_GROUPS, Some(1), Ok(&[0x21, 27])),
        (P::CONTROL_GROUPS, Some(2), Ok(&[0x21, 0])),
        (
            P::CONTROL_GROUPS,
            Some(u32::MAX),
            Err(ErrorCode::INVALID_ARRAY_INDEX),
        ),
        // The scalars take no index.
        (P::PRESENT_VALUE, None, Ok(&[0x00])),
        (
            P::PRESENT_VALUE,
            Some(1),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (P::LAST_PRIORITY, None, Ok(&[0x21, 16])),
        (P::WRITE_STATUS, None, Ok(&[0x91, 0])),
        (P::CHANNEL_NUMBER, None, Ok(&[0x21, 11])),
        (
            P::CHANNEL_NUMBER,
            Some(0),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (P::STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
        (P::OUT_OF_SERVICE, None, Ok(&[0x10])),
        (P::RELIABILITY, None, Ok(&[0x91, 0])),
        (P::ALLOW_GROUP_DELAY_INHIBIT, None, Ok(&[0x10])),
        (
            P::ALLOW_GROUP_DELAY_INHIBIT,
            Some(1),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            P::PROPERTY_LIST,
            None,
            Ok(&[
                0x91, 28, 0x91, 85, 0x92, 0x01, 0x71, 0x92, 0x01, 0x72, 0x91, 111, 0x91, 103, 0x91,
                81, 0x91, 54, 0x92, 0x01, 0x70, 0x92, 0x01, 0x6D, 0x92, 0x01, 0x6E, 0x92, 0x01,
                0x6F,
            ]),
        ),
        (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 12])),
    ];
    assert_cases(&db, oid, cases);
}
