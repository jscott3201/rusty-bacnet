//! The two terms of an access rule (#1331, Clause 12.34.9.1): how each
//! time-range value reads as TRUE or FALSE, the references that read FALSE,
//! and which locations cover a point.

use bacnet_types::enums::BinaryPV;

use super::tests::*;
use super::*;
use crate::access_control::AccessZoneObject;
use crate::binary::BinaryValueObject;
use crate::command_source::CommandOrigin;
use crate::schedule::ScheduleObject;

fn schedule(instance: u32) -> ObjectIdentifier {
    oid(ObjectType::SCHEDULE, instance)
}

/// Whether a positive rule for Point 1 whose time range is `time_range`
/// grants at Point 1, in `db`; a FALSE time range is out of time range.
fn holds_in(mut db: ObjectDatabase, time_range: BACnetDeviceObjectPropertyReference) -> bool {
    add_rights(&mut db, 1, vec![during(time_range, Some(point(1)))], vec![]);
    add_credential(&mut db, &[rights(1).into()]);
    match decision(&db) {
        AccessRightsDecision::Granted { rule } => {
            assert_eq!(rule, position(1, AccessRuleKind::Positive, 1));
            true
        }
        other => {
            assert_eq!(other, out_of_time(1, 1));
            false
        }
    }
}

/// Whether Schedule 1's Present_Value, holding `value`, reads as TRUE.
fn reads_true(value: PropertyValue) -> bool {
    let mut db = database();
    db.add(Box::new(ScheduleObject::new(1, "SCHED-1", value).unwrap()))
        .unwrap();
    holds_in(db, present_value(schedule(1)))
}

#[test]
fn time_range_values_named_by_the_clause() {
    assert!(reads_true(PropertyValue::Boolean(true)));
    assert!(!reads_true(PropertyValue::Boolean(false)));
    assert!(reads_true(PropertyValue::Unsigned(1)));
    assert!(reads_true(PropertyValue::Unsigned(u64::MAX)));
    assert!(!reads_true(PropertyValue::Unsigned(0)));
    assert!(reads_true(PropertyValue::Signed(1)));
    assert!(!reads_true(PropertyValue::Signed(0)));
    assert!(!reads_true(PropertyValue::Signed(-1)));
    assert!(reads_true(PropertyValue::Enumerated(
        BinaryPV::ACTIVE.to_raw()
    )));
    assert!(!reads_true(PropertyValue::Enumerated(
        BinaryPV::INACTIVE.to_raw()
    )));
    assert!(!reads_true(PropertyValue::Null));
}

#[test]
fn time_range_values_the_clause_leaves_to_the_device_read_false() {
    // Every Enumerated reads as a BACnetBinaryPV, so another number is FALSE.
    assert!(!reads_true(PropertyValue::Enumerated(2)));
    assert!(!reads_true(PropertyValue::Real(1.0)));
    assert!(!reads_true(PropertyValue::Double(1.0)));
    assert!(!reads_true(PropertyValue::CharacterString("on".into())));
    assert!(!reads_true(PropertyValue::OctetString(vec![1])));
    assert!(!reads_true(PropertyValue::ObjectIdentifier(point(1))));
}

#[test]
fn time_range_always_holds_and_an_unknown_specifier_never_does() {
    let mut db = database();
    add_rights(&mut db, 1, vec![anytime(Some(point(1)))], vec![]);
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), granted(1, 1));

    // Specifier 2 names neither SPECIFIED nor ALWAYS; the built-in object
    // refuses it, so an application's own object serves it.
    let mut rule = anytime(Some(point(1)));
    rule.time_range_specifier = AccessRuleTimeRangeSpecifier::from_raw(2);
    let mut db = database();
    db.add(stub_rights(1, PropertyValue::Boolean(true), &[rule]))
        .unwrap();
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), out_of_time(1, 1));

    // SPECIFIED without a reference: nothing to read.
    let mut rule = anytime(Some(point(1)));
    rule.time_range_specifier = AccessRuleTimeRangeSpecifier::SPECIFIED;
    let mut db = database();
    db.add(stub_rights(1, PropertyValue::Boolean(true), &[rule]))
        .unwrap();
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), out_of_time(1, 1));
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
fn time_range_references_that_read_false() {
    let on = present_value(schedule(1));
    assert!(holds_in(with_true_schedule(), on.clone()));
    // Unspecified: instance 4194303, and the device too when named.
    assert!(!holds_in(
        with_true_schedule(),
        present_value(schedule(UNSPECIFIED))
    ));
    let mut unspecified_in_device = present_value(schedule(UNSPECIFIED));
    unspecified_in_device.device_identifier = Some(device(UNSPECIFIED));
    assert!(!holds_in(with_true_schedule(), unspecified_in_device));
    // A missing object, and a property the object doesn't serve.
    assert!(!holds_in(with_true_schedule(), present_value(schedule(2))));
    let mut unserved = on.clone();
    unserved.property_identifier = PropertyIdentifier::LOG_ENABLE.to_raw();
    assert!(!holds_in(with_true_schedule(), unserved));
    // Another device, even though Schedule 1 exists here.
    let mut remote = on.clone();
    remote.device_identifier = Some(device(OTHER_DEVICE));
    assert!(!holds_in(with_true_schedule(), remote));
}

#[test]
fn time_range_references_naming_this_device_are_read() {
    for device_instance in [DEVICE, UNSPECIFIED] {
        let mut here = present_value(schedule(1));
        here.device_identifier = Some(device(device_instance));
        assert!(holds_in(with_true_schedule(), here), "{device_instance}");
    }
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
    assert!(holds_in(with_commanded_binary_value(), slot(8)));
    assert!(!holds_in(with_commanded_binary_value(), slot(9)));
    assert!(!holds_in(with_commanded_binary_value(), slot(17)));
    // Present_Value is ACTIVE, but takes no index.
    assert!(holds_in(
        with_commanded_binary_value(),
        present_value(binary)
    ));
    assert!(!holds_in(
        with_commanded_binary_value(),
        present_value(binary).with_index(1)
    ));
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
    assert!(grants_at(in_device(UNSPECIFIED, point(1)), 1));
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
    // No Zone 2 here.
    assert!(!grants_at(zone(2).into(), 1));
    assert!(!grants_at(zone(UNSPECIFIED).into(), 1));
}

#[test]
fn a_zone_entry_point_must_name_the_point_in_this_device() {
    for (entry, covers) in [
        (in_device(DEVICE, point(1)), true),
        (in_device(UNSPECIFIED, point(1)), true),
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
