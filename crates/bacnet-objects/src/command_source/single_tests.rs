//! Value_Source on the noncommandable modes of Analog, Binary and
//! Multi-state Value (#1552): absent until tracked, then the last writer of
//! Present_Value, whom Clause 19.5.1.3 alone lets correct it.

use crate::analog::AnalogValueObject;
use crate::binary::BinaryValueObject;
use crate::command_source::{CommandDeviceBinding, CommandOrigin};
use crate::multistate::MultiStateValueObject;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::{PropertyPresenceCondition, PropertyWriteCapability};
use crate::traits::BACnetObject;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// Device `device` writing from MAC `mac`, its binding known.
fn remote(device: u32, mac: u8) -> CommandOrigin {
    CommandOrigin::Remote {
        actual_address: BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&[mac]),
        },
        binding: CommandDeviceBinding::Unique(oid(ObjectType::DEVICE, device)),
    }
}

fn local(device: u32, initiator: Option<ObjectIdentifier>) -> CommandOrigin {
    CommandOrigin::Local {
        owner_device: oid(ObjectType::DEVICE, device),
        initiating_object: initiator,
    }
}

/// Value_Source naming `object`: `[1]` around a device-object reference
/// holding only its object identifier `[1]`.
fn names(object: ObjectIdentifier) -> PropertyValue {
    let mut bytes = vec![0x1E, 0x1C];
    bytes.extend(object.encode());
    bytes.push(0x1F);
    PropertyValue::ApplicationData(bytes)
}

const NONE: [u8; 1] = [0x08];

fn none() -> PropertyValue {
    PropertyValue::ApplicationData(NONE.to_vec())
}

/// The three Value objects in `access`, tracked or not, each with a valid
/// Present_Value to write that differs from its initial one.
fn values(
    access: PresentValueAccess,
    tracked: bool,
) -> Vec<(Box<dyn BACnetObject>, PropertyValue)> {
    let mut av = AnalogValueObject::with_access(1, "AV-1", 62, access).unwrap();
    let mut bv = BinaryValueObject::with_access(1, "BV-1", access).unwrap();
    let mut msv = MultiStateValueObject::with_access(1, "MSV-1", 3, access).unwrap();
    av.set_value_source_tracking(tracked);
    bv.set_value_source_tracking(tracked);
    msv.set_value_source_tracking(tracked);
    vec![
        (Box::new(av), PropertyValue::Real(5.0)),
        (Box::new(bv), PropertyValue::Enumerated(1)),
        (Box::new(msv), PropertyValue::Unsigned(2)),
    ]
}

fn assert_code(result: Result<impl std::fmt::Debug, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { code, .. }) => {
            assert_eq!(code, expected.to_raw() as u32, "expected {expected:?}")
        }
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

fn write_from(
    object: &mut dyn BACnetObject,
    property: P,
    value: PropertyValue,
    origin: &CommandOrigin,
) -> Result<(), Error> {
    object.write_property_from(property, None, value, None, origin)
}

#[test]
fn untracked_noncommandable_values_have_no_value_source() {
    for access in [PresentValueAccess::Writable, PresentValueAccess::ReadOnly] {
        for (mut object, _) in values(access, false) {
            assert!(!object.property_list().contains(&P::VALUE_SOURCE));
            assert_code(
                object.read_property(P::VALUE_SOURCE, None),
                ErrorCode::UNKNOWN_PROPERTY,
            );
            assert_code(
                write_from(object.as_mut(), P::VALUE_SOURCE, none(), &remote(30, 1)),
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
    }
    // Writes need no writer while untracked, as before.
    for (mut object, value) in values(PresentValueAccess::Writable, false) {
        object
            .write_property(P::PRESENT_VALUE, None, value.clone(), None)
            .unwrap();
        assert_eq!(object.read_property(P::PRESENT_VALUE, None).unwrap(), value);
    }
}

#[test]
fn tracked_noncommandable_values_list_a_required_value_source() {
    for access in [PresentValueAccess::Writable, PresentValueAccess::ReadOnly] {
        let untracked: Vec<_> = values(access, false)
            .into_iter()
            .map(|(object, _)| object.property_list().into_owned())
            .collect();
        for ((object, _), mut expected) in values(access, true).into_iter().zip(untracked) {
            // Where a commandable object lists it: last, before Property_List.
            expected.push(P::VALUE_SOURCE);
            assert_eq!(object.property_list().as_ref(), expected);
            assert!(object.required_properties().contains(&P::VALUE_SOURCE));
            let metadata = object.property_metadata();
            let row = metadata
                .iter()
                .find(|row| row.property_identifier == P::VALUE_SOURCE)
                .unwrap();
            assert_eq!(
                row.presence_condition,
                Some(PropertyPresenceCondition::ValueSourceTracking)
            );
            assert_eq!(
                row.write_capability,
                PropertyWriteCapability::WhenCommandOwner
            );
            for absent in [P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME] {
                assert_code(
                    object.read_property(absent, None),
                    ErrorCode::UNKNOWN_PROPERTY,
                );
            }
            assert_eq!(object.read_property(P::VALUE_SOURCE, None).unwrap(), none());
            assert_code(
                object.read_property(P::VALUE_SOURCE, Some(1)),
                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            );
        }
    }
}

#[test]
fn a_sourced_write_publishes_its_writer() {
    for (mut object, value) in values(PresentValueAccess::Writable, true) {
        write_from(
            object.as_mut(),
            P::PRESENT_VALUE,
            value.clone(),
            &remote(30, 1),
        )
        .unwrap();
        assert_eq!(object.read_property(P::PRESENT_VALUE, None).unwrap(), value);
        assert_eq!(
            object.read_property(P::VALUE_SOURCE, None).unwrap(),
            names(oid(ObjectType::DEVICE, 30))
        );
        // A local write names its initiator, or the Device when it has none.
        let schedule = oid(ObjectType::SCHEDULE, 4);
        write_from(
            object.as_mut(),
            P::PRESENT_VALUE,
            value.clone(),
            &local(9, Some(schedule)),
        )
        .unwrap();
        assert_eq!(
            object.read_property(P::VALUE_SOURCE, None).unwrap(),
            names(schedule)
        );
        write_from(object.as_mut(), P::PRESENT_VALUE, value, &local(9, None)).unwrap();
        assert_eq!(
            object.read_property(P::VALUE_SOURCE, None).unwrap(),
            names(oid(ObjectType::DEVICE, 9))
        );
    }
}

#[test]
fn only_the_last_writer_corrects_the_source() {
    let forwarded = names(oid(ObjectType::DEVICE, 77));
    for (mut object, value) in values(PresentValueAccess::Writable, true) {
        let object = object.as_mut();
        // Nobody has written yet, so nobody may correct.
        assert_code(
            write_from(object, P::VALUE_SOURCE, forwarded.clone(), &remote(30, 1)),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        write_from(object, P::PRESENT_VALUE, value.clone(), &remote(30, 1)).unwrap();
        // Another device is refused, before its value is looked at.
        for claim in [forwarded.clone(), PropertyValue::Real(1.0)] {
            assert_code(
                write_from(object, P::VALUE_SOURCE, claim, &remote(31, 2)),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        assert_eq!(
            object.read_property(P::VALUE_SOURCE, None).unwrap(),
            names(oid(ObjectType::DEVICE, 30))
        );
        // The writer may, with one complete BACnetValueSource.
        for bad in [
            PropertyValue::Real(1.0),
            PropertyValue::ApplicationData(vec![0x08, 0x08]),
            PropertyValue::ApplicationData(vec![0x1E]),
        ] {
            assert_code(
                write_from(object, P::VALUE_SOURCE, bad, &remote(30, 1)),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        assert_code(
            object.write_property_from(
                P::VALUE_SOURCE,
                Some(1),
                forwarded.clone(),
                None,
                &remote(30, 1),
            ),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        );
        write_from(object, P::VALUE_SOURCE, forwarded.clone(), &remote(30, 1)).unwrap();
        assert_eq!(
            object.read_property(P::VALUE_SOURCE, None).unwrap(),
            forwarded
        );
        // A correction keeps the owner, so it may correct again.
        write_from(object, P::VALUE_SOURCE, none(), &remote(30, 1)).unwrap();
        assert_eq!(object.read_property(P::VALUE_SOURCE, None).unwrap(), none());
        // A new write replaces the owner.
        write_from(object, P::PRESENT_VALUE, value.clone(), &remote(31, 2)).unwrap();
        assert_code(
            write_from(object, P::VALUE_SOURCE, forwarded.clone(), &remote(30, 1)),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        // A local owner corrects whatever its initiator; a remote one can't.
        write_from(object, P::PRESENT_VALUE, value, &local(9, None)).unwrap();
        assert_code(
            write_from(object, P::VALUE_SOURCE, forwarded.clone(), &remote(31, 2)),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        let initiator = Some(oid(ObjectType::SCHEDULE, 4));
        write_from(
            object,
            P::VALUE_SOURCE,
            forwarded.clone(),
            &local(9, initiator),
        )
        .unwrap();
        assert_eq!(
            object.read_property(P::VALUE_SOURCE, None).unwrap(),
            forwarded
        );
    }
}

#[test]
fn writes_that_change_nothing_leave_the_source() {
    for (mut object, value) in values(PresentValueAccess::Writable, true) {
        let object = object.as_mut();
        write_from(object, P::PRESENT_VALUE, value.clone(), &remote(30, 1)).unwrap();
        let writer = names(oid(ObjectType::DEVICE, 30));
        // A NULL is a no-op on a noncommandable Present_Value (Clause 19.2).
        write_from(
            object,
            P::PRESENT_VALUE,
            PropertyValue::Null,
            &remote(31, 2),
        )
        .unwrap();
        // A refused value, and an origin that can't be a writer.
        assert_code(
            write_from(
                object,
                P::PRESENT_VALUE,
                PropertyValue::Boolean(true),
                &remote(31, 2),
            ),
            ErrorCode::INVALID_DATA_TYPE,
        );
        let malformed = CommandOrigin::Remote {
            actual_address: BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::new(),
            },
            binding: CommandDeviceBinding::Unknown,
        };
        assert_code(
            write_from(object, P::PRESENT_VALUE, value.clone(), &malformed),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(object.read_property(P::VALUE_SOURCE, None).unwrap(), writer);
        assert_eq!(object.read_property(P::PRESENT_VALUE, None).unwrap(), value);
        // The first writer still owns the source.
        write_from(object, P::VALUE_SOURCE, none(), &remote(30, 1)).unwrap();
    }
    // A read-only Present_Value in service refuses peers, who publish nothing.
    for (mut object, value) in values(PresentValueAccess::ReadOnly, true) {
        assert_code(
            write_from(object.as_mut(), P::PRESENT_VALUE, value, &remote(30, 1)),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(object.read_property(P::VALUE_SOURCE, None).unwrap(), none());
    }
}

#[test]
fn a_tracked_source_refuses_writes_that_name_no_writer() {
    for (mut object, value) in values(PresentValueAccess::Writable, true) {
        let before = object.read_property(P::PRESENT_VALUE, None).unwrap();
        assert_code(
            object.write_property(P::PRESENT_VALUE, None, value, None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_code(
            object.write_property(P::VALUE_SOURCE, None, none(), None),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            before
        );
    }
}

#[test]
fn application_updates_name_the_device_or_nobody() {
    for (mut object, value) in values(PresentValueAccess::ReadOnly, true) {
        let object = object.as_mut();
        object
            .set_present_value_from_internal(value.clone(), &local(9, None))
            .unwrap();
        assert_eq!(
            object.read_property(P::VALUE_SOURCE, None).unwrap(),
            names(oid(ObjectType::DEVICE, 9))
        );
        // The Device owns that source.
        write_from(object, P::VALUE_SOURCE, none(), &local(9, None)).unwrap();
        // Without a Device, no source is known and nobody may correct.
        object
            .set_present_value_from_internal(value.clone(), &local(9, None))
            .unwrap();
        object.set_present_value_internal(value.clone()).unwrap();
        assert_eq!(object.read_property(P::VALUE_SOURCE, None).unwrap(), none());
        assert_code(
            write_from(object, P::VALUE_SOURCE, none(), &local(9, None)),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        // Out of service, the application is refused and names nobody.
        object
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
            .unwrap();
        assert_code(
            object.set_present_value_from_internal(value, &local(9, None)),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(object.read_property(P::VALUE_SOURCE, None).unwrap(), none());
    }
}

#[test]
fn tracking_is_for_noncommandable_values_only_and_can_be_turned_off() {
    let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    let commandable = av.property_metadata().into_owned();
    av.set_value_source_tracking(true);
    assert_eq!(av.property_metadata().as_ref(), commandable);
    assert_code(
        av.write_property(P::PRESENT_VALUE, None, PropertyValue::Real(1.0), None),
        ErrorCode::WRITE_ACCESS_DENIED,
    );

    let mut av =
        AnalogValueObject::with_access(2, "AV-2", 62, PresentValueAccess::Writable).unwrap();
    let untracked = av.property_metadata().into_owned();
    av.set_value_source_tracking(true);
    av.set_value_source_tracking(false);
    assert_eq!(av.property_metadata().as_ref(), untracked);
    av.write_property(P::PRESENT_VALUE, None, PropertyValue::Real(1.0), None)
        .unwrap();
}
