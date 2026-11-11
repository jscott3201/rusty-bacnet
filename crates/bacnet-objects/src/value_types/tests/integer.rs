//! The Integer Value's access and presentation range (#2456). The access
//! rules it shares with the analog, binary and multi-state values are run
//! over it in `present_value_access_tests`; these are its own.

use super::super::*;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::{PropertyWriteCapability, has_property};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;

fn code<T: core::fmt::Debug>(result: Result<T, Error>) -> ErrorCode {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            code
        }
        other => panic!("expected a PROPERTY error, got {other:?}"),
    }
}

fn read(object: &IntegerValueObject, property: P) -> Result<PropertyValue, Error> {
    object.read_property(property, None)
}

fn listed(object: &IntegerValueObject, property: P) -> bool {
    let PropertyValue::List(list) = read(object, P::PROPERTY_LIST).unwrap() else {
        panic!("Property_List is a list");
    };
    list.contains(&PropertyValue::Enumerated(property.to_raw()))
}

/// Each range end is absent until set; once set it reads as an INTEGER,
/// is listed, and is read-only over the network. Neither limits a written
/// Present_Value or Relinquish_Default.
#[test]
fn the_range_is_absent_until_set() {
    let mut object = IntegerValueObject::new(1, "IV-1").unwrap();
    for property in [P::MIN_PRES_VALUE, P::MAX_PRES_VALUE] {
        assert_eq!(code(read(&object, property)), ErrorCode::UNKNOWN_PROPERTY);
        assert!(!has_property(&object.property_metadata(), property));
        assert!(!listed(&object, property));
    }
    object.set_min_pres_value(-10);
    assert_eq!(
        read(&object, P::MIN_PRES_VALUE).unwrap(),
        PropertyValue::Signed(-10)
    );
    assert_eq!(
        code(read(&object, P::MAX_PRES_VALUE)),
        ErrorCode::UNKNOWN_PROPERTY,
        "each end on its own"
    );
    object.set_max_pres_value(10);
    assert_eq!(
        read(&object, P::MAX_PRES_VALUE).unwrap(),
        PropertyValue::Signed(10)
    );
    for property in [P::MIN_PRES_VALUE, P::MAX_PRES_VALUE] {
        assert!(listed(&object, property), "{property:?}");
        let row = object
            .property_metadata()
            .iter()
            .find(|row| row.property_identifier == property)
            .copied()
            .unwrap();
        assert_eq!(row.write_capability, PropertyWriteCapability::ReadOnly);
        assert_eq!(
            code(object.write_property(property, None, PropertyValue::Signed(0), None)),
            ErrorCode::WRITE_ACCESS_DENIED
        );
    }
    object
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Signed(99), Some(8))
        .unwrap();
    object.set_relinquish_default(-99).unwrap();
    assert_eq!(
        read(&object, P::PRESENT_VALUE).unwrap(),
        PropertyValue::Signed(99)
    );
}

/// A noncommandable Integer Value has no Relinquish_Default: the setter is
/// refused as a write of it is, and nothing changes. A commandable one
/// takes any INTEGER, and an empty priority array falls back to it.
#[test]
fn set_relinquish_default_needs_a_commandable_object() {
    for access in [PresentValueAccess::ReadOnly, PresentValueAccess::Writable] {
        let mut object = IntegerValueObject::with_access(1, "IV-1", access).unwrap();
        assert_eq!(
            code(object.set_relinquish_default(7)),
            ErrorCode::UNKNOWN_PROPERTY
        );
        assert_eq!(
            read(&object, P::PRESENT_VALUE).unwrap(),
            PropertyValue::Signed(0)
        );
    }
    let mut object = IntegerValueObject::new(1, "IV-1").unwrap();
    for default in [i32::MIN, -1, i32::MAX] {
        object.set_relinquish_default(default).unwrap();
        assert_eq!(
            read(&object, P::RELINQUISH_DEFAULT).unwrap(),
            PropertyValue::Signed(default)
        );
        assert_eq!(
            read(&object, P::PRESENT_VALUE).unwrap(),
            PropertyValue::Signed(default)
        );
    }
    assert_eq!(
        code(object.write_property(P::RELINQUISH_DEFAULT, None, PropertyValue::Real(1.0), None)),
        ErrorCode::INVALID_DATA_TYPE
    );
}

/// `with_access` serves the access it was given: the commandable rows only
/// when commandable, and Present_Value's write capability to match. `new`
/// is commandable.
#[test]
fn with_access_serves_its_access() {
    let rows = [
        P::PRIORITY_ARRAY,
        P::RELINQUISH_DEFAULT,
        P::CURRENT_COMMAND_PRIORITY,
    ];
    for (access, commandable, capability) in [
        (
            PresentValueAccess::ReadOnly,
            false,
            PropertyWriteCapability::WhenOutOfService,
        ),
        (
            PresentValueAccess::Writable,
            false,
            PropertyWriteCapability::Always,
        ),
        (
            PresentValueAccess::Commandable,
            true,
            PropertyWriteCapability::Always,
        ),
    ] {
        let object = IntegerValueObject::with_access(3, "IV-3", access).unwrap();
        for property in rows {
            assert_eq!(
                has_property(&object.property_metadata(), property),
                commandable,
                "{access:?} {property:?}"
            );
            assert_eq!(read(&object, property).is_ok(), commandable);
        }
        let present_value = object
            .property_metadata()
            .iter()
            .find(|row| row.property_identifier == P::PRESENT_VALUE)
            .unwrap()
            .write_capability;
        assert_eq!(present_value, capability, "{access:?}");
    }
    let new = IntegerValueObject::new(3, "IV-3").unwrap();
    assert!(has_property(&new.property_metadata(), P::PRIORITY_ARRAY));
}

/// A writable one takes a write as Present_Value whatever its priority;
/// a commandable one, written with no command origin, still commands the
/// slot named, 16 when none is, as the macro's value types do.
#[test]
fn writes_follow_the_access() {
    let mut writable =
        IntegerValueObject::with_access(1, "IV-1", PresentValueAccess::Writable).unwrap();
    for (value, priority) in [(5, Some(1)), (-6, None), (7, Some(16))] {
        writable
            .write_property(
                P::PRESENT_VALUE,
                None,
                PropertyValue::Signed(value),
                priority,
            )
            .unwrap();
        assert_eq!(
            read(&writable, P::PRESENT_VALUE).unwrap(),
            PropertyValue::Signed(value)
        );
    }
    let mut commandable = IntegerValueObject::new(1, "IV-1").unwrap();
    commandable
        .write_property(P::PRESENT_VALUE, None, PropertyValue::Signed(-3), None)
        .unwrap();
    assert_eq!(
        commandable
            .read_property(P::PRIORITY_ARRAY, Some(16))
            .unwrap(),
        PropertyValue::Signed(-3)
    );
    assert_eq!(
        read(&commandable, P::CURRENT_COMMAND_PRIORITY).unwrap(),
        PropertyValue::Unsigned(16)
    );
    for priority in [0, 17] {
        assert_eq!(
            code(commandable.write_property(
                P::PRESENT_VALUE,
                None,
                PropertyValue::Signed(1),
                Some(priority)
            )),
            ErrorCode::VALUE_OUT_OF_RANGE
        );
    }
}

/// The setters the point-list builder calls: Description, Units and
/// COV_Increment, each read back as set; every Unsigned is an increment,
/// by setter and by write alike.
#[test]
fn the_builders_setters_read_back() {
    let mut object =
        IntegerValueObject::with_access(1, "IV-1", PresentValueAccess::ReadOnly).unwrap();
    object.set_description("Starts today");
    object.set_units(EngineeringUnits::KILOWATT_HOURS).unwrap();
    object.set_cov_increment(u64::MAX).unwrap();
    assert_eq!(
        read(&object, P::DESCRIPTION).unwrap(),
        PropertyValue::CharacterString("Starts today".into())
    );
    assert_eq!(object.units(), EngineeringUnits::KILOWATT_HOURS);
    assert_eq!(
        read(&object, P::COV_INCREMENT).unwrap(),
        PropertyValue::Unsigned(u64::MAX)
    );
    object
        .write_property(P::COV_INCREMENT, None, PropertyValue::Unsigned(3), None)
        .unwrap();
    assert_eq!(
        read(&object, P::COV_INCREMENT).unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        code(object.set_units(EngineeringUnits::from_raw(65_536))),
        ErrorCode::VALUE_OUT_OF_RANGE
    );
    assert_eq!(object.units(), EngineeringUnits::KILOWATT_HOURS);
}

/// The application sets a noncommandable Present_Value through the local
/// input; a commandable one doesn't take it, and the object is unchanged.
#[test]
fn the_local_input_needs_a_noncommandable_object() {
    use crate::traits::LocalInputValue;
    let mut object =
        IntegerValueObject::with_access(1, "IV-1", PresentValueAccess::ReadOnly).unwrap();
    object
        .local_input()
        .unwrap()
        .apply_local_input(LocalInputValue::present_value(PropertyValue::Signed(
            i32::MIN,
        )))
        .unwrap();
    assert_eq!(
        read(&object, P::PRESENT_VALUE).unwrap(),
        PropertyValue::Signed(i32::MIN)
    );
    let mut commandable = IntegerValueObject::new(1, "IV-1").unwrap();
    let refused = commandable
        .local_input()
        .unwrap()
        .apply_local_input(LocalInputValue::present_value(PropertyValue::Signed(4)));
    assert!(
        matches!(
            refused,
            Err(Error::Remote(RemoteError::Error {
                class: ErrorClass::OBJECT,
                code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            }))
        ),
        "{refused:?}"
    );
    assert_eq!(
        read(&commandable, P::PRESENT_VALUE).unwrap(),
        PropertyValue::Signed(0)
    );
}

/// The application's Reliability (#2446), in each access mode: taken in
/// service, where Status_Flags shows FAULT until NO_FAULT_DETECTED clears
/// it, and refused outside BACnetReliability. A client's write of it is
/// refused in service; out of service it is the client's (Clause 12.43.9),
/// and the application's is refused.
#[test]
fn the_application_sets_reliability_in_service() {
    use bacnet_types::enums::Reliability;
    for access in [
        PresentValueAccess::ReadOnly,
        PresentValueAccess::Writable,
        PresentValueAccess::Commandable,
    ] {
        let mut object = IntegerValueObject::with_access(1, "IV-1", access).unwrap();
        let set = |object: &mut IntegerValueObject, value| {
            object.reliability_eval().unwrap().set_reliability(value)
        };
        set(&mut object, Reliability::NO_SENSOR).unwrap();
        assert_eq!(reliability(&object), Reliability::NO_SENSOR, "{access:?}");
        assert_eq!(flags(&object), 0b0100, "{access:?}: FAULT");
        assert_eq!(
            code(set(&mut object, Reliability::from_raw(26))),
            ErrorCode::VALUE_OUT_OF_RANGE
        );
        set(&mut object, Reliability::NO_FAULT_DETECTED).unwrap();
        assert_eq!(flags(&object), 0, "{access:?}: cleared");
        let over_range = enumerated(Reliability::OVER_RANGE);
        assert_eq!(
            code(object.write_property(P::RELIABILITY, None, over_range, None)),
            ErrorCode::WRITE_ACCESS_DENIED,
            "{access:?}: a client's, in service"
        );
        out_of_service(&mut object, true);
        assert_eq!(
            code(set(&mut object, Reliability::NO_SENSOR)),
            ErrorCode::WRITE_ACCESS_DENIED,
            "{access:?}: the application's, out of service"
        );
        assert_eq!(reliability(&object), Reliability::NO_FAULT_DETECTED);
        assert_eq!(flags(&object), 0b0001, "{access:?}: OUT_OF_SERVICE alone");
    }
}

/// Out of service a client writes Reliability to simulate a fault (Clause
/// 12.43.9, #2446): an Enumerated within BACnetReliability is taken and
/// Status_Flags shows FAULT; another datatype or a value outside the
/// enumeration is refused, changing nothing. Back in service the
/// application's Reliability, set aside on the way out, comes back; a
/// second simulation sets aside and restores a fault the same way. The row
/// is writable out of service.
#[test]
fn a_client_simulates_reliability_out_of_service() {
    use bacnet_types::enums::Reliability;
    for access in [
        PresentValueAccess::ReadOnly,
        PresentValueAccess::Writable,
        PresentValueAccess::Commandable,
    ] {
        let mut object = IntegerValueObject::with_access(1, "IV-1", access).unwrap();
        let row = object
            .property_metadata()
            .iter()
            .find(|row| row.property_identifier == P::RELIABILITY)
            .copied()
            .unwrap();
        assert_eq!(
            row.write_capability,
            PropertyWriteCapability::WhenOutOfService
        );
        let write = |object: &mut IntegerValueObject, value| {
            object.write_property(P::RELIABILITY, None, value, None)
        };
        out_of_service(&mut object, true);
        write(&mut object, enumerated(Reliability::OPEN_LOOP)).unwrap();
        assert_eq!(reliability(&object), Reliability::OPEN_LOOP, "{access:?}");
        assert_eq!(flags(&object), 0b0101, "{access:?}: FAULT, OUT_OF_SERVICE");
        for (value, refused) in [
            (PropertyValue::Unsigned(1), ErrorCode::INVALID_DATA_TYPE),
            (PropertyValue::Enumerated(26), ErrorCode::VALUE_OUT_OF_RANGE),
            (
                PropertyValue::Enumerated(65_536),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
        ] {
            assert_eq!(code(write(&mut object, value)), refused, "{access:?}");
        }
        assert_eq!(reliability(&object), Reliability::OPEN_LOOP);
        out_of_service(&mut object, false);
        assert_eq!(reliability(&object), Reliability::NO_FAULT_DETECTED);
        assert_eq!(flags(&object), 0, "{access:?}: restored");
        object
            .reliability_eval()
            .unwrap()
            .set_reliability(Reliability::NO_SENSOR)
            .unwrap();
        out_of_service(&mut object, true);
        write(&mut object, enumerated(Reliability::NO_FAULT_DETECTED)).unwrap();
        assert_eq!(
            flags(&object),
            0b0001,
            "{access:?}: the fault simulated away"
        );
        out_of_service(&mut object, false);
        assert_eq!(reliability(&object), Reliability::NO_SENSOR, "{access:?}");
        assert_eq!(flags(&object), 0b0100, "{access:?}: the fault back");
    }
}

fn enumerated(reliability: bacnet_types::enums::Reliability) -> PropertyValue {
    PropertyValue::Enumerated(reliability.to_raw())
}

fn reliability(object: &IntegerValueObject) -> bacnet_types::enums::Reliability {
    match read(object, P::RELIABILITY).unwrap() {
        PropertyValue::Enumerated(raw) => bacnet_types::enums::Reliability::from_raw(raw),
        other => panic!("Reliability is an Enumerated: {other:?}"),
    }
}

/// Status_Flags' four bits, IN_ALARM first.
fn flags(object: &IntegerValueObject) -> u8 {
    match read(object, P::STATUS_FLAGS).unwrap() {
        PropertyValue::BitString { data, .. } => data[0] >> 4,
        other => panic!("Status_Flags is a BIT STRING: {other:?}"),
    }
}

fn out_of_service(object: &mut IntegerValueObject, oos: bool) {
    let value = PropertyValue::Boolean(oos);
    object
        .write_property(P::OUT_OF_SERVICE, None, value, None)
        .unwrap();
}
