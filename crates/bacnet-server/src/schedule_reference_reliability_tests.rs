//! A Schedule reference its target can never write faults the Schedule
//! (#1433): a missing object, a missing property, an index on a property
//! that isn't an array, or an index past the end of an array give
//! Reliability CONFIGURATION_ERROR and FAULT in Status_Flags, as a datatype
//! the target refuses does. A denied write doesn't: that can be the target's
//! state. The fault clears once the member leaves the list or a later write
//! to it succeeds, a pass's retry of the refused member included (#1436).
//!
//! Each Schedule here holds one default value and no weekly or exception
//! entries, so its first pass in the period writes that value.

use std::sync::Arc;

use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::schedule::ScheduleObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, Reliability};
use bytes::BytesMut;

use super::tests::SettableClock;
use super::*;

use PropertyIdentifier as P;

pub(super) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

pub(super) fn sch() -> ObjectIdentifier {
    oid(ObjectType::SCHEDULE, 1)
}

pub(super) fn av(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, instance)
}

pub(super) fn reference(object: ObjectIdentifier, property: P) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(object, property.to_raw())
}

pub(super) fn indexed(
    object: ObjectIdentifier,
    property: P,
    index: u32,
) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new_indexed(object, property.to_raw(), index)
}

/// A Device, AI-1, AV-1, AV-2, MSV-1 (three states) and SCH-1, which writes
/// `default` to `references`, with the clock inside the period.
pub(super) fn database(
    default: PropertyValue,
    references: Vec<BACnetObjectPropertyReference>,
) -> Arc<RwLock<ObjectDatabase>> {
    let mut schedule = ScheduleObject::new(1, "SCH-1", default).unwrap();
    schedule.set_object_property_references(references).unwrap();
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(SettableClock::at(2026, 10, 5, 9, 0)));
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    for instance in [1, 2] {
        db.add(Box::new(
            AnalogValueObject::new(
                instance,
                format!("AV-{instance}"),
                EngineeringUnits::DEGREES_CELSIUS,
            )
            .unwrap(),
        ))
        .unwrap();
    }
    db.add(Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()))
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
    Arc::new(RwLock::new(db))
}

pub(super) fn read(db: &ObjectDatabase, object: ObjectIdentifier, property: P) -> PropertyValue {
    db.get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// SCH-1's Reliability, and whether Status_Flags shows FAULT.
pub(super) async fn health(db: &RwLock<ObjectDatabase>) -> (Reliability, bool) {
    let db = db.read().await;
    let PropertyValue::Enumerated(raw) = read(&db, sch(), P::RELIABILITY) else {
        panic!("Reliability reads as an enumeration");
    };
    let PropertyValue::BitString { data, .. } = read(&db, sch(), P::STATUS_FLAGS) else {
        panic!("Status_Flags reads as a bit string");
    };
    let fault = data.first().is_some_and(|octet| octet & 0x40 != 0);
    (Reliability::from_raw(raw), fault)
}

pub(super) const FAULTED: (Reliability, bool) = (Reliability::CONFIGURATION_ERROR, true);
pub(super) const HEALTHY: (Reliability, bool) = (Reliability::NO_FAULT_DETECTED, false);

#[tokio::test]
async fn a_reference_its_target_can_never_write_faults_the_schedule() {
    let real = || PropertyValue::Real(5.0);
    for (case, default, references) in [
        // A REAL Schedule whose second member puts an index on Description.
        (
            "PROPERTY_IS_NOT_AN_ARRAY",
            real(),
            vec![
                reference(av(1), P::PRESENT_VALUE),
                indexed(av(2), P::DESCRIPTION, 1),
            ],
        ),
        (
            "UNKNOWN_OBJECT",
            real(),
            vec![reference(av(9), P::PRESENT_VALUE)],
        ),
        (
            "UNKNOWN_PROPERTY",
            real(),
            vec![reference(av(1), P::STATE_TEXT)],
        ),
        (
            "INVALID_ARRAY_INDEX",
            PropertyValue::CharacterString("Away".into()),
            vec![indexed(
                oid(ObjectType::MULTI_STATE_VALUE, 1),
                P::STATE_TEXT,
                9,
            )],
        ),
    ] {
        let db = database(default, references);
        tick_schedules(&db).await;
        assert_eq!(health(&db).await, FAULTED, "{case}");
    }

    // The probe's good member was still written.
    let db = database(
        real(),
        vec![
            reference(av(1), P::PRESENT_VALUE),
            indexed(av(2), P::DESCRIPTION, 1),
        ],
    );
    tick_schedules(&db).await;
    assert_eq!(read(&*db.read().await, av(1), P::PRESENT_VALUE), real());

    // A denied write, here an Analog Input in service, isn't configuration.
    let db = database(
        real(),
        vec![reference(
            oid(ObjectType::ANALOG_INPUT, 1),
            P::PRESENT_VALUE,
        )],
    );
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, HEALTHY);
}

/// Write SCH-1's List_Of_Object_Property_References as a client would.
async fn write_references(
    db: &RwLock<ObjectDatabase>,
    references: &[BACnetObjectPropertyReference],
) {
    let mut encoded = BytesMut::new();
    for reference in references {
        encode_object_property_reference(&mut encoded, reference);
    }
    db.write()
        .await
        .get_mut(&sch())
        .unwrap()
        .write_property(
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            None,
            PropertyValue::ApplicationData(encoded.to_vec()),
            None,
        )
        .unwrap();
}

#[tokio::test]
async fn the_fault_clears_once_the_reference_is_fixed() {
    let db = database(
        PropertyValue::Real(5.0),
        vec![
            reference(av(1), P::PRESENT_VALUE),
            indexed(av(2), P::DESCRIPTION, 1),
        ],
    );
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, FAULTED);

    // The faulty member leaves the list.
    let fixed = [
        reference(av(1), P::PRESENT_VALUE),
        reference(av(2), P::PRESENT_VALUE),
    ];
    write_references(&db, &fixed).await;
    assert_eq!(health(&db).await, HEALTHY);
    // The new list gets the value, and both members take it.
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, HEALTHY);
    assert_eq!(
        read(&*db.read().await, av(2), P::PRESENT_VALUE),
        PropertyValue::Real(5.0)
    );
}

#[tokio::test(start_paused = true)]
async fn a_missing_object_created_later_takes_the_value_at_the_next_pass() {
    let db = database(
        PropertyValue::Real(5.0),
        vec![reference(av(9), P::PRESENT_VALUE)],
    );
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, FAULTED);
    // The value never changes, but each pass offers it to AV-9 again.
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, FAULTED, "AV-9 still missing");

    // The object appears, and the next pass writes it and clears the fault,
    // though the Schedule's value is the one it has held all along.
    db.write()
        .await
        .add(Box::new(
            AnalogValueObject::new(9, "AV-9", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    tick_schedules(&db).await;
    assert_eq!(health(&db).await, HEALTHY);
    assert_eq!(
        read(&*db.read().await, av(9), P::PRESENT_VALUE),
        PropertyValue::Real(5.0)
    );
}
