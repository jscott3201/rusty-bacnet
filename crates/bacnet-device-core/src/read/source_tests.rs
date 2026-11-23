//! The local Device's Object_List and Database_Revision through the core's
//! read plan (#2669): with no overlay, as a firmware responder reads, and
//! through the COV overlay's Device view, as its reporter reads; in a
//! Group's members; and in a ReadPropertyMultiple's ALL.

use super::*;
use crate::cov::CovTable;
use crate::cov::active::ActiveCovOverlay;
use crate::read::{NoOverlay, ReadOverlay, ReadScope, RpmPlan, plan_property, read_value};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::group::GroupObject;
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_time::Instant;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

const LIST: PropertyIdentifier = PropertyIdentifier::OBJECT_LIST;
const REVISION: PropertyIdentifier = PropertyIdentifier::DATABASE_REVISION;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn device() -> ObjectIdentifier {
    oid(ObjectType::DEVICE, 77)
}

fn reference(property: PropertyIdentifier, index: Option<u32>) -> PropertyReference {
    PropertyReference {
        property_identifier: property,
        property_array_index: index,
    }
}

/// Device 77, AI-1 and Group-1, whose members are the Device's Object_List
/// count and its Database_Revision.
fn database() -> ObjectDatabase {
    let mut group = GroupObject::new(1, "Group-1").unwrap();
    group
        .add_member(ReadAccessSpecification {
            object_identifier: device(),
            list_of_property_references: vec![reference(LIST, Some(0)), reference(REVISION, None)],
        })
        .unwrap();
    build_database_with_extra(
        &DeviceIdentity::new(77, 0)
            .unwrap()
            .with_segmentation(bacnet_types::enums::Segmentation::NO_SEGMENTATION),
        vec![
            Box::new(AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap()),
            Box::new(group),
        ],
    )
    .unwrap()
}

fn read<O: ReadOverlay>(
    db: &ObjectDatabase,
    overlay: Option<&O>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<PropertyValue, Error> {
    let plan = plan_property(db, &ReadScope::UNLIMITED, overlay, object, property, index)
        .unwrap_or_else(|_| panic!("planned"));
    read_value(db, overlay, plan)
}

fn ids(list: &[ObjectIdentifier]) -> PropertyValue {
    PropertyValue::List(
        list.iter()
            .copied()
            .map(PropertyValue::ObjectIdentifier)
            .collect(),
    )
}

/// What each read of the Device's two properties answers, with `overlay`.
fn expect<O: ReadOverlay>(
    db: &ObjectDatabase,
    overlay: Option<&O>,
    list: &[ObjectIdentifier],
    revision: u64,
) {
    let last = list.len() as u32;
    assert_eq!(read(db, overlay, device(), LIST, None).unwrap(), ids(list));
    assert_eq!(
        read(db, overlay, device(), LIST, Some(0)).unwrap(),
        PropertyValue::Unsigned(u64::from(last))
    );
    assert_eq!(
        read(db, overlay, device(), LIST, Some(last)).unwrap(),
        PropertyValue::ObjectIdentifier(list[list.len() - 1])
    );
    let past = read(db, overlay, device(), LIST, Some(last + 1)).unwrap_err();
    assert!(
        matches!(past, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY && code == ErrorCode::INVALID_ARRAY_INDEX),
        "{past:?}"
    );
    assert_eq!(
        read(db, overlay, device(), REVISION, None).unwrap(),
        PropertyValue::Unsigned(revision)
    );
}

/// With no overlay, as a firmware responder reads, the Device's list and
/// revision follow the application's adds and removes.
#[test]
fn a_read_with_no_overlay_follows_the_database() {
    let mut db = database();
    let group = oid(ObjectType::GROUP, 1);
    let (ai1, bv2) = (
        oid(ObjectType::ANALOG_INPUT, 1),
        oid(ObjectType::BINARY_VALUE, 2),
    );
    expect(&db, None::<&NoOverlay>, &[device(), ai1, group], 3);
    db.add(Box::new(BinaryValueObject::new(2, "BV-2").unwrap()))
        .unwrap();
    expect(&db, None::<&NoOverlay>, &[device(), ai1, group, bv2], 4);
    drop(db.remove(&ai1).unwrap());
    expect(&db, None::<&NoOverlay>, &[device(), group, bv2], 5);
}

/// Through the COV overlay, whose view serves the Device for the reporter,
/// the two properties still come from the database.
#[test]
fn a_read_through_the_cov_overlay_serves_the_database() {
    let mut db = database();
    db.add(Box::new(BinaryValueObject::new(2, "BV-2").unwrap()))
        .unwrap();
    let table = CovTable::default();
    let overlay = ActiveCovOverlay::<'_, false>::new(&table, &db, device(), Instant::ZERO);
    expect(
        &db,
        Some(&overlay),
        &[
            device(),
            oid(ObjectType::ANALOG_INPUT, 1),
            oid(ObjectType::GROUP, 1),
            oid(ObjectType::BINARY_VALUE, 2),
        ],
        4,
    );
}

/// A Group's member naming the Device's Object_List count and its
/// Database_Revision reads the database's, as the Device itself does.
#[test]
fn a_groups_member_reads_the_database() {
    let mut db = database();
    db.add(Box::new(BinaryValueObject::new(2, "BV-2").unwrap()))
        .unwrap();
    let PropertyValue::List(members) = read(
        &db,
        None::<&NoOverlay>,
        oid(ObjectType::GROUP, 1),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    )
    .unwrap() else {
        panic!("a Group's Present_Value is a list");
    };
    let PropertyValue::ApplicationData(encoded) = &members[0] else {
        panic!("each member is one encoded ReadAccessResult");
    };
    let mut ack = BytesMut::new();
    ack.extend_from_slice(encoded);
    let result = ReadPropertyMultipleACK::decode(&ack).unwrap();
    let rows = &result.list_of_read_access_results[0].list_of_results;
    let values: Vec<_> = rows
        .iter()
        .map(|row| {
            decode_application_value(row.property_value.as_ref().unwrap(), 0)
                .unwrap()
                .0
        })
        .collect();
    assert_eq!(
        values,
        [PropertyValue::Unsigned(4), PropertyValue::Unsigned(4)]
    );
}

/// A ReadPropertyMultiple of the Device's ALL reads both from the database.
#[test]
fn rpm_all_reads_both_from_the_database() {
    let mut db = database();
    drop(db.remove(&oid(ObjectType::GROUP, 1)).unwrap());
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: device(),
            list_of_property_references: vec![reference(PropertyIdentifier::ALL, None)],
        }],
    };
    let scope = ReadScope::UNLIMITED;
    let plan = RpmPlan::new(&db, &scope, None::<&NoOverlay>, &request, usize::MAX).unwrap();
    let mut out = BytesMut::new();
    plan.read(
        &db,
        None::<&NoOverlay>,
        &mut out,
        usize::MAX,
        |_, _, _, _| {},
    )
    .unwrap_or_else(|_| panic!("read"));
    let ack = ReadPropertyMultipleACK::decode(&out).unwrap();
    let row = |property| {
        let row = ack.list_of_read_access_results[0]
            .list_of_results
            .iter()
            .find(|row| row.property_identifier == property)
            .unwrap();
        row.property_value.clone().unwrap()
    };
    let mut list = BytesMut::new();
    for entry in [device(), oid(ObjectType::ANALOG_INPUT, 1)] {
        bacnet_encoding::primitives::encode_property_value(
            &mut list,
            &PropertyValue::ObjectIdentifier(entry),
        )
        .unwrap();
    }
    assert_eq!(row(LIST), list.to_vec());
    // 3 added, one removed.
    assert_eq!(row(REVISION), [0x21, 4]);
}
