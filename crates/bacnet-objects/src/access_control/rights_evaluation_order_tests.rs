//! Which rule and which object the Access Rights check names when several
//! could (#1331), negative rules that can't be judged here, and the error
//! the check passes back when a credential's Assigned_Access_Rights can't be
//! read.

use bacnet_types::enums::{ErrorClass, ErrorCode};

use super::tests::*;
use super::*;
use crate::schedule::ScheduleObject;

fn point_denial(rights_instance: u32, index: u32) -> AccessRightsDecision {
    denied(
        AccessEvent::DENIED_POINT_NO_ACCESS_RIGHTS,
        Some(position(rights_instance, AccessRuleKind::Negative, index)),
    )
}

#[test]
fn a_grant_after_an_out_of_time_rule_names_the_grant() {
    // Rule 1 covers Point 1 but is outside its time range; rule 2 holds.
    let mut db = database();
    let closed = add_closed_schedule(&mut db);
    add_rights(
        &mut db,
        1,
        vec![during(closed, Some(point(1))), anytime(Some(point(1)))],
        vec![],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), granted(1, 2));
}

#[test]
fn a_disabled_rule_keeps_its_place_in_the_array() {
    // The index reported is the rule's array index, disabled rules counted.
    let mut db = database();
    add_rights(
        &mut db,
        1,
        vec![disabled(anytime(None)), anytime(Some(point(1)))],
        vec![disabled(anytime(None)), anytime(Some(point(2)))],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), granted(1, 2));
    assert_eq!(evaluate_at(&db, 2).decision, point_denial(1, 2));
}

#[test]
fn objects_are_tried_in_assignment_order() {
    // Both objects grant; Access Rights 2, assigned first, names the grant.
    let mut db = database();
    add_rights(&mut db, 1, vec![anytime(None)], vec![]);
    add_rights(
        &mut db,
        2,
        vec![disabled(anytime(None)), anytime(None)],
        vec![],
    );
    add_credential(&mut db, &[rights(2).into(), rights(1).into()]);
    assert_eq!(decision(&db), granted(2, 2));

    // Both objects deny, one for the zone and one for the point; whichever
    // is assigned first names the denial.
    for (order, expected) in [
        ([2, 1], point_denial(2, 1)),
        (
            [1, 2],
            denied(
                AccessEvent::DENIED_ZONE_NO_ACCESS_RIGHTS,
                Some(position(1, AccessRuleKind::Negative, 1)),
            ),
        ),
    ] {
        let mut db = database();
        add_rights(&mut db, 1, vec![], vec![anytime(Some(zone(1)))]);
        add_rights(&mut db, 2, vec![], vec![anytime(Some(point(1)))]);
        let assigned: Vec<_> = order.iter().map(|&i| rights(i).into()).collect();
        add_credential(&mut db, &assigned);
        assert_eq!(decision(&db), expected, "{order:?}");
    }
}

#[test]
fn an_unreadable_assignment_list_is_passed_back() {
    // An application's own credential that serves no Assigned_Access_Rights,
    // or serves something that isn't a list of assignments.
    for (served, expected) in [
        (None, ErrorCode::UNKNOWN_PROPERTY),
        (
            Some(PropertyValue::Unsigned(0)),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        let mut db = database();
        let values = served
            .map(|value| vec![(PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS, value)])
            .unwrap_or_default();
        db.add(Stub::new(credential(1), values)).unwrap();
        match evaluate_access_rights(&db, credential(1), point(1)) {
            Err(Error::Protocol { class, code }) => {
                assert_eq!(class, ErrorClass::PROPERTY.to_raw() as u32);
                assert_eq!(code, expected.to_raw() as u32, "{expected:?}");
            }
            other => panic!("expected PROPERTY / {expected:?}, got {other:?}"),
        }
    }
}

#[test]
fn a_negative_rule_with_a_time_range_never_true_bars_no_one() {
    // An unspecified time range, and one reading NULL: FALSE at every
    // moment, so neither negative rule holds and the positive rule grants.
    let mut db = database();
    db.add(Box::new(
        ScheduleObject::new(60, "EMPTY", PropertyValue::Null).unwrap(),
    ))
    .unwrap();
    let unspecified = present_value(oid(ObjectType::SCHEDULE, UNSPECIFIED));
    let null = present_value(oid(ObjectType::SCHEDULE, 60));
    add_rights(
        &mut db,
        1,
        vec![anytime(None)],
        vec![
            during(unspecified, Some(point(1))),
            during(null, Some(point(1))),
        ],
    );
    add_credential(&mut db, &[rights(1).into()]);
    assert_eq!(decision(&db), granted(1, 1));
}

#[test]
fn a_negative_rule_naming_what_is_not_read_here_bars_no_one() {
    // Clause 12.34.9.1 has a reference that is unspecified or can't be
    // retrieved evaluate to FALSE, for a negative rule too: a location or
    // time range behind the wildcard Device or in another device doesn't
    // bar, and the ALL positive rule grants.
    let mut remote_time = present_value(oid(ObjectType::SCHEDULE, 1));
    remote_time.device_identifier = Some(device(OTHER_DEVICE));
    let mut wildcard_time = remote_time.clone();
    wildcard_time.device_identifier = Some(device(UNSPECIFIED));
    let negatives = [
        BACnetAccessRule::new(None, Some(in_device(UNSPECIFIED, point(1))), true),
        BACnetAccessRule::new(None, Some(in_device(OTHER_DEVICE, point(1))), true),
        BACnetAccessRule::new(None, Some(in_device(UNSPECIFIED, zone(1))), true),
        BACnetAccessRule::new(Some(wildcard_time), Some(point(1).into()), true),
        BACnetAccessRule::new(Some(remote_time), Some(point(1).into()), true),
    ];
    for negative in negatives {
        // Schedule 1 here reads TRUE, so only the device member keeps the
        // time-range rules from holding.
        let mut db = database();
        let open = ScheduleObject::new(1, "OPEN", PropertyValue::Boolean(true)).unwrap();
        db.add(Box::new(open)).unwrap();
        add_rights(&mut db, 1, vec![anytime(None)], vec![negative.clone()]);
        add_credential(&mut db, &[rights(1).into()]);
        assert_eq!(decision(&db), granted(1, 1), "{negative:?}");
    }
}
