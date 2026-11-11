//! The point-list builder (#2445): one point of each type built, then read
//! back through the objects' own `read_property` and compared with the
//! list; the defaults an absent field leaves; names; and purity. The
//! refusals are in `refusal_tests`, the new setters in `setter_tests`.
use super::*;
use crate::property_metadata::PropertyWriteCapability;
use bacnet_types::enums::{BinaryPV, EngineeringUnits};
use core::net::Ipv4Addr;

mod refusal_tests;
mod setter_tests;

pub(super) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

pub(super) fn identity() -> DeviceIdentity {
    DeviceIdentity::new(1001, 42)
        .unwrap()
        .with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 47808)
        .unwrap()
}

fn texts(labels: &[&str]) -> Vec<String> {
    labels.iter().map(|label| String::from(*label)).collect()
}

/// One point of each of the ten types, every optional field given, and
/// every relinquish default other than the object's own, so each one set
/// shows.
fn full_list() -> PointList {
    let celsius = EngineeringUnits::DEGREES_CELSIUS;
    let analog = |access| {
        Analog::new(celsius, access)
            .with_cov_increment(0.25)
            .with_min_pres_value(-40.0)
            .with_max_pres_value(120.0)
    };
    let binary = |access| {
        Binary::new(access)
            .with_active_text("Running")
            .with_inactive_text("Stopped")
    };
    let multi_state =
        |access| MultiState::new(3, access).with_state_texts(texts(&["Off", "Low", "High"]));
    let points = vec![
        Point::new(
            oid(ObjectType::ANALOG_INPUT, 1),
            "ai",
            analog(Access::ReadOnly),
        ),
        Point::new(
            oid(ObjectType::ANALOG_OUTPUT, 1),
            "ao",
            analog(Access::Commandable {
                relinquish_default: Some(12.5),
            }),
        ),
        Point::new(
            oid(ObjectType::ANALOG_VALUE, 1),
            "av",
            analog(Access::Commandable {
                relinquish_default: Some(21.5),
            }),
        ),
        Point::new(
            oid(ObjectType::BINARY_INPUT, 1),
            "bi",
            binary(Access::ReadOnly),
        ),
        Point::new(
            oid(ObjectType::BINARY_OUTPUT, 1),
            "bo",
            binary(Access::Commandable {
                relinquish_default: Some(BinaryPV::ACTIVE),
            }),
        ),
        Point::new(
            oid(ObjectType::BINARY_VALUE, 1),
            "bv",
            binary(Access::Commandable {
                relinquish_default: Some(BinaryPV::ACTIVE),
            }),
        ),
        Point::new(
            oid(ObjectType::MULTI_STATE_INPUT, 1),
            "msi",
            multi_state(Access::ReadOnly),
        ),
        Point::new(
            oid(ObjectType::MULTI_STATE_OUTPUT, 1),
            "mso",
            multi_state(Access::Commandable {
                relinquish_default: Some(2),
            }),
        ),
        Point::new(
            oid(ObjectType::MULTI_STATE_VALUE, 1),
            "msv",
            multi_state(Access::Commandable {
                relinquish_default: Some(3),
            }),
        ),
        Point::new(
            oid(ObjectType::INTEGER_VALUE, 1),
            "iv",
            Integer::new(
                EngineeringUnits::KILOWATT_HOURS,
                Access::Commandable {
                    relinquish_default: Some(-12),
                },
            )
            .with_cov_increment(5)
            .with_min_pres_value(-1_000)
            .with_max_pres_value(i32::MAX),
        ),
    ];
    let points = points
        .into_iter()
        .map(|point| {
            let description = format!("{} point", point.name);
            point.with_description(description)
        })
        .collect();
    PointList::new(
        DeviceInfo::new(1001)
            .with_name("AHU-1")
            .with_description("Air handler"),
        points,
    )
}

/// The access an object serves: commandable when it has a Priority_Array,
/// writable when its metadata takes a Present_Value write at any time.
fn served_access<T>(object: &dyn BACnetObject, relinquish_default: Option<T>) -> Access<T> {
    if object.read_property(P::PRIORITY_ARRAY, None).is_ok() {
        return Access::Commandable { relinquish_default };
    }
    let writable = object.property_metadata().iter().any(|row| {
        row.property_identifier == P::PRESENT_VALUE
            && row.write_capability == PropertyWriteCapability::Always
    });
    if writable {
        Access::Writable
    } else {
        Access::ReadOnly
    }
}

/// The point `object` is, read back through its own `read_property`: an
/// absent optional property reads as `None`.
pub(super) fn read_back(database: &ObjectDatabase, object: ObjectIdentifier) -> Point {
    let served = database.get(&object).expect("the object was built");
    let read = |property| served.read_property(property, None).ok();
    let text = |property| match read(property) {
        Some(PropertyValue::CharacterString(text)) => Some(text),
        _ => None,
    };
    let real = |property| match read(property) {
        Some(PropertyValue::Real(value)) => Some(value),
        _ => None,
    };
    let unsigned = |property| match read(property) {
        Some(PropertyValue::Unsigned(value)) => Some(value as u32),
        _ => None,
    };
    let signed = |property| match read(property) {
        Some(PropertyValue::Signed(value)) => Some(value),
        _ => None,
    };
    let units = || match read(P::UNITS) {
        Some(PropertyValue::Enumerated(units)) => EngineeringUnits::from_raw(units),
        _ => panic!("{object} serves no Units"),
    };
    let kind = match family_of(object.object_type()).expect("a covered type").0 {
        Family::Analog => {
            let access = served_access(served, real(P::RELINQUISH_DEFAULT));
            let mut analog = Analog::new(units(), access);
            analog.cov_increment = real(P::COV_INCREMENT);
            analog.min_pres_value = real(P::MIN_PRES_VALUE);
            analog.max_pres_value = real(P::MAX_PRES_VALUE);
            PointKind::Analog(analog)
        }
        Family::Binary => {
            let default = match read(P::RELINQUISH_DEFAULT) {
                Some(PropertyValue::Enumerated(raw)) => Some(BinaryPV::from_raw(raw)),
                _ => None,
            };
            let mut binary = Binary::new(served_access(served, default));
            binary.active_text = text(P::ACTIVE_TEXT);
            binary.inactive_text = text(P::INACTIVE_TEXT);
            PointKind::Binary(binary)
        }
        Family::MultiState => {
            let count = unsigned(P::NUMBER_OF_STATES).expect("Number_Of_States");
            let access = served_access(served, unsigned(P::RELINQUISH_DEFAULT));
            let Some(PropertyValue::List(labels)) = read(P::STATE_TEXT) else {
                panic!("{object} serves no State_Text");
            };
            let labels: Vec<String> = labels
                .into_iter()
                .map(|label| match label {
                    PropertyValue::CharacterString(label) => label,
                    other => panic!("{object} has a label {other:?}"),
                })
                .collect();
            PointKind::MultiState(MultiState::new(count, access).with_state_texts(labels))
        }
        Family::Integer => {
            let access = served_access(served, signed(P::RELINQUISH_DEFAULT));
            let mut integer = Integer::new(units(), access);
            integer.cov_increment = match read(P::COV_INCREMENT) {
                Some(PropertyValue::Unsigned(increment)) => Some(increment),
                _ => None,
            };
            integer.min_pres_value = signed(P::MIN_PRES_VALUE);
            integer.max_pres_value = signed(P::MAX_PRES_VALUE);
            PointKind::Integer(integer)
        }
    };
    Point {
        object,
        name: served.object_name().into(),
        description: text(P::DESCRIPTION),
        kind,
    }
}

fn object_list(database: &ObjectDatabase, device: ObjectIdentifier) -> Vec<ObjectIdentifier> {
    let Ok(PropertyValue::List(entries)) = database
        .get(&device)
        .expect("the Device")
        .read_property(P::OBJECT_LIST, None)
    else {
        panic!("the Device serves no Object_List");
    };
    entries
        .into_iter()
        .map(|entry| match entry {
            PropertyValue::ObjectIdentifier(oid) => oid,
            other => panic!("Object_List holds {other:?}"),
        })
        .collect()
}

/// Every field of every point reads back as the list gave it.
#[test]
fn a_full_list_reads_back_as_given() {
    let list = full_list();
    let database = build_database(list.clone(), &identity()).unwrap();
    for point in &list.points {
        assert_eq!(&read_back(&database, point.object), point);
    }
    assert_eq!(database.len(), 2 + list.points.len());
}

/// The Device takes the list's name and Description; its Object_List is
/// the Device, the port, then the points in list order.
#[test]
fn the_device_lists_itself_its_port_then_the_points_in_order() {
    let list = full_list();
    let identity = identity();
    let device = identity.device_oid();
    let database = build_database(list.clone(), &identity).unwrap();
    let served = database.get(&device).unwrap();
    assert_eq!(served.object_name(), "AHU-1");
    assert_eq!(
        served.read_property(P::DESCRIPTION, None).unwrap(),
        PropertyValue::CharacterString("Air handler".into())
    );
    assert!(database.find_by_name("AHU-1").is_some());
    let mut expected = vec![device, oid(ObjectType::NETWORK_PORT, 1)];
    expected.extend(list.points.iter().map(|point| point.object));
    assert_eq!(object_list(&database, device), expected);
}

/// With no name, the Device keeps the identity's.
#[test]
fn a_device_without_a_name_keeps_the_identitys() {
    let identity = identity().with_name("plant-controller");
    let list = PointList::new(DeviceInfo::new(1001), Vec::new());
    let database = build_database(list, &identity).unwrap();
    let served = database.get(&identity.device_oid()).unwrap();
    assert_eq!(served.object_name(), "plant-controller");
    assert_eq!(
        served.read_property(P::DESCRIPTION, None).unwrap(),
        PropertyValue::CharacterString(String::new())
    );
}

/// Each family builds on its own, one point at a time, as the right type.
#[test]
fn build_object_builds_each_family_as_its_type() {
    for point in full_list().points {
        let object = point.object;
        let built = build_object(point.clone()).unwrap();
        assert_eq!(built.object_identifier(), object);
        assert_eq!(built.object_name(), point.name);
        assert_eq!(
            built.read_property(P::OBJECT_TYPE, None).unwrap(),
            PropertyValue::Enumerated(object.object_type().to_raw())
        );
    }
}

/// A commandable access that keeps the object's own relinquish default.
fn unset_default<T>() -> Access<T> {
    Access::Commandable {
        relinquish_default: None,
    }
}

/// What an absent optional field leaves: the object's own default, which
/// a reader then sees. A change to an object's default fails here.
#[test]
fn an_absent_field_reads_back_as_the_objects_default() {
    let units = EngineeringUnits::PERCENT;
    let list = PointList::new(
        DeviceInfo::new(1001),
        vec![
            Point::new(
                oid(ObjectType::ANALOG_OUTPUT, 7),
                "ao",
                Analog::new(units, unset_default()),
            ),
            Point::new(
                oid(ObjectType::BINARY_VALUE, 7),
                "bv",
                Binary::new(unset_default()),
            ),
            Point::new(
                oid(ObjectType::MULTI_STATE_OUTPUT, 7),
                "mso",
                MultiState::new(2, unset_default()),
            ),
            Point::new(
                oid(ObjectType::INTEGER_VALUE, 7),
                "iv",
                Integer::new(units, unset_default()),
            ),
        ],
    );
    let database = build_database(list, &identity()).unwrap();

    let mut analog = Analog::new(
        units,
        Access::Commandable {
            relinquish_default: Some(0.0),
        },
    );
    analog.cov_increment = Some(0.0);
    let mut binary = Binary::new(Access::Commandable {
        relinquish_default: Some(BinaryPV::INACTIVE),
    });
    binary.active_text = Some("Active".into());
    binary.inactive_text = Some("Inactive".into());
    let multi_state = MultiState::new(
        2,
        Access::Commandable {
            relinquish_default: Some(1),
        },
    )
    .with_state_texts(texts(&["State 1", "State 2"]));
    // No Min_Pres_Value or Max_Pres_Value: an Integer Value starts without.
    let integer = Integer::new(
        units,
        Access::Commandable {
            relinquish_default: Some(0),
        },
    )
    .with_cov_increment(0);
    for (object, name, kind) in [
        (
            oid(ObjectType::ANALOG_OUTPUT, 7),
            "ao",
            PointKind::Analog(analog),
        ),
        (
            oid(ObjectType::BINARY_VALUE, 7),
            "bv",
            PointKind::Binary(binary),
        ),
        (
            oid(ObjectType::MULTI_STATE_OUTPUT, 7),
            "mso",
            PointKind::MultiState(multi_state),
        ),
        (
            oid(ObjectType::INTEGER_VALUE, 7),
            "iv",
            PointKind::Integer(integer),
        ),
    ] {
        let expected = Point {
            object,
            name: name.into(),
            description: Some(String::new()),
            kind,
        };
        assert_eq!(read_back(&database, object), expected);
    }
}

/// Each Value type takes each access, and serves the one it was given.
#[test]
fn a_value_takes_every_access() {
    let units = EngineeringUnits::NO_UNITS;
    let mut points = Vec::new();
    let accesses = [
        (
            Access::ReadOnly,
            Access::ReadOnly,
            Access::ReadOnly,
            Access::ReadOnly,
        ),
        (
            Access::Writable,
            Access::Writable,
            Access::Writable,
            Access::Writable,
        ),
        (
            Access::Commandable {
                relinquish_default: Some(5.0),
            },
            Access::Commandable {
                relinquish_default: Some(BinaryPV::ACTIVE),
            },
            Access::Commandable {
                relinquish_default: Some(2),
            },
            Access::Commandable {
                relinquish_default: Some(-2),
            },
        ),
    ];
    for (instance, (analog, binary, multi_state, integer)) in (1..).zip(accesses) {
        points.push(Point::new(
            oid(ObjectType::ANALOG_VALUE, instance),
            format!("av-{instance}"),
            Analog::new(units, analog),
        ));
        points.push(Point::new(
            oid(ObjectType::BINARY_VALUE, instance),
            format!("bv-{instance}"),
            Binary::new(binary),
        ));
        points.push(Point::new(
            oid(ObjectType::MULTI_STATE_VALUE, instance),
            format!("msv-{instance}"),
            MultiState::new(2, multi_state),
        ));
        points.push(Point::new(
            oid(ObjectType::INTEGER_VALUE, instance),
            format!("iv-{instance}"),
            Integer::new(units, integer),
        ));
    }
    let list = PointList::new(DeviceInfo::new(1001), points.clone());
    let database = build_database(list, &identity()).unwrap();
    for point in points {
        let served = read_back(&database, point.object).kind;
        let same = match (&point.kind, &served) {
            (PointKind::Analog(given), PointKind::Analog(read)) => given.access == read.access,
            (PointKind::Binary(given), PointKind::Binary(read)) => given.access == read.access,
            (PointKind::MultiState(given), PointKind::MultiState(read)) => {
                given.access == read.access
            }
            (PointKind::Integer(given), PointKind::Integer(read)) => given.access == read.access,
            _ => false,
        };
        assert!(
            same,
            "{}: {:?} served as {served:?}",
            point.name, point.kind
        );
    }
}

/// Nothing limits a name's length or characters: a dotted path of about
/// 200 characters is taken, served and found by name as it is.
#[test]
fn a_long_dotted_name_is_taken_as_it_is() {
    let name = "campus-north.building-12.floor-03.mechanical-room-b.air-handler-07.\
                supply-fan-section.discharge-air.temperature-sensor-primary.\
                calibrated-reading.degrees-celsius.trended-at-one-minute.archive-copy-v2";
    assert!(
        (195..=210).contains(&name.len()),
        "{} characters",
        name.len()
    );
    let object = oid(ObjectType::ANALOG_INPUT, 9);
    let point = Point::new(
        object,
        name,
        Analog::new(EngineeringUnits::DEGREES_CELSIUS, Access::ReadOnly),
    );
    let list = PointList::new(DeviceInfo::new(1001), vec![point]);
    let database = build_database(list, &identity()).unwrap();
    let served = database.find_by_name(name).expect("found by its name");
    assert_eq!(served.object_identifier(), object);
    assert_eq!(
        served.read_property(P::OBJECT_NAME, None).unwrap(),
        PropertyValue::CharacterString(name.into())
    );
}

/// The database comes back before anything serves it: an Input's
/// Present_Value and Out_Of_Service are set through it first.
#[test]
fn the_host_sets_runtime_state_before_serving() {
    let object = oid(ObjectType::BINARY_INPUT, 2);
    let point = Point::new(object, "filter-alarm", Binary::new(Access::ReadOnly));
    let list = PointList::new(DeviceInfo::new(1001), vec![point]);
    let mut database = build_database(list, &identity()).unwrap();
    let input = database.get_mut(&object).unwrap();
    input
        .local_input()
        .expect("a Binary Input takes a local Present_Value")
        .apply_local_input(crate::traits::LocalInputValue::present_value(
            PropertyValue::Enumerated(1),
        ))
        .unwrap();
    input
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    assert_eq!(
        input.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        input.read_property(P::OUT_OF_SERVICE, None).unwrap(),
        PropertyValue::Boolean(true)
    );
}

/// The application commands a built Output at a priority with a local
/// origin, the route a host has before a server runs, and relinquishes it
/// with a NULL: Present_Value then falls back to the list's relinquish
/// default.
#[test]
fn the_application_commands_and_relinquishes_a_built_output() {
    let identity = identity();
    let object = oid(ObjectType::ANALOG_OUTPUT, 3);
    let point = Point::new(
        object,
        "damper",
        Analog::new(
            EngineeringUnits::PERCENT,
            Access::Commandable {
                relinquish_default: Some(20.0),
            },
        ),
    );
    let list = PointList::new(DeviceInfo::new(1001), vec![point]);
    let mut database = build_database(list, &identity).unwrap();
    let local = crate::command_source::CommandOrigin::Local {
        owner_device: identity.device_oid(),
        initiating_object: None,
    };
    let output = database.get_mut(&object).unwrap();
    let slot_8 = |output: &dyn BACnetObject| output.read_property(P::PRIORITY_ARRAY, Some(8));
    output
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Real(75.0),
            Some(8),
            &local,
        )
        .unwrap();
    assert_eq!(
        output.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Real(75.0)
    );
    assert_eq!(slot_8(output).unwrap(), PropertyValue::Real(75.0));
    output
        .write_property_from(P::PRESENT_VALUE, None, PropertyValue::Null, Some(8), &local)
        .unwrap();
    assert_eq!(
        output.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Real(20.0)
    );
    assert_eq!(slot_8(output).unwrap(), PropertyValue::Null);
}

/// The builder is a pure function of the list: a second build gives the
/// same Object_List and the same objects.
#[test]
fn building_twice_gives_the_same_database() {
    let identity = identity();
    let first = build_database(full_list(), &identity).unwrap();
    let second = build_database(full_list(), &identity).unwrap();
    let device = identity.device_oid();
    assert_eq!(object_list(&first, device), object_list(&second, device));
    for point in full_list().points {
        assert_eq!(
            read_back(&first, point.object),
            read_back(&second, point.object)
        );
    }
}

/// An Integer Value's edges build and read back: a one-value range, a
/// relinquish default outside the range and at the INTEGER's ends (the
/// builder compares a default with no range), and the largest Unsigned
/// COV_Increment.
#[test]
fn an_integer_values_edges_build() {
    let units = EngineeringUnits::NO_UNITS;
    let points: Vec<Point> = [(1, i32::MIN), (2, i32::MAX)]
        .into_iter()
        .map(|(instance, default)| {
            let integer = Integer::new(
                units,
                Access::Commandable {
                    relinquish_default: Some(default),
                },
            )
            .with_cov_increment(u64::MAX)
            .with_min_pres_value(7)
            .with_max_pres_value(7);
            Point::new(
                oid(ObjectType::INTEGER_VALUE, instance),
                format!("iv-{instance}"),
                integer,
            )
            .with_description("")
        })
        .collect();
    let list = PointList::new(DeviceInfo::new(1001), points.clone());
    let database = build_database(list, &identity()).unwrap();
    for point in &points {
        assert_eq!(&read_back(&database, point.object), point);
    }
    let served = database.get(&points[0].object).unwrap();
    assert_eq!(
        served.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Signed(i32::MIN)
    );
}
