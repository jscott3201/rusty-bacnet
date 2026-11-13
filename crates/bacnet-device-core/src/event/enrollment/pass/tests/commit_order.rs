//! The order a pass commits in, and so the order of everything it returns
//! (#2319 PR 2b): the database's Event Enrollment order, which is the order
//! the objects were added in. The pass keys its per-enrollment state in a
//! B-tree map, which would walk them by identifier; nothing reads the map
//! in order, and these tests fail if something starts to.

use super::super::*;
use crate::event::{MessageTexts, read_committed_history_snapshot};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::BACnetTimeStamp;

const OUT_OF_RANGE: BACnetEventParameter = BACnetEventParameter::OutOfRange {
    time_delay: 0,
    low_limit: 20.0,
    high_limit: 80.0,
    deadband: 2.0,
};

/// Add Event Enrollment `instance`, watching `property` of `monitored`
/// (none: no reference at all) with OUT_OF_RANGE over 80.
fn add_enrollment(
    db: &mut ObjectDatabase,
    instance: u32,
    monitored: Option<(ObjectIdentifier, PropertyIdentifier)>,
) -> ObjectIdentifier {
    let mut ee =
        EventEnrollmentObject::new(instance, format!("EE-{instance}"), EventType::OUT_OF_RANGE)
            .unwrap();
    if let Some((oid, property)) = monitored {
        let reference = BACnetDeviceObjectPropertyReference::new_local(oid, property.to_raw());
        ee.set_object_property_reference(Some(reference)).unwrap();
    }
    ee.set_event_parameters(OUT_OF_RANGE);
    ee.set_event_enable(EventTransitionBits::all());
    let oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();
    oid
}

/// Enrollments added out of identifier order, two of each kind: a
/// HIGH_LIMIT with a REAL to send (2, 1), one over an Unsigned with no
/// payload (4, 3), one with no reference, a Reliability result (6, 5), and
/// one with detection disabled, which only gets a diagnostic (8, 7).
fn mixed() -> (ObjectDatabase, [ObjectIdentifier; 8]) {
    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(90.0);
    let class = PropertyValue::Unsigned(90);
    ai.write_property(PropertyIdentifier::NOTIFICATION_CLASS, None, class, None)
        .unwrap();
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();
    let value = Some((ai_oid, PropertyIdentifier::PRESENT_VALUE));
    let unsigned = Some((ai_oid, PropertyIdentifier::NOTIFICATION_CLASS));
    let oids = [
        add_enrollment(&mut db, 2, value),
        add_enrollment(&mut db, 1, value),
        add_enrollment(&mut db, 4, unsigned),
        add_enrollment(&mut db, 3, unsigned),
        add_enrollment(&mut db, 6, None),
        add_enrollment(&mut db, 5, None),
        add_enrollment(&mut db, 8, value),
        add_enrollment(&mut db, 7, value),
    ];
    for oid in &oids[6..] {
        let disabled = PropertyValue::Boolean(false);
        let ee = db.get_mut(oid).unwrap();
        ee.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            disabled,
            None,
        )
        .unwrap();
    }
    (db, oids)
}

fn delivered(delivery: &CommittedEventEnrollmentDelivery) -> ObjectIdentifier {
    match &delivery.result {
        CommittedEventEnrollmentResult::Normal(result) => result.enrollment_oid,
        CommittedEventEnrollmentResult::Reliability(result) => result.enrollment_oid,
    }
}

/// Every list a pass returns comes out in the order the enrollments were
/// added, not by identifier.
#[test]
fn a_pass_returns_its_commits_in_database_order() {
    let (mut db, [ee2, ee1, ee4, ee3, ee6, ee5, ee8, ee7]) = mixed();
    let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);

    let report = &batch.report;
    let transitions: Vec<_> = report
        .transitions
        .iter()
        .map(|t| t.enrollment_oid)
        .collect();
    assert_eq!(transitions, [ee2, ee1, ee4, ee3]);
    let reliability = report.reliability_results.iter().map(|r| r.enrollment_oid);
    assert_eq!(reliability.collect::<Vec<_>>(), [ee6, ee5]);
    let deliveries: Vec<_> = batch.deliveries.iter().map(delivered).collect();
    assert_eq!(deliveries, [ee2, ee1]);
    let suppressed: Vec<_> = batch.suppressed.iter().map(|s| s.enrollment_oid).collect();
    assert_eq!(suppressed, [ee4, ee3, ee6, ee5]);

    let diagnostics = report.diagnostics.iter().map(|d| d.enrollment_oid);
    assert_eq!(diagnostics.collect::<Vec<_>>(), [ee8, ee7]);
}

/// With no Device clock each commit takes the next event sequence number,
/// so commit order is what numbers them: the enrollment added first gets
/// the lower one.
#[test]
fn sequence_numbers_follow_database_order() {
    let (mut db, [ee2, ee1, ..]) = mixed();
    evaluate_event_enrollments_for_delivery(&mut db, 1);
    let sequence = |oid| {
        let object = db.get(&oid).unwrap();
        let history = read_committed_history_snapshot(
            object,
            EventTransition::ToOffnormal,
            MessageTexts::Optional,
        );
        match history.unwrap().timestamp {
            BACnetTimeStamp::SequenceNumber(number) => number,
            other => panic!("no Device clock, so a sequence number: {other:?}"),
        }
    };
    assert_eq!(sequence(ee1), sequence(ee2) + 1);
}

/// Refer `enrollment`'s Event_Algorithm_Inhibit_Ref to `property` of
/// `object`.
fn inhibit_from(
    enrollment: &mut EventEnrollmentObject,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) {
    let reference =
        bacnet_types::constructed::BACnetObjectPropertyReference::new(object, property.to_raw());
    let mut encoded = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_object_property_reference(&mut encoded, &reference);
    let encoded = PropertyValue::ApplicationData(encoded.to_vec());
    enrollment
        .write_property(
            PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
            None,
            encoded,
            None,
        )
        .unwrap();
}

fn inhibited(db: &ObjectDatabase, oid: ObjectIdentifier) -> bool {
    let inhibit = db.get(&oid).unwrap();
    let inhibit = inhibit.read_property(PropertyIdentifier::EVENT_ALGORITHM_INHIBIT, None);
    inhibit.unwrap() == PropertyValue::Boolean(true)
}

/// The start of a pass follows each enrollment's inhibit reference in
/// database order too, so one that refers to a later enrollment's own
/// Event_Algorithm_Inhibit sees it as it was before this pass: a chain of
/// them catches up one link per pass (the server's one-second refresh
/// shortens that).
#[test]
fn inhibit_references_follow_in_database_order() {
    use bacnet_objects::binary::BinaryValueObject;
    let mut db = ObjectDatabase::new();
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let bv_oid = bv.object_identifier();
    db.add(Box::new(bv)).unwrap();
    let active = PropertyValue::Enumerated(1);
    db.get_mut(&bv_oid)
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            active,
            Some(8),
            &super::helpers::test_origin(),
        )
        .unwrap();
    let first = EventEnrollmentObject::new(10, "EE-10", EventType::OUT_OF_RANGE).unwrap();
    let later = EventEnrollmentObject::new(11, "EE-11", EventType::OUT_OF_RANGE).unwrap();
    let (first_oid, later_oid) = (first.object_identifier(), later.object_identifier());
    let (mut first, mut later) = (first, later);
    inhibit_from(
        &mut first,
        later_oid,
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
    );
    inhibit_from(&mut later, bv_oid, PropertyIdentifier::PRESENT_VALUE);
    db.add(Box::new(first)).unwrap();
    db.add(Box::new(later)).unwrap();

    evaluate_event_enrollments_for_delivery(&mut db, 1);
    assert!(inhibited(&db, later_oid));
    assert!(
        !inhibited(&db, first_oid),
        "it read EE-11 before EE-11 followed"
    );
    evaluate_event_enrollments_for_delivery(&mut db, 1);
    assert!(inhibited(&db, first_oid));
}
