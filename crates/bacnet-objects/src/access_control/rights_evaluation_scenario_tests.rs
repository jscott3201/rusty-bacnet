//! End to end (#1331): a Schedule's Present_Value opens a zone during office
//! hours, a Binary Value's bars its entry point during a lockdown, and the
//! decision follows each value as it changes. The schedule runs at hand-set
//! times, so no test waits.

use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::BACnetTimeValue;
use bacnet_types::enums::BinaryPV;
use bacnet_types::primitives::Time;

use super::tests::*;
use super::*;
use crate::access_control::AccessRightsObject;
use crate::binary::BinaryValueObject;
use crate::command_source::CommandOrigin;
use crate::schedule::ScheduleObject;

fn hours() -> ObjectIdentifier {
    oid(ObjectType::SCHEDULE, 1)
}

fn lockdown() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 1)
}

fn at(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

/// Run the office-hours schedule's pass on Monday 14 September 2026 at
/// `time`, as the server's schedule task would.
fn run_schedule(db: &mut ObjectDatabase, time: Time) {
    let monday = SpecificDate::new(2026, 9, 14).unwrap();
    db.get_mut(&hours())
        .unwrap()
        .tick_schedule(monday, time, &|_| false);
}

/// Command the lockdown value at priority 8 (`Some`), or relinquish it.
fn command_lockdown(db: &mut ObjectDatabase, value: Option<BinaryPV>) {
    let origin = CommandOrigin::Local {
        owner_device: device(DEVICE),
        initiating_object: None,
    };
    let value = value.map_or(PropertyValue::Null, |pv| {
        PropertyValue::Enumerated(pv.to_raw())
    });
    db.get_mut(&lockdown())
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value,
            Some(8),
            &origin,
        )
        .unwrap();
}

fn present_value_of(db: &ObjectDatabase, object: ObjectIdentifier) -> PropertyValue {
    db.get(&object)
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
}

/// Device 100; Access Points 1 and 2; Access Zone 1 entered through Point 1
/// and left through Point 2; office hours from 08:00 to 17:00 on Mondays;
/// a lockdown Binary Value, INACTIVE; and Credential 1 holding Access
/// Rights 1, which lets it into Zone 1 during office hours and bars Point 1
/// while the lockdown is ACTIVE.
fn site() -> ObjectDatabase {
    let mut db = database();
    let mut office = ScheduleObject::new(1, "OFFICE-HOURS", PropertyValue::Boolean(false)).unwrap();
    office
        .set_weekly_schedule(
            0,
            vec![
                BACnetTimeValue {
                    time: at(8, 0),
                    value: PropertyValue::Boolean(true),
                },
                BACnetTimeValue {
                    time: at(17, 0),
                    value: PropertyValue::Boolean(false),
                },
            ],
        )
        .unwrap();
    db.add(Box::new(office)).unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "LOCKDOWN").unwrap()))
        .unwrap();

    let mut staff = AccessRightsObject::new(1, "STAFF").unwrap();
    staff
        .set_positive_access_rules([BACnetAccessRule::new(
            Some(present_value(hours())),
            Some(in_device(DEVICE, zone(1))),
            true,
        )])
        .unwrap();
    staff
        .set_negative_access_rules([BACnetAccessRule::new(
            Some(present_value(lockdown())),
            Some(point(1).into()),
            true,
        )])
        .unwrap();
    db.add(Box::new(staff)).unwrap();
    add_credential(&mut db, &[rights(1).into()]);
    db
}

#[test]
fn the_decision_follows_the_schedule_and_the_lockdown() {
    let mut db = site();
    let open = granted(1, 1);
    let closed = out_of_time(1, 1);
    let locked = denied(
        AccessEvent::DENIED_POINT_NO_ACCESS_RIGHTS,
        Some(position(1, AccessRuleKind::Negative, 1)),
    );

    // Before its first pass the schedule holds its default, FALSE.
    assert_eq!(decision(&db), closed);

    run_schedule(&mut db, at(9, 0));
    assert_eq!(present_value_of(&db, hours()), PropertyValue::Boolean(true));
    assert_eq!(decision(&db), open);
    // Point 2 leads out of the zone, so the rule never covers it.
    assert_eq!(evaluate_at(&db, 2).decision, no_rights());

    // The lockdown's negative rule wins over the open zone.
    command_lockdown(&mut db, Some(BinaryPV::ACTIVE));
    assert_eq!(decision(&db), locked);
    command_lockdown(&mut db, None);
    assert_eq!(decision(&db), open);

    run_schedule(&mut db, at(17, 30));
    assert_eq!(
        present_value_of(&db, hours()),
        PropertyValue::Boolean(false)
    );
    assert_eq!(decision(&db), closed);
    // Locked down after hours: the negative rule still comes first.
    command_lockdown(&mut db, Some(BinaryPV::ACTIVE));
    assert_eq!(decision(&db), locked);

    let evaluation = evaluate_at(&db, 1);
    assert!(evaluation.unresolved.is_empty());
}
