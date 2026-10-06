//! The two terms of an access rule (#1331, Clause 12.34.9.1): how each
//! time-range value reads as TRUE or FALSE, the time ranges with nothing to
//! judge, and which locations cover a point.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_types::enums::BinaryPV;

use super::tests::*;
use super::*;
use crate::access_control::{AccessCredentialObject, AccessZoneObject};
use crate::binary::BinaryValueObject;
use crate::command_source::CommandOrigin;
use crate::schedule::ScheduleObject;

fn schedule(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::SCHEDULE, instance)
}

/// How a positive rule for Point 1 fares at Point 1 with a given time range.
#[derive(Debug, PartialEq, Eq)]
enum Judged {
    /// TRUE: the rule grants.
    Holds,
    /// A value read FALSE: DENIED_OUT_OF_TIME_RANGE, naming the rule.
    OutOfRange,
    /// Nothing to judge: DENIED_NO_ACCESS_RIGHTS.
    Never,
}

/// How a positive rule for Point 1 whose time range is `time_range` fares at
/// Point 1, in `db`.
fn judge(mut db: ObjectDatabase, time_range: BACnetDeviceObjectPropertyReference) -> Judged {
    add_rights(&mut db, 1, vec![during(time_range, Some(point(1)))], vec![]);
    add_credential(&mut db, &[rights(1).into()]);
    judge_decision(decision(&db))
}

fn judge_decision(decision: AccessRightsDecision) -> Judged {
    if decision == granted(1, 1) {
        Judged::Holds
    } else if decision == out_of_time(1, 1) {
        Judged::OutOfRange
    } else {
        assert_eq!(decision, no_rights());
        Judged::Never
    }
}

/// How Schedule 1's Present_Value, holding `value`, reads.
fn schedule_holding(value: PropertyValue) -> Judged {
    let mut db = database();
    db.add(Box::new(ScheduleObject::new(1, "SCHED-1", value).unwrap()))
        .unwrap();
    judge(db, present_value(schedule(1)))
}

#[test]
fn time_range_values_named_by_the_clause() {
    use Judged::*;
    assert_eq!(schedule_holding(PropertyValue::Boolean(true)), Holds);
    assert_eq!(schedule_holding(PropertyValue::Boolean(false)), OutOfRange);
    assert_eq!(schedule_holding(PropertyValue::Unsigned(1)), Holds);
    assert_eq!(schedule_holding(PropertyValue::Unsigned(u64::MAX)), Holds);
    assert_eq!(schedule_holding(PropertyValue::Unsigned(0)), OutOfRange);
    assert_eq!(schedule_holding(PropertyValue::Signed(1)), Holds);
    assert_eq!(schedule_holding(PropertyValue::Signed(0)), OutOfRange);
    assert_eq!(schedule_holding(PropertyValue::Signed(-1)), OutOfRange);
    // A Schedule's Present_Value has no fixed enumeration, so it reads as a
    // BACnetBinaryPV.
    let active = PropertyValue::Enumerated(BinaryPV::ACTIVE.to_raw());
    assert_eq!(schedule_holding(active), Holds);
    let inactive = PropertyValue::Enumerated(BinaryPV::INACTIVE.to_raw());
    assert_eq!(schedule_holding(inactive), OutOfRange);
    // NULL leaves nothing to judge.
    assert_eq!(schedule_holding(PropertyValue::Null), Never);
}

#[test]
fn time_range_values_the_clause_leaves_to_the_device_read_false() {
    use Judged::*;
    assert_eq!(schedule_holding(PropertyValue::Enumerated(2)), OutOfRange);
    assert_eq!(schedule_holding(PropertyValue::Real(1.0)), OutOfRange);
    assert_eq!(schedule_holding(PropertyValue::Double(1.0)), OutOfRange);
    let text = PropertyValue::CharacterString("on".into());
    assert_eq!(schedule_holding(text), OutOfRange);
    assert_eq!(
        schedule_holding(PropertyValue::OctetString(vec![1])),
        OutOfRange
    );
    let identifier = PropertyValue::ObjectIdentifier(point(1));
    assert_eq!(schedule_holding(identifier), OutOfRange);
}

#[test]
fn an_enumerated_time_range_reads_by_its_property() {
    // Reliability 1 is NO_SENSOR, no BACnetBinaryPV: FALSE, though the
    // number is ACTIVE's. Present_Value's enumeration depends on the object,
    // so the same number there reads as ACTIVE.
    let one = PropertyValue::Enumerated(1);
    let analog = oid(ObjectType::ANALOG_VALUE, 5);
    let reading = |property: PropertyIdentifier| {
        let mut db = database();
        db.add(Stub::new(
            analog,
            vec![
                (PropertyIdentifier::RELIABILITY, one.clone()),
                (PropertyIdentifier::PRESENT_VALUE, one.clone()),
            ],
        ))
        .unwrap();
        judge(
            db,
            BACnetDeviceObjectPropertyReference::new_local(analog, property.to_raw()),
        )
    };
    assert_eq!(reading(PropertyIdentifier::RELIABILITY), Judged::OutOfRange);
    assert_eq!(reading(PropertyIdentifier::PRESENT_VALUE), Judged::Holds);

    // Credential_Status is a BACnetBinaryPV, ACTIVE on a new credential.
    let mut db = database();
    let card = AccessCredentialObject::new(9, "CARD-9").unwrap();
    db.add(Box::new(card)).unwrap();
    let status = BACnetDeviceObjectPropertyReference::new_local(
        credential(9),
        PropertyIdentifier::CREDENTIAL_STATUS.to_raw(),
    );
    assert_eq!(judge(db, status), Judged::Holds);
}

#[test]
fn time_range_always_holds_and_an_unknown_specifier_never_does() {
    let mut db = database();
    add_rights(&mut db, 1, vec![anytime(Some(point(1)))], vec![]);
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), granted(1, 1));

    // Specifier 2 names neither SPECIFIED nor ALWAYS, and SPECIFIED without
    // a reference names nothing to read. The built-in object refuses both,
    // so an application's own object serves them.
    let mut unknown = anytime(Some(point(1)));
    unknown.time_range_specifier = AccessRuleTimeRangeSpecifier::from_raw(2);
    let mut no_reference = anytime(Some(point(1)));
    no_reference.time_range_specifier = AccessRuleTimeRangeSpecifier::SPECIFIED;
    for rule in [unknown, no_reference] {
        let mut db = database();
        db.add(stub_rights(
            1,
            PropertyValue::Boolean(true),
            std::slice::from_ref(&rule),
        ))
        .unwrap();
        add_credential(&mut db, &[rights(1).into()]);
        assert_eq!(judge_decision(decision(&db)), Judged::Never, "{rule:?}");
    }
}

/// A database holding Schedule 1, its Present_Value TRUE.
fn with_true_schedule() -> ObjectDatabase {
    let mut db = database();
    db.add(Box::new(
        ScheduleObject::new(1, "SCHED-1", PropertyValue::Boolean(true)).unwrap(),
    ))
    .unwrap();
    db
}

#[test]
fn time_range_references_with_nothing_to_judge() {
    let on = present_value(schedule(1));
    assert_eq!(judge(with_true_schedule(), on.clone()), Judged::Holds);
    // Unspecified: instance 4194303, and the device too when named.
    let unspecified = present_value(schedule(UNSPECIFIED));
    assert_eq!(judge(with_true_schedule(), unspecified), Judged::Never);
    let mut unspecified_in_device = present_value(schedule(UNSPECIFIED));
    unspecified_in_device.device_identifier = Some(device(UNSPECIFIED));
    assert_eq!(
        judge(with_true_schedule(), unspecified_in_device),
        Judged::Never
    );
    // A missing object, and a property the object doesn't serve.
    let missing = present_value(schedule(2));
    assert_eq!(judge(with_true_schedule(), missing), Judged::Never);
    let mut unserved = on.clone();
    unserved.property_identifier = PropertyIdentifier::LOG_ENABLE.to_raw();
    assert_eq!(judge(with_true_schedule(), unserved), Judged::Never);
    // Another device, or the wildcard Device, even though Schedule 1 exists
    // here: never read.
    for device_instance in [OTHER_DEVICE, UNSPECIFIED] {
        let mut remote = on.clone();
        remote.device_identifier = Some(device(device_instance));
        assert_eq!(judge(with_true_schedule(), remote), Judged::Never);
    }
}

#[test]
fn an_enabled_rule_with_an_unspecified_time_range_gives_no_access() {
    // The placeholder an index-0 write grows (Clause 12.34.9.3), once
    // enabled and given a location but with its time range left unspecified.
    let mut db = database();
    let placeholder = during(present_value(schedule(UNSPECIFIED)), Some(point(1)));
    add_rights(&mut db, 1, vec![placeholder], vec![]);
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), no_rights());
}

#[test]
fn time_range_references_naming_this_device_are_read() {
    let mut here = present_value(schedule(1));
    here.device_identifier = Some(device(DEVICE));
    assert_eq!(judge(with_true_schedule(), here), Judged::Holds);
}

/// A database holding Binary Value 1, commanded ACTIVE at priority 8.
fn with_commanded_binary_value() -> ObjectDatabase {
    let mut db = database();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    let origin = CommandOrigin::Local {
        owner_device: device(DEVICE),
        initiating_object: None,
    };
    db.get_mut(&oid(ObjectType::BINARY_VALUE, 1))
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(BinaryPV::ACTIVE.to_raw()),
            Some(8),
            &origin,
        )
        .unwrap();
    db
}

#[test]
fn time_range_honours_the_array_index() {
    let binary = oid(ObjectType::BINARY_VALUE, 1);
    let slot = |index: u32| {
        BACnetDeviceObjectPropertyReference::new_local(
            binary,
            PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
        )
        .with_index(index)
    };
    // Slot 8 holds ACTIVE; slot 9 is NULL; 17 is past the end.
    assert_eq!(judge(with_commanded_binary_value(), slot(8)), Judged::Holds);
    assert_eq!(judge(with_commanded_binary_value(), slot(9)), Judged::Never);
    assert_eq!(
        judge(with_commanded_binary_value(), slot(17)),
        Judged::Never
    );
    // Present_Value is ACTIVE, but takes no index.
    let value = present_value(binary);
    assert_eq!(
        judge(with_commanded_binary_value(), value.clone()),
        Judged::Holds
    );
    let indexed = value.with_index(1);
    assert_eq!(judge(with_commanded_binary_value(), indexed), Judged::Never);
}

#[test]
fn index_zero_is_no_time_range_value() {
    // Index 0 reads the array's size, 16 here: an Unsigned that would read
    // TRUE, but a size says nothing about time.
    let size = BACnetDeviceObjectPropertyReference::new_local(
        oid(ObjectType::BINARY_VALUE, 1),
        PropertyIdentifier::PRIORITY_ARRAY.to_raw(),
    )
    .with_index(0);
    assert_eq!(judge(with_commanded_binary_value(), size), Judged::Never);
}

/// The decision at `at` for a positive rule, enabled at any time, whose
/// location is `location`.
fn grants_at(location: BACnetDeviceObjectReference, at: u32) -> bool {
    let mut db = database();
    add_rights(
        &mut db,
        1,
        vec![BACnetAccessRule::new(None, Some(location), true)],
        vec![],
    );
    add_credential(&mut db, &[rights(1).into()]);
    let decision = evaluate_at(&db, at).decision;
    match decision {
        AccessRightsDecision::Granted { .. } => true,
        other => {
            assert_eq!(other, no_rights());
            false
        }
    }
}

#[test]
fn a_point_location_covers_that_point_only() {
    assert!(grants_at(point(1).into(), 1));
    assert!(!grants_at(point(1).into(), 2));
    assert!(grants_at(in_device(DEVICE, point(1)), 1));
    // The wildcard Device names no device in particular.
    assert!(!grants_at(in_device(UNSPECIFIED, point(1)), 1));
    assert!(!grants_at(in_device(OTHER_DEVICE, point(1)), 1));
    // Unspecified, alone or with the wildcard device.
    assert!(!grants_at(point(UNSPECIFIED).into(), 1));
    assert!(!grants_at(in_device(UNSPECIFIED, point(UNSPECIFIED)), 1));
}

#[test]
fn a_zone_location_covers_its_entry_points() {
    // Zone 1 is entered through Point 1 and left through Point 2.
    assert!(grants_at(zone(1).into(), 1));
    assert!(!grants_at(zone(1).into(), 2));
    assert!(grants_at(in_device(DEVICE, zone(1)), 1));
    assert!(!grants_at(in_device(OTHER_DEVICE, zone(1)), 1));
    assert!(!grants_at(in_device(UNSPECIFIED, zone(1)), 1));
    // No Zone 2 here.
    assert!(!grants_at(zone(2).into(), 1));
    assert!(!grants_at(zone(UNSPECIFIED).into(), 1));
}

#[test]
fn a_zone_entry_point_must_name_the_point_in_this_device() {
    for (entry, covers) in [
        (in_device(DEVICE, point(1)), true),
        (in_device(UNSPECIFIED, point(1)), false),
        (in_device(OTHER_DEVICE, point(1)), false),
    ] {
        let mut db = database();
        let mut hall = AccessZoneObject::new(2, "AZ-2").unwrap();
        hall.set_entry_points([entry.clone()]).unwrap();
        db.add(Box::new(hall)).unwrap();
        add_rights(&mut db, 1, vec![anytime(Some(zone(2)))], vec![]);
        add_credential(&mut db, &[rights(1).into()]);
        let expected = if covers { granted(1, 1) } else { no_rights() };
        assert_eq!(decision(&db), expected, "{entry:?}");
    }
}

/// A zone that counts its Entry_Points reads.
struct CountedZone {
    zone: AccessZoneObject,
    reads: Arc<AtomicUsize>,
}

impl BACnetObject for CountedZone {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.zone.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.zone.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == PropertyIdentifier::ENTRY_POINTS {
            self.reads.fetch_add(1, Ordering::Relaxed);
        }
        self.zone.read_property(property, array_index)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.zone
            .write_property(property, array_index, value, priority)
    }

    fn property_list(&self) -> std::borrow::Cow<'static, [PropertyIdentifier]> {
        self.zone.property_list()
    }
}

#[test]
fn a_zone_is_read_once_per_evaluation() {
    let mut db = database();
    let mut hall = AccessZoneObject::new(2, "AZ-2").unwrap();
    hall.set_entry_points([point(1)]).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    db.add(Box::new(CountedZone {
        zone: hall,
        reads: reads.clone(),
    }))
    .unwrap();
    db.add(Box::new(
        ScheduleObject::new(1, "SCHED-1", PropertyValue::Boolean(false)).unwrap(),
    ))
    .unwrap();
    // Three rules name the zone, none of them holding.
    let shut = present_value(schedule(1));
    add_rights(
        &mut db,
        1,
        vec![
            during(shut.clone(), Some(zone(2))),
            during(shut.clone(), Some(zone(2))),
        ],
        vec![during(shut, Some(zone(2)))],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), out_of_time(1, 1));
    assert_eq!(reads.load(Ordering::Relaxed), 1);
}

#[test]
fn all_covers_every_point_whatever_its_reference() {
    assert!(grants_at_with(anytime(None), 1));
    assert!(grants_at_with(anytime(None), 2));
    // ALL with a reference to Point 2: the reference is ignored.
    let mut rule = anytime(None);
    rule.location = Some(point(2).into());
    assert!(grants_at_with(rule, 1));
}

#[test]
fn malformed_locations_cover_nothing() {
    // The built-in object refuses each of these, so an application's own
    // object serves them.
    let mut other_type = anytime(None);
    other_type.location_specifier = AccessRuleLocationSpecifier::SPECIFIED;
    other_type.location = Some(oid(ObjectType::ACCESS_DOOR, 1).into());
    let mut missing = anytime(None);
    missing.location_specifier = AccessRuleLocationSpecifier::SPECIFIED;
    let mut unknown = anytime(Some(point(1)));
    unknown.location_specifier = AccessRuleLocationSpecifier::from_raw(2);
    for rule in [other_type, missing, unknown] {
        assert!(!grants_at_with(rule.clone(), 1), "{rule:?}");
    }
}

/// Whether `rule`, the only positive rule, served by an application's own
/// Access Rights object, grants at `at`.
fn grants_at_with(rule: BACnetAccessRule, at: u32) -> bool {
    let mut db = database();
    db.add(stub_rights(1, PropertyValue::Boolean(true), &[rule]))
        .unwrap();
    add_credential(&mut db, &[rights(1).into()]);
    let decision = evaluate_at(&db, at).decision;
    match decision {
        AccessRightsDecision::Granted { .. } => true,
        other => {
            assert_eq!(other, no_rights());
            false
        }
    }
}
