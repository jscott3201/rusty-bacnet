//! The round trip from a point list (#2445): a database built from the
//! list by `bacnet_objects::points::build_database`, served over the lab
//! link and read back by `Engine::inventory` at [`Detail::Full`], gives the
//! list again; an absent field reads back as the object's default; and
//! what `Full` costs, by RPM and by ReadProperty.

use bacnet_objects::device_identity::DeviceIdentity;
use bacnet_objects::points::build_database;
use bacnet_types::enums::{BinaryPV, EngineeringUnits, ObjectType};

use super::inventory_tests::{Faults, LAB, refusing_rpm, serve, stop};
use super::tests::oid;
use super::*;

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

/// One point of each of the ten types, every optional field given and
/// every relinquish default other than the object's own; then a value of
/// each family, the Integer Value's among them, for each other access.
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
            binary(Access::ReadOnly).into(),
        ),
        point(
            ObjectType::BINARY_OUTPUT,
            1,
            "bo",
            binary(commanded(Some(BinaryPV::ACTIVE))).into(),
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
async fn read_back(
    list: PointList,
    faults: Faults,
    options: &InventoryOptions,
) -> (Inventory, usize) {
    let database = build_database(list, &identity()).unwrap();
    let (engine, server) = serve(faults, identity(), database).await;
    let inventory = engine.inventory(LAB, options).await.unwrap();
    let requests = engine.limits().recent();
    stop(engine, server).await;
    (inventory, requests)
}

fn full() -> InventoryOptions {
    InventoryOptions::default().with_detail(Detail::Full)
}

/// A list built into a database, served and read back at `Full` is the
/// list again, by RPM and by ReadProperty alike: the Device's name and
/// Description, and every point's every field, a Writable value reading
/// as ReadOnly. By RPM it costs N + 2, as the export does.
#[tokio::test(start_paused = true)]
async fn a_full_list_reads_back_as_built() {
    let list = full_list();
    for (faults, by_rpm) in [(Faults::default(), true), (refusing_rpm(), false)] {
        let (inventory, requests) = read_back(list.clone(), faults, &full()).await;
        assert_eq!(inventory.stopped, None);
        assert!(
            inventory
                .entries
                .iter()
                .all(|entry| entry.error().is_none()),
            "{inventory:?}"
        );
        assert_eq!(inventory.point_list(), as_read(list.clone()));
        if by_rpm {
            assert_eq!(requests, list.points.len() + 2);
        }
    }
    // At Export the same device reads as the export has always read it:
    // none of the settings Full adds.
    let (inventory, requests) = read_back(
        list.clone(),
        Faults::default(),
        &InventoryOptions::default(),
    )
    .await;
    assert_eq!(requests, list.points.len() + 2);
    assert_eq!(inventory.device.description, None);
    let PointKind::Analog(av) = &inventory.points().nth(2).unwrap().kind else {
        panic!("{inventory:?}");
    };
    let expected = Analog::new(EngineeringUnits::DEGREES_CELSIUS, commanded(None));
    assert_eq!(av, &expected);
}

/// With no optional field given and every relinquish default left to the
/// object, each reads back as the object's own default: an empty
/// Description, COV_Increment 0, no Min_Pres_Value or Max_Pres_Value,
/// "Active" and "Inactive", "State N", and a relinquish default of 0 (an
/// analog object's and an Integer Value's), INACTIVE or state 1. A change
/// to an object's default fails here.
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
    let (inventory, _) = read_back(list.clone(), Faults::default(), &full()).await;

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
        binary(Access::ReadOnly).into(),
        binary(commanded(Some(BinaryPV::INACTIVE))).into(),
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
    assert_eq!(inventory.point_list(), PointList::new(device, points));
}

/// What `Full` costs from a device that refuses RPM, each type read alone:
/// the refused RPM (the Device's, now), the Device's name and Description,
/// the Object_List, then a ReadProperty a property, by type.
#[tokio::test(start_paused = true)]
async fn full_by_read_property() {
    let costs = [
        (ObjectType::ANALOG_INPUT, 6),
        (ObjectType::ANALOG_OUTPUT, 7),
        (ObjectType::ANALOG_VALUE, 8),
        (ObjectType::BINARY_INPUT, 4),
        (ObjectType::BINARY_OUTPUT, 5),
        (ObjectType::BINARY_VALUE, 6),
        (ObjectType::MULTI_STATE_INPUT, 4),
        (ObjectType::MULTI_STATE_OUTPUT, 5),
        (ObjectType::MULTI_STATE_VALUE, 6),
        (ObjectType::INTEGER_VALUE, 8),
    ];
    let list = full_list();
    for (object_type, cost) in costs {
        let options = InventoryOptions::new(vec![object_type])
            .unwrap()
            .with_detail(Detail::Full);
        let (inventory, requests) = read_back(list.clone(), refusing_rpm(), &options).await;
        let objects = list
            .points
            .iter()
            .filter(|point| point.object.object_type() == object_type)
            .count();
        assert_eq!(inventory.points().count(), objects, "{object_type}");
        assert_eq!(requests, 1 + 2 + 1 + objects * cost, "{object_type}");
    }
}
