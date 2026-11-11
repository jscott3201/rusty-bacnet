//! Every refusal of the point-list builder names its point and the
//! property at fault (#2445), and nothing is built.
use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

/// The refusal's variant, by name: `Refusal` holds an `Error`, which has no
/// `PartialEq`.
fn variant(reason: &Refusal) -> &'static str {
    match reason {
        Refusal::Object(_) => "Object",
        Refusal::DuplicateIdentifier => "DuplicateIdentifier",
        Refusal::DuplicateName => "DuplicateName",
        Refusal::EmptyName => "EmptyName",
        Refusal::TypeNotCovered => "TypeNotCovered",
        Refusal::KindMismatch => "KindMismatch",
        Refusal::AccessNotAllowed => "AccessNotAllowed",
        Refusal::NotFinite => "NotFinite",
        Refusal::MinAboveMax => "MinAboveMax",
        Refusal::OutOfRange => "OutOfRange",
        Refusal::StateTextCount => "StateTextCount",
        Refusal::DeviceMismatch => "DeviceMismatch",
    }
}

fn is_value_out_of_range(reason: &Refusal) -> bool {
    matches!(
        reason,
        Refusal::Object(Error::Remote(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::VALUE_OUT_OF_RANGE,
            ..
        }))
    )
}

/// A good point that comes first, so each refusal is of the later one.
fn first() -> Point {
    Point::new(
        oid(ObjectType::ANALOG_INPUT, 1),
        "first",
        Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly),
    )
}

/// The refusal `result` holds. Neither a database nor an object has
/// `Debug`, so the `Result` helpers that print the success don't apply.
fn refused<T>(result: Result<T, BuildError>) -> BuildError {
    match result {
        Ok(_) => panic!("built what should be refused"),
        Err(error) => error,
    }
}

fn refuse(point: Point) -> BuildError {
    let list = PointList::new(DeviceInfo::new(1001), vec![first(), point]);
    refused(build_database(list, &identity()))
}

fn ai(instance: u32, name: &str, analog: Analog) -> Point {
    Point::new(oid(ObjectType::ANALOG_INPUT, instance), name, analog)
}

fn commandable<T>(relinquish_default: Option<T>) -> Access<T> {
    Access::Commandable { relinquish_default }
}

/// Each case: the point, the property named, and the refusal.
#[test]
fn every_refusal_names_its_point_and_property() {
    let units = EngineeringUnits::NO_UNITS;
    let analog = |access| Analog::new(units, access);
    let multi = |count, access| MultiState::new(count, access);
    let integer = |access| Integer::new(units, access);
    let cases: Vec<(Point, Option<P>, &str)> = vec![
        // Identifiers and names, against an earlier point, the Device and
        // the declared port.
        (
            ai(1, "second", analog(Access::ReadOnly)),
            Some(P::OBJECT_IDENTIFIER),
            "DuplicateIdentifier",
        ),
        (
            Point::new(
                oid(ObjectType::DEVICE, 1001),
                "dev",
                analog(Access::ReadOnly),
            ),
            Some(P::OBJECT_IDENTIFIER),
            "DuplicateIdentifier",
        ),
        (
            Point::new(
                oid(ObjectType::NETWORK_PORT, 1),
                "np",
                analog(Access::ReadOnly),
            ),
            Some(P::OBJECT_IDENTIFIER),
            "DuplicateIdentifier",
        ),
        (
            ai(2, "first", analog(Access::ReadOnly)),
            Some(P::OBJECT_NAME),
            "DuplicateName",
        ),
        (
            ai(2, "device-1001", analog(Access::ReadOnly)),
            Some(P::OBJECT_NAME),
            "DuplicateName",
        ),
        (
            ai(2, "port-1", analog(Access::ReadOnly)),
            Some(P::OBJECT_NAME),
            "DuplicateName",
        ),
        (
            ai(2, "", analog(Access::ReadOnly)),
            Some(P::OBJECT_NAME),
            "EmptyName",
        ),
        // The reserved instance, which `ObjectIdentifier::new` still makes.
        (
            ai(
                ObjectIdentifier::MAX_INSTANCE,
                "wildcard",
                analog(Access::ReadOnly),
            ),
            Some(P::OBJECT_IDENTIFIER),
            "Object",
        ),
        // Type and kind.
        (
            Point::new(
                oid(ObjectType::POSITIVE_INTEGER_VALUE, 1),
                "unsigned",
                analog(Access::Writable),
            ),
            None,
            "TypeNotCovered",
        ),
        (
            Point::new(
                oid(ObjectType::INTEGER_VALUE, 2),
                "kind",
                analog(Access::Writable),
            ),
            None,
            "KindMismatch",
        ),
        (
            Point::new(
                oid(ObjectType::ANALOG_VALUE, 2),
                "kind",
                integer(Access::Writable),
            ),
            None,
            "KindMismatch",
        ),
        (
            Point::new(
                oid(ObjectType::ANALOG_INPUT, 2),
                "kind",
                Binary::new(Access::ReadOnly),
            ),
            None,
            "KindMismatch",
        ),
        (
            Point::new(
                oid(ObjectType::MULTI_STATE_VALUE, 2),
                "kind",
                analog(Access::Writable),
            ),
            None,
            "KindMismatch",
        ),
        // Access: an Input read-only, an Output commandable.
        (
            ai(2, "w", analog(Access::Writable)),
            Some(P::PRESENT_VALUE),
            "AccessNotAllowed",
        ),
        (
            ai(2, "c", analog(commandable(None))),
            Some(P::PRESENT_VALUE),
            "AccessNotAllowed",
        ),
        (
            Point::new(
                oid(ObjectType::ANALOG_OUTPUT, 2),
                "r",
                analog(Access::ReadOnly),
            ),
            Some(P::PRESENT_VALUE),
            "AccessNotAllowed",
        ),
        (
            Point::new(
                oid(ObjectType::BINARY_OUTPUT, 2),
                "w",
                Binary::new(Access::Writable),
            ),
            Some(P::PRESENT_VALUE),
            "AccessNotAllowed",
        ),
        (
            Point::new(
                oid(ObjectType::BINARY_INPUT, 2),
                "c",
                Binary::new(commandable(None)),
            ),
            Some(P::PRESENT_VALUE),
            "AccessNotAllowed",
        ),
        (
            Point::new(
                oid(ObjectType::MULTI_STATE_OUTPUT, 2),
                "r",
                multi(2, Access::ReadOnly),
            ),
            Some(P::PRESENT_VALUE),
            "AccessNotAllowed",
        ),
        // An Integer kind fits no Input or Output, whatever its access.
        (
            Point::new(
                oid(ObjectType::ANALOG_INPUT, 2),
                "integer",
                integer(Access::ReadOnly),
            ),
            None,
            "KindMismatch",
        ),
        // Reals.
        (
            ai(
                2,
                "nan",
                analog(Access::ReadOnly).with_cov_increment(f32::NAN),
            ),
            Some(P::COV_INCREMENT),
            "NotFinite",
        ),
        (
            ai(
                2,
                "inf",
                analog(Access::ReadOnly).with_min_pres_value(f32::INFINITY),
            ),
            Some(P::MIN_PRES_VALUE),
            "NotFinite",
        ),
        (
            ai(
                2,
                "-inf",
                analog(Access::ReadOnly).with_max_pres_value(f32::NEG_INFINITY),
            ),
            Some(P::MAX_PRES_VALUE),
            "NotFinite",
        ),
        (
            Point::new(
                oid(ObjectType::ANALOG_VALUE, 2),
                "rd",
                analog(commandable(Some(f32::NAN))),
            ),
            Some(P::RELINQUISH_DEFAULT),
            "NotFinite",
        ),
        (
            ai(
                2,
                "range",
                analog(Access::ReadOnly)
                    .with_min_pres_value(10.0)
                    .with_max_pres_value(0.0),
            ),
            Some(P::MIN_PRES_VALUE),
            "MinAboveMax",
        ),
        (
            Point::new(
                oid(ObjectType::INTEGER_VALUE, 2),
                "range",
                integer(commandable(Some(0)))
                    .with_min_pres_value(1)
                    .with_max_pres_value(0),
            ),
            Some(P::MIN_PRES_VALUE),
            "MinAboveMax",
        ),
        // Ranges.
        (
            ai(
                2,
                "units",
                Analog::new(EngineeringUnits::from_raw(65_536), Access::ReadOnly),
            ),
            Some(P::UNITS),
            "OutOfRange",
        ),
        (
            Point::new(
                oid(ObjectType::INTEGER_VALUE, 2),
                "units",
                Integer::new(EngineeringUnits::from_raw(65_536), Access::Writable),
            ),
            Some(P::UNITS),
            "OutOfRange",
        ),
        (
            Point::new(
                oid(ObjectType::MULTI_STATE_INPUT, 2),
                "none",
                multi(0, Access::ReadOnly),
            ),
            Some(P::NUMBER_OF_STATES),
            "OutOfRange",
        ),
        (
            Point::new(
                oid(ObjectType::MULTI_STATE_INPUT, 2),
                "many",
                multi(MAX_NUMBER_OF_STATES + 1, Access::ReadOnly),
            ),
            Some(P::NUMBER_OF_STATES),
            "OutOfRange",
        ),
        (
            Point::new(
                oid(ObjectType::MULTI_STATE_INPUT, 2),
                "labels",
                multi(3, Access::ReadOnly).with_state_texts(texts(&["One", "Two"])),
            ),
            Some(P::STATE_TEXT),
            "StateTextCount",
        ),
    ];
    for (point, property, expected) in cases {
        let object = point.object;
        let name = point.name.clone();
        let error = refuse(point);
        assert_eq!(error.object, object, "{name}: {error}");
        assert_eq!(error.property, property, "{name}: {error}");
        assert_eq!(variant(&error.reason), expected, "{name}: {error}");
    }
}

/// What the objects' own setters refuse arrives as `Refusal::Object` with
/// the error a write would get, naming the property.
#[test]
fn a_setter_refusal_names_its_property() {
    let cases = [
        (
            ai(
                2,
                "cov",
                Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly).with_cov_increment(-1.0),
            ),
            P::COV_INCREMENT,
        ),
        (
            Point::new(
                oid(ObjectType::BINARY_OUTPUT, 2),
                "bo",
                Binary::new(commandable(Some(BinaryPV::from_raw(2)))),
            ),
            P::RELINQUISH_DEFAULT,
        ),
        (
            Point::new(
                oid(ObjectType::MULTI_STATE_OUTPUT, 2),
                "mso",
                MultiState::new(3, commandable(Some(4))),
            ),
            P::RELINQUISH_DEFAULT,
        ),
        (
            Point::new(
                oid(ObjectType::MULTI_STATE_VALUE, 2),
                "msv",
                MultiState::new(3, commandable(Some(0))),
            ),
            P::RELINQUISH_DEFAULT,
        ),
    ];
    for (point, property) in cases {
        let object = point.object;
        let error = refuse(point);
        assert_eq!((error.object, error.property), (object, Some(property)));
        assert!(is_value_out_of_range(&error.reason), "{error}");
    }
}

/// The Device's part: its instance must be the identity's, and a name it
/// gives (or the identity's) must not be empty.
#[test]
fn the_devices_part_is_checked() {
    let identity = identity();
    let device = identity.device_oid();
    let mismatched = PointList::new(DeviceInfo::new(1002), vec![first()]);
    let error = refused(build_database(mismatched, &identity));
    assert_eq!(
        (error.object, error.property, variant(&error.reason)),
        (device, Some(P::OBJECT_IDENTIFIER), "DeviceMismatch")
    );
    let unnamed = PointList::new(DeviceInfo::new(1001).with_name(""), vec![first()]);
    let error = refused(build_database(unnamed, &identity));
    assert_eq!(
        (error.object, error.property, variant(&error.reason)),
        (device, Some(P::OBJECT_NAME), "EmptyName")
    );
    let nameless = identity.clone().with_name("");
    let error = refused(build_database(
        PointList::new(DeviceInfo::new(1001), vec![]),
        &nameless,
    ));
    assert_eq!(variant(&error.reason), "EmptyName");
}

/// A Device named as a declared port's object is named is refused in the
/// uniqueness pass, naming the Device and Object_Name, not later by the
/// database without a property.
#[test]
fn the_device_may_not_take_a_ports_name() {
    let identity = identity();
    let list = PointList::new(DeviceInfo::new(1001).with_name("port-1"), vec![first()]);
    let error = refused(build_database(list, &identity));
    assert_eq!(
        (error.object, error.property, variant(&error.reason)),
        (identity.device_oid(), Some(P::OBJECT_NAME), "DuplicateName")
    );
    let renamed = identity.with_name("port-1");
    let error = refused(build_database(
        PointList::new(DeviceInfo::new(1001), vec![]),
        &renamed,
    ));
    assert_eq!(
        (error.property, variant(&error.reason)),
        (Some(P::OBJECT_NAME), "DuplicateName")
    );
}

/// A point that takes a name the list gives the Device is refused too.
#[test]
fn a_point_may_not_take_the_lists_device_name() {
    let list = PointList::new(
        DeviceInfo::new(1001).with_name("AHU-1"),
        vec![ai(
            2,
            "AHU-1",
            Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly),
        )],
    );
    let error = refused(build_database(list, &identity()));
    assert_eq!(variant(&error.reason), "DuplicateName");
    assert_eq!(error.object, oid(ObjectType::ANALOG_INPUT, 2));
}

/// The refusal shows its object, its property and its reason, and only
/// when displayed.
#[test]
fn a_refusal_displays_object_property_and_reason() {
    let error = refuse(ai(
        3,
        "first",
        Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly),
    ));
    assert_eq!(
        error.to_string(),
        "ANALOG_INPUT,3 OBJECT_NAME: name already in use"
    );
    let error = refuse(Point::new(
        oid(ObjectType::LARGE_ANALOG_VALUE, 1),
        "double",
        Binary::new(Access::ReadOnly),
    ));
    assert_eq!(
        error.to_string(),
        "LARGE_ANALOG_VALUE,1: object type not covered by point lists"
    );
}

/// `build_object` checks the point alone: an empty name is refused, and a
/// kind that doesn't fit, but not a name another object has.
#[test]
fn build_object_checks_the_point_alone() {
    let error = refused(build_object(ai(
        5,
        "",
        Analog::new(EngineeringUnits::NO_UNITS, Access::ReadOnly),
    )));
    assert_eq!(
        (error.property, variant(&error.reason)),
        (Some(P::OBJECT_NAME), "EmptyName")
    );
    assert!(build_object(first()).is_ok());
    assert!(build_object(first()).is_ok());
}
