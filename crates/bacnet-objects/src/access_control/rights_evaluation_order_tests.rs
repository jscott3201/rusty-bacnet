//! Which rule and which object the Access Rights check names when several
//! could (#1331), and the error it passes back when a credential's
//! Assigned_Access_Rights can't be read.

use bacnet_types::enums::{ErrorClass, ErrorCode};

use super::tests::*;
use super::*;

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
