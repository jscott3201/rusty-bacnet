//! The round trip from a point list (#2445): a database built from the
//! list by `bacnet_objects::points::build_database`, served over the lab
//! link and read back by `Device::inventory` under [`point_list_options`],
//! gives the list again, a REVERSE Polarity included (#2760); an absent
//! field reads back as the object's default; a device that doesn't give a
//! Polarity leaves it unset and the rest of the list whole; and what that
//! reading costs, by RPM and by ReadProperty.

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device_identity::DeviceIdentity;
use bacnet_objects::points::build_database;
use bacnet_types::enums::{
    BinaryPV, EngineeringUnits, ErrorClass, ErrorCode, ObjectType, Polarity,
};
use bacnet_types::primitives::PropertyValue;

use super::tests::oid;
use super::*;
use crate::inventory::test_lab::{Answering, Faults, LAB, lab_device, refusing_rpm, serve, stop};

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(LAB, 42).unwrap()
}

fn commanded<T>(relinquish_default: Option<T>) -> Access<T> {
    Access::Commandable { relinquish_default }
}

/// A value that isn't commandable: Writable or ReadOnly.
fn plain<T>(writable: bool) -> Access<T> {
    if writable {
        Access::Writable
    } else {
        Access::ReadOnly
    }
}

/// One point of each of the ten types, every optional field given (the
/// Binary Input's and Output's Polarity REVERSE) and every relinquish
/// default other than the object's own; then a value of each family, the
/// Integer Value's among them, for each other access.
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
    let multi_state = |access| MultiState::new(3, access).with_state_texts(["Off", "Low", "High"]);
    let integer = |access| {
        Integer::new(EngineeringUnits::KILOWATT_HOURS, access)
            .with_cov_increment(10)
            .with_min_pres_value(-500)
            .with_max_pres_value(1_000_000)
    };
    let point = |object_type, instance, name: &str, kind: PointKind| {
        Point::new(oid(object_type, instance), name, kind)
            .with_description(format!("{name}, as the list gives it"))
    };
    let mut points = vec![
        point(
            ObjectType::ANALOG_INPUT,
            1,
            "ai",
            analog(Access::ReadOnly).into(),
        ),
        point(
            ObjectType::ANALOG_OUTPUT,
            1,
            "ao",
            analog(commanded(Some(12.5))).into(),
        ),
        point(
            ObjectType::ANALOG_VALUE,
            1,
            "av",
            analog(commanded(Some(21.5))).into(),
        ),
        point(
            ObjectType::BINARY_INPUT,
            1,
            "bi",
            binary(Access::ReadOnly)
                .with_polarity(Polarity::REVERSE)
                .into(),
        ),
        point(
            ObjectType::BINARY_OUTPUT,
            1,
            "bo",
            binary(commanded(Some(BinaryPV::ACTIVE)))
                .with_polarity(Polarity::REVERSE)
                .into(),
        ),
        point(
            ObjectType::BINARY_VALUE,
            1,
            "bv",
            binary(commanded(Some(BinaryPV::ACTIVE))).into(),
        ),
        point(
            ObjectType::MULTI_STATE_INPUT,
            1,
            "msi",
            multi_state(Access::ReadOnly).into(),
        ),
        point(
            ObjectType::MULTI_STATE_OUTPUT,
            1,
            "mso",
            multi_state(commanded(Some(2))).into(),
        ),
        point(
            ObjectType::MULTI_STATE_VALUE,
            1,
            "msv",
            multi_state(commanded(Some(3))).into(),
        ),
        point(
            ObjectType::INTEGER_VALUE,
            1,
            "iv",
            integer(commanded(Some(-7))).into(),
        ),
    ];
    for (instance, suffix, writable) in [(2, "writable", true), (3, "read-only", false)] {
        let named = |family: &str| format!("{family} {suffix}");
        let kinds: [(ObjectType, &str, PointKind); 4] = [
            (
                ObjectType::ANALOG_VALUE,
                "av",
                analog(plain(writable)).into(),
            ),
            (
                ObjectType::BINARY_VALUE,
                "bv",
                binary(plain(writable)).into(),
            ),
            (
                ObjectType::MULTI_STATE_VALUE,
                "msv",
                multi_state(plain(writable)).into(),
            ),
            (
                ObjectType::INTEGER_VALUE,
                "iv",
                integer(plain(writable)).into(),
            ),
        ];
        for (object_type, family, kind) in kinds {
            points.push(point(object_type, instance, &named(family), kind));
        }
    }
    PointList::new(
        DeviceInfo::new(LAB)
            .with_name("AHU-1")
            .with_description("Air handler 1"),
        points,
    )
}

/// `list` as a read gives it back: a read can't tell a Writable value from
/// a ReadOnly one, so a Writable one reads as ReadOnly.
fn as_read(mut list: PointList) -> PointList {
    fn fold<T>(access: &mut Access<T>) {
        if matches!(access, Access::Writable) {
            *access = Access::ReadOnly;
        }
    }
    for point in &mut list.points {
        match &mut point.kind {
            PointKind::Analog(analog) => fold(&mut analog.access),
            PointKind::Binary(binary) => fold(&mut binary.access),
            PointKind::MultiState(multi_state) => fold(&mut multi_state.access),
            PointKind::Integer(integer) => fold(&mut integer.access),
            _ => {}
        }
    }
    list
}

/// `list` built, served over a link with `faults`, and read back at
/// `options`; with the requests it took.
async fn read_back(list: PointList, faults: Faults, options: &InventoryOptions) -> (Model, usize) {
    let database = build_database(list, &identity()).unwrap();
    let (client, server) = serve(faults, identity(), database).await;
    let inventory = lab_device(&client).await.inventory(options).await.unwrap();
    let requests = client.limits().recent();
    stop(client, server).await;
    assert!(inventory.stopped.is_none(), "{inventory:?}");
    (Model::from(&inventory), requests)
}

fn full() -> InventoryOptions {
    point_list_options()
}

/// A list built into a database, served and read back with the point
/// list's options is the list again, by RPM and by ReadProperty alike: the
/// Device's name and Description, and every point's every field, a
/// Writable value reading as ReadOnly. By RPM its fourteen objects (the
/// Device object among them) take far fewer requests than one each.
#[tokio::test(start_paused = true)]
async fn a_full_list_reads_back_as_built() {
    let list = full_list();
    for (faults, by_rpm) in [(Faults::default(), true), (refusing_rpm(), false)] {
        let (model, requests) = read_back(list.clone(), faults, &full()).await;
        assert!(model.entries.iter().all(Result::is_ok), "{model:?}");
        assert_eq!(model.point_list(), as_read(list.clone()));
        if by_rpm {
            assert!(requests < list.points.len(), "{requests}");
        }
    }
    // With the export's own options the same device reads as the export
    // reads it: none of the settings, and no Description for the Device.
    let export = inventory_options(None).unwrap();
    let (model, _) = read_back(list.clone(), Faults::default(), &export).await;
    assert_eq!(model.device.description, None);
    let PointKind::Analog(av) = &model.points().nth(2).unwrap().kind else {
        panic!("{model:?}");
    };
    let expected = Analog::new(EngineeringUnits::DEGREES_CELSIUS, commanded(None));
    assert_eq!(av, &expected);
}

/// A Binary Input and Output built REVERSE read back REVERSE, and the
/// list read builds them REVERSE again: relinquished to INACTIVE, the
/// output drives ON before and after. The Binary Value beside them is
/// never asked for a Polarity, so it has none and no problem.
#[tokio::test(start_paused = true)]
async fn a_reverse_polarity_survives_the_round_trip() {
    let bo = oid(ObjectType::BINARY_OUTPUT, 1);
    let list = PointList::new(
        DeviceInfo::new(LAB),
        vec![
            Point::new(
                oid(ObjectType::BINARY_INPUT, 1),
                "bi",
                Binary::new(Access::ReadOnly).with_polarity(Polarity::REVERSE),
            ),
            Point::new(
                bo,
                "bo",
                Binary::new(commanded(None)).with_polarity(Polarity::REVERSE),
            ),
            Point::new(
                oid(ObjectType::BINARY_VALUE, 1),
                "bv",
                Binary::new(commanded(None)),
            ),
        ],
    );
    let database = build_database(list.clone(), &identity()).unwrap();
    assert_eq!(database.physical_output(&bo), Some(true));
    let (model, _) = read_back(list, Faults::default(), &full()).await;
    let read = model.point_list();
    let polarities: Vec<_> = read
        .points
        .iter()
        .map(|point| match &point.kind {
            PointKind::Binary(binary) => binary.polarity,
            kind => panic!("{kind:?}"),
        })
        .collect();
    let reverse = Some(Polarity::REVERSE);
    assert_eq!(polarities, [reverse, reverse, None]);
    let rebuilt = build_database(read, &identity()).unwrap();
    assert_eq!(rebuilt.physical_output(&bo), Some(true));
    let bi = rebuilt.get(&oid(ObjectType::BINARY_INPUT, 1)).unwrap();
    let served = bi.read_property(PropertyIdentifier::POLARITY, None);
    assert_eq!(served.unwrap(), PropertyValue::Enumerated(1));
}

/// `list` built, its Binary Inputs and Outputs then put in their places
/// behind an [`Answering`] that calls Polarity an unknown property and
/// leaves it out of the Property_List: an older or non-conforming device.
fn without_polarity(list: &PointList) -> ObjectDatabase {
    let mut served = build_database(list.clone(), &identity()).unwrap();
    let mut source = build_database(list.clone(), &identity()).unwrap();
    let unknown = (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY);
    for point in &list.points {
        let object_type = point.object.object_type();
        if [ObjectType::BINARY_INPUT, ObjectType::BINARY_OUTPUT].contains(&object_type) {
            let object = source.remove(&point.object).unwrap().unwrap();
            let answers = vec![(PropertyIdentifier::POLARITY, Err(unknown))];
            served.add(Answering::wrap(object, answers)).unwrap();
        }
    }
    served
}

/// A device whose Binary Input and Output don't give their Polarity, by
/// RPM and by ReadProperty: the reading goes on, each record names the
/// problem, and each point is the list's with no Polarity, which a
/// builder makes NORMAL; every other field reads as the full list's do.
#[tokio::test(start_paused = true)]
async fn a_polarity_the_device_doesnt_give_is_unset() {
    let list = full_list();
    let mut expected = as_read(list.clone());
    for point in &mut expected.points {
        if let PointKind::Binary(binary) = &mut point.kind {
            binary.polarity = None;
        }
    }
    let binaries = [
        oid(ObjectType::BINARY_INPUT, 1),
        oid(ObjectType::BINARY_OUTPUT, 1),
    ];
    for faults in [Faults::default(), refusing_rpm()] {
        let (client, server) = serve(faults, identity(), without_polarity(&list)).await;
        let inventory = lab_device(&client).await.inventory(&full()).await.unwrap();
        stop(client, server).await;
        assert!(inventory.stopped.is_none(), "{inventory:?}");
        for record in &inventory.points {
            let problems: Vec<_> = record.problems.iter().map(|p| p.property).collect();
            if binaries.contains(&record.object) {
                assert_eq!(problems, [PropertyIdentifier::POLARITY], "{record:?}");
                let error = record.problem(PropertyIdentifier::POLARITY).unwrap();
                assert_eq!(error.to_string(), "PROPERTY:UNKNOWN_PROPERTY");
                assert!(record.polarity.is_none());
            } else {
                assert!(problems.is_empty(), "{record:?}");
            }
        }
        let model = Model::from(&inventory);
        assert!(model.entries.iter().all(Result::is_ok), "{model:?}");
        assert_eq!(model.point_list(), expected);
    }
}

/// With no optional field given and every relinquish default left to the
/// object, each reads back as the object's own default: an empty
/// Description, COV_Increment 0, no Min_Pres_Value or Max_Pres_Value,
/// "Active" and "Inactive", "State N", a relinquish default of 0 (an
/// analog object's and an Integer Value's), INACTIVE or state 1, and a
/// Binary Input's and Output's Polarity NORMAL (a Binary Value has none).
/// A change to an object's default fails here. The list read is a fixed
/// point: built and read again, it reads the same.
#[tokio::test(start_paused = true)]
async fn an_absent_field_reads_back_as_the_objects_default() {
    let units = EngineeringUnits::PERCENT;
    let bare =
        |object_type, name: &str, kind: PointKind| Point::new(oid(object_type, 1), name, kind);
    let list = PointList::new(
        DeviceInfo::new(LAB),
        vec![
            bare(
                ObjectType::ANALOG_INPUT,
                "ai",
                Analog::new(units, Access::ReadOnly).into(),
            ),
            bare(
                ObjectType::ANALOG_OUTPUT,
                "ao",
                Analog::new(units, commanded(None)).into(),
            ),
            bare(
                ObjectType::ANALOG_VALUE,
                "av",
                Analog::new(units, commanded(None)).into(),
            ),
            bare(
                ObjectType::BINARY_INPUT,
                "bi",
                Binary::new(Access::ReadOnly).into(),
            ),
            bare(
                ObjectType::BINARY_OUTPUT,
                "bo",
                Binary::new(commanded(None)).into(),
            ),
            bare(
                ObjectType::BINARY_VALUE,
                "bv",
                Binary::new(commanded(None)).into(),
            ),
            bare(
                ObjectType::MULTI_STATE_INPUT,
                "msi",
                MultiState::new(2, Access::ReadOnly).into(),
            ),
            bare(
                ObjectType::MULTI_STATE_OUTPUT,
                "mso",
                MultiState::new(2, commanded(None)).into(),
            ),
            bare(
                ObjectType::MULTI_STATE_VALUE,
                "msv",
                MultiState::new(2, commanded(None)).into(),
            ),
            bare(
                ObjectType::INTEGER_VALUE,
                "iv",
                Integer::new(units, commanded(None)).into(),
            ),
        ],
    );
    let (model, _) = read_back(list.clone(), Faults::default(), &full()).await;

    let analog = |access| Analog::new(units, access).with_cov_increment(0.0);
    let binary = |access| {
        Binary::new(access)
            .with_active_text("Active")
            .with_inactive_text("Inactive")
    };
    let multi_state = |access| MultiState::new(2, access).with_state_texts(["State 1", "State 2"]);
    let kinds: [PointKind; 10] = [
        analog(Access::ReadOnly).into(),
        analog(commanded(Some(0.0))).into(),
        analog(commanded(Some(0.0))).into(),
        binary(Access::ReadOnly)
            .with_polarity(Polarity::NORMAL)
            .into(),
        binary(commanded(Some(BinaryPV::INACTIVE)))
            .with_polarity(Polarity::NORMAL)
            .into(),
        binary(commanded(Some(BinaryPV::INACTIVE))).into(),
        multi_state(Access::ReadOnly).into(),
        multi_state(commanded(Some(1))).into(),
        multi_state(commanded(Some(1))).into(),
        Integer::new(units, commanded(Some(0)))
            .with_cov_increment(0)
            .into(),
    ];
    let points = list
        .points
        .into_iter()
        .zip(kinds)
        .map(|(point, kind)| Point {
            kind,
            ..point.with_description("")
        })
        .collect();
    let device = DeviceInfo::new(LAB)
        .with_name(identity().name())
        .with_description("");
    assert_eq!(model.point_list(), PointList::new(device, points));
    let (again, _) = read_back(model.point_list(), Faults::default(), &full()).await;
    assert_eq!(again.point_list(), model.point_list());
}

/// What a point list's reading costs from a device that refuses RPM, each
/// type read alone: the refused RPM (the summary's), the summary's
/// thirteen properties, the Object_List, then a ReadProperty a property,
/// by type. A binary object reads its name, Description and two texts;
/// then Polarity for an Input; Polarity and Relinquish_Default for an
/// Output; Priority_Array and Relinquish_Default for a Value, which has no
/// Polarity to ask for.
#[tokio::test(start_paused = true)]
async fn full_by_read_property() {
    let costs = [
        (ObjectType::ANALOG_INPUT, 6),
        (ObjectType::ANALOG_OUTPUT, 7),
        (ObjectType::ANALOG_VALUE, 8),
        (ObjectType::BINARY_INPUT, 5),
        (ObjectType::BINARY_OUTPUT, 6),
        (ObjectType::BINARY_VALUE, 6),
        (ObjectType::MULTI_STATE_INPUT, 4),
        (ObjectType::MULTI_STATE_OUTPUT, 5),
        (ObjectType::MULTI_STATE_VALUE, 6),
        (ObjectType::INTEGER_VALUE, 8),
    ];
    let list = full_list();
    for (object_type, cost) in costs {
        let options = InventoryOptions::new()
            .with_types([object_type])
            .with_settings(true);
        let (model, requests) = read_back(list.clone(), refusing_rpm(), &options).await;
        let objects = list
            .points
            .iter()
            .filter(|point| point.object.object_type() == object_type)
            .count();
        assert_eq!(model.points().count(), objects, "{object_type}");
        assert_eq!(requests, 1 + 13 + 1 + objects * cost, "{object_type}");
    }
}
