//! The export from an inventory: which records become points, what a
//! record that lacks a required property says, and a live device's
//! inventory as a model and as Turtle. The round trip from a point list
//! is in `round_trip_tests`, over the same lab link.

use bacnet_types::enums::{BinaryPV, EngineeringUnits, ObjectType, Polarity, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;

use super::inventory::exported;
use super::tests::oid;
use super::*;
use crate::format::RawRow;
use crate::inventory::test_lab::{
    Faults, LAB, lab, lab_device, lab_logged, passing, refused, row, services, stop, text,
};

/// `object`'s record from `rows`, as an inventory builds it.
fn record(object: ObjectIdentifier, rows: &[RawRow]) -> PointRecord {
    let at = crate::DeviceAddress::local(&[2]).unwrap();
    crate::inventory::record_of(&at, LAB, object, rows.to_vec())
}

/// `object`'s entry from `rows`.
fn entry_of(object: ObjectIdentifier, rows: &[RawRow]) -> Result<Point, Skipped> {
    exported(&record(object, rows))
}

/// Whether `point` reads as commandable.
fn commandable(point: &Point) -> bool {
    match &point.kind {
        PointKind::Analog(analog) => matches!(analog.access, Access::Commandable { .. }),
        PointKind::Binary(binary) => matches!(binary.access, Access::Commandable { .. }),
        PointKind::MultiState(states) => matches!(states.access, Access::Commandable { .. }),
        PointKind::Integer(integer) => matches!(integer.access, Access::Commandable { .. }),
        _ => false,
    }
}

/// The export's inventory reads the ten exported types only, in batches:
/// the summary, the Object_List and two requests for eleven objects. The
/// trend log isn't read. The device's text comes back exactly, and a value
/// without a Priority_Array is read-only, the writable one included.
#[tokio::test(start_paused = true)]
async fn the_model_derives_from_an_inventory() {
    let (client, server, log) = lab_logged(Faults::default()).await;
    let options = inventory_options(None).unwrap();
    let inventory = lab_device(&client).await.inventory(&options).await.unwrap();
    assert_eq!(services(&log), [14, 12, 14, 14], "{inventory:?}");
    let model = Model::from(&inventory);
    assert_eq!(model.device, DeviceInfo::new(LAB).with_name("Lab \"AHU\""));
    assert_eq!(model.stopped, None);
    let points: Vec<&Point> = model.points().collect();
    let object = |object_type, instance| {
        *points
            .iter()
            .find(|o| o.object == oid(object_type, instance))
            .unwrap_or_else(|| panic!("{object_type} {instance}: {model:?}"))
    };
    assert_eq!(points.len(), 11, "none skipped: {model:?}");
    assert_eq!(model.entries.len(), 11);
    // Each is the record's own point.
    for (point, record) in points.iter().zip(&inventory.points) {
        assert_eq!(record.to_point().as_ref(), Some(*point));
    }
    let ai = object(ObjectType::ANALOG_INPUT, 1);
    assert_eq!(ai.name, "Zone Temp");
    assert_eq!(ai.description.as_deref(), Some("Room 101"));
    let PointKind::Analog(analog) = &ai.kind else {
        panic!("{ai:?}");
    };
    // The export reads none of the optional settings.
    let mut expected = Analog::new(EngineeringUnits::DEGREES_FAHRENHEIT, Access::ReadOnly);
    assert_eq!(analog, &expected);
    assert!(commandable(object(ObjectType::ANALOG_OUTPUT, 1)));
    assert!(commandable(object(ObjectType::ANALOG_VALUE, 1)));
    let total = object(ObjectType::ANALOG_VALUE, 2);
    expected.units = EngineeringUnits::KILOWATT_HOURS;
    assert_eq!(total.kind, PointKind::Analog(expected), "{total:?}");
    assert!(commandable(object(ObjectType::BINARY_VALUE, 1)));
    assert!(!commandable(object(ObjectType::BINARY_INPUT, 1)));
    let msv = object(ObjectType::MULTI_STATE_VALUE, 1);
    let commanded = Access::Commandable {
        relinquish_default: None,
    };
    let three = MultiState::new(3, commanded).with_state_texts(["Off", "Low", "High"]);
    assert_eq!(msv.kind, PointKind::MultiState(three));
    let PointKind::MultiState(msi) = &object(ObjectType::MULTI_STATE_INPUT, 1).kind else {
        panic!("a multi-state input");
    };
    assert_eq!((msi.number_of_states, msi.access), (4, Access::ReadOnly));
    // The integer value, writable, reads as read-only, as the analog does.
    let meter = object(ObjectType::INTEGER_VALUE, 1);
    let integer = Integer::new(EngineeringUnits::KILOWATT_HOURS, Access::ReadOnly);
    assert_eq!(meter.kind, PointKind::Integer(integer), "{meter:?}");

    let turtle = to_turtle(&model, &TurtleOptions::default());
    super::tests::check_structure(&turtle);
    assert!(
        turtle.contains("bacnet:device-name \"Lab \\\"AHU\\\"\" ;"),
        "{turtle}"
    );
    assert!(
        turtle.contains("d:integer-value-1.present-value a s223:QuantifiableProperty ;"),
        "{turtle}"
    );
    assert!(
        turtle.contains("# 11 objects exported, 0 skipped"),
        "{turtle}"
    );

    // An inventory of every object gives the same model and the same
    // Turtle: the objects the export doesn't map are left out.
    let whole = lab_device(&client)
        .await
        .inventory(&InventoryOptions::new())
        .await
        .unwrap();
    assert_eq!(whole.points.len(), 13);
    let from_whole = Model::from(&whole);
    assert_eq!(from_whole.entries, model.entries);
    assert_eq!(
        to_turtle(&from_whole, &TurtleOptions::default()),
        turtle,
        "the Device's Description, which the whole inventory read, isn't in the Turtle"
    );
    assert_eq!(from_whole.device.description.as_deref(), Some(""));
    assert!(
        whole
            .point(oid(ObjectType::TREND_LOG, 1))
            .is_some_and(|log| log.to_point().is_none())
    );
    stop(client, server).await;
}

/// A device that stops answering part way gives a model that says where:
/// the first object not read, with the client's error, as the Turtle's
/// header repeats. A list that failed part way gives no model at all.
#[tokio::test(start_paused = true)]
async fn a_stopped_inventory_says_where() {
    // The summary, the Object_List and the first batch of eight pass.
    let (client, server) = lab(passing(3)).await;
    let options = inventory_options(None).unwrap();
    let inventory = lab_device(&client).await.inventory(&options).await.unwrap();
    let model = Model::from(&inventory);
    assert_eq!(model.entries.len(), 8, "{model:?}");
    let stopped = model.stopped.as_deref().unwrap();
    assert!(stopped.starts_with("multi-state-output 1: "), "{stopped}");
    let turtle = to_turtle(&model, &TurtleOptions::default());
    assert!(
        turtle.contains("# Incomplete: the reading stopped early"),
        "{turtle}"
    );
    stop(client, server).await;

    // A list read element by element that fails part way is no inventory.
    let faults = Faults {
        pass: 5,
        reject_whole_list: true,
        ..Faults::default()
    };
    let (client, server) = lab(faults).await;
    let device = lab_device(&client).await;
    assert!(device.inventory(&options).await.is_err());
    drop(device);
    stop(client, server).await;
}

/// A property every object of the type has skips the object, naming the
/// property, whether the device refused it or gave something else; an
/// optional one the device refused is `None`. A value's relinquish
/// default counts only with a Priority_Array.
#[test]
fn a_missing_required_property_skips_the_object() {
    use PropertyIdentifier as P;
    let name = || row(P::OBJECT_NAME, text("Zone"));
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let failed = |entry: Result<Point, Skipped>| entry.err().map(|skipped| skipped.error);
    let no_units = entry_of(ai, &[name(), refused(P::UNITS)]);
    assert!(
        matches!(
            no_units,
            Err(Skipped {
                property: Some(P::UNITS),
                ..
            })
        ),
        "{no_units:?}"
    );
    assert_eq!(
        failed(no_units).as_deref(),
        Some("units: PROPERTY:UNKNOWN_PROPERTY")
    );
    let not_units = [name(), row(P::UNITS, PropertyValue::Real(1.0))];
    let not_units = entry_of(ai, &not_units);
    assert_eq!(
        failed(not_units).as_deref(),
        Some("units: not an engineering unit")
    );
    let no_name = [
        refused(P::OBJECT_NAME),
        row(P::UNITS, PropertyValue::Enumerated(62)),
    ];
    let no_name = entry_of(ai, &no_name);
    assert_eq!(
        failed(no_name).as_deref(),
        Some("object-name: PROPERTY:UNKNOWN_PROPERTY")
    );
    // A record whose read never asked for the name says so.
    let unasked = entry_of(ai, &[row(P::UNITS, PropertyValue::Enumerated(62))]);
    assert_eq!(failed(unasked).as_deref(), Some("object-name: not read"));
    // An integer value needs its Units as an analog object does; its range
    // and relinquish default decode as INTEGER and its increment as an
    // Unsigned of any size, and anything else there is None, a problem on
    // the record.
    let iv = oid(ObjectType::INTEGER_VALUE, 1);
    let no_units = entry_of(iv, &[name(), refused(P::UNITS)]);
    assert_eq!(
        failed(no_units).as_deref(),
        Some("units: PROPERTY:UNKNOWN_PROPERTY")
    );
    let rows = [
        name(),
        row(P::UNITS, PropertyValue::Enumerated(19)),
        row(P::PRIORITY_ARRAY, PropertyValue::Null),
        row(P::RELINQUISH_DEFAULT, PropertyValue::Signed(-4)),
        row(P::COV_INCREMENT, PropertyValue::Unsigned(u64::MAX)),
        row(P::MIN_PRES_VALUE, PropertyValue::Signed(i32::MIN)),
        row(P::MAX_PRES_VALUE, PropertyValue::Real(5.0)),
    ];
    let mut expected = Integer::new(
        EngineeringUnits::KILOWATT_HOURS,
        Access::Commandable {
            relinquish_default: Some(-4),
        },
    )
    .with_cov_increment(u64::MAX)
    .with_min_pres_value(i32::MIN);
    expected.max_pres_value = None;
    assert_eq!(entry_of(iv, &rows), Ok(Point::new(iv, "Zone", expected)));
    let said = record(iv, &rows)
        .problem(P::MAX_PRES_VALUE)
        .map(ToString::to_string);
    let said = said.unwrap_or_default();
    assert!(said.starts_with("not a Signed (got a Real, 5"), "{said}");
    let msi = oid(ObjectType::MULTI_STATE_INPUT, 1);
    let states = [name(), row(P::NUMBER_OF_STATES, text("three"))];
    let states = entry_of(msi, &states);
    assert_eq!(
        failed(states).as_deref(),
        Some("number-of-states: not an Unsigned")
    );

    // An optional property refused is None; a binary value's relinquish
    // default without a Priority_Array is no access of its own.
    let bv = oid(ObjectType::BINARY_VALUE, 1);
    let rows = [
        name(),
        refused(P::DESCRIPTION),
        refused(P::PRIORITY_ARRAY),
        row(P::ACTIVE_TEXT, text("")),
        refused(P::INACTIVE_TEXT),
        row(P::RELINQUISH_DEFAULT, PropertyValue::Enumerated(1)),
    ];
    let read = entry_of(bv, &rows);
    let mut expected = Binary::new(Access::ReadOnly);
    expected.active_text = Some(String::new());
    assert_eq!(read, Ok(Point::new(bv, "Zone", expected)));
    let bo = oid(ObjectType::BINARY_OUTPUT, 1);
    let read = entry_of(bo, &rows);
    let mut expected = Binary::new(Access::Commandable {
        relinquish_default: Some(BinaryPV::ACTIVE),
    });
    expected.active_text = Some(String::new());
    assert_eq!(read, Ok(Point::new(bo, "Zone", expected)));

    // A type the export doesn't map has no point, and names no property.
    let trend = entry_of(oid(ObjectType::TREND_LOG, 1), &[name()]);
    let Err(skipped) = trend else {
        panic!("a trend log is no point");
    };
    assert_eq!(skipped.property, None);
    assert_eq!(skipped.error, "trend-log objects aren't exported");
}

/// A Binary Input's or Output's Polarity is the one its record holds,
/// NORMAL and REVERSE alike and any other number too, so a list keeps what
/// the device has (the record names any other number as a problem, which
/// a builder would refuse); one the device didn't give, or of another
/// type, is `None`, and the object is still a point. A Binary Value's is
/// `None` even when a hand-built record holds one: that type has no
/// Polarity, and a builder would refuse it.
#[test]
fn a_binary_points_polarity_is_the_records() {
    use PropertyIdentifier as P;
    let name = || row(P::OBJECT_NAME, text("Contact"));
    let polarity = |raw| row(P::POLARITY, PropertyValue::Enumerated(raw));
    let of = |object, rows: &[RawRow]| match entry_of(object, rows) {
        Ok(Point {
            kind: PointKind::Binary(binary),
            ..
        }) => binary.polarity,
        other => panic!("{other:?}"),
    };
    for object_type in [ObjectType::BINARY_INPUT, ObjectType::BINARY_OUTPUT] {
        let object = oid(object_type, 1);
        assert_eq!(of(object, &[name(), polarity(0)]), Some(Polarity::NORMAL));
        assert_eq!(of(object, &[name(), polarity(1)]), Some(Polarity::REVERSE));
        let other = Some(Polarity::from_raw(7));
        assert_eq!(of(object, &[name(), polarity(7)]), other);
        let said = record(object, &[name(), polarity(7)]).problems;
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(said[0].error.to_string(), "neither normal nor reverse");
        assert_eq!(of(object, &[name(), refused(P::POLARITY)]), None);
        let unsigned = row(P::POLARITY, PropertyValue::Unsigned(1));
        assert_eq!(of(object, &[name(), unsigned]), None);
    }
    let bv = oid(ObjectType::BINARY_VALUE, 1);
    assert_eq!(of(bv, &[name(), polarity(1)]), None);
}
