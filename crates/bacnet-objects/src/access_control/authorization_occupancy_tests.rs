//! Occupancy uses explicit direction and complete local topology/count evidence.
use super::authorization_test_support::*;
use super::*;

fn context(
    direction: AuthorizationDirection,
    enforcement: AuthorizationFact<bool>,
) -> [AuthorizationOccupancyZone; 1] {
    [AuthorizationOccupancyZone {
        zone: zone(),
        direction,
        fallback_enforced: enforcement,
    }]
}
#[test]
fn enforced_upper_limit_denies_at_equality_and_above() {
    for (count, permitted) in [(9, true), (10, false), (11, false)] {
        let mut db = database(None);
        add_zone(&mut db, count, 1, 10);
        let zones = context(
            AuthorizationDirection::Entering,
            AuthorizationFact::Known(true),
        );
        let mut req = request();
        req.occupancy = AuthorizationOccupancy::Zones(&zones);
        if permitted {
            granted(&eval(&db, req));
        } else {
            denied(&eval(&db, req), AccessEvent::DENIED_UPPER_OCCUPANCY_LIMIT);
        }
    }
}
#[test]
fn enforced_lower_limit_denies_at_equality_and_below() {
    for (count, permitted) in [(0, false), (1, false), (2, true)] {
        let mut db = database(None);
        add_zone(&mut db, count, 1, 10);
        let zones = context(
            AuthorizationDirection::Leaving,
            AuthorizationFact::Known(true),
        );
        let mut req = request();
        req.occupancy = AuthorizationOccupancy::Zones(&zones);
        if permitted {
            granted(&eval(&db, req));
        } else {
            denied(&eval(&db, req), AccessEvent::DENIED_LOWER_OCCUPANCY_LIMIT);
        }
    }
}
#[test]
fn zero_bounds_do_not_read_or_invent_a_count() {
    let mut db = database(None);
    add_zone(&mut db, 0, 0, 0);
    override_read(&mut db, zone(), P::OCCUPANCY_COUNT, Read::Forbidden, true);
    for direction in [
        AuthorizationDirection::Entering,
        AuthorizationDirection::Leaving,
    ] {
        let zones = context(direction, AuthorizationFact::Known(true));
        let mut req = request();
        req.occupancy = AuthorizationOccupancy::Zones(&zones);
        granted(&eval(&db, req));
    }
}
#[test]
fn unenforced_bound_and_explicit_inapplicability_do_not_need_count_evidence() {
    let db = database(None);
    granted(&eval(&db, request()));
    let zones = context(
        AuthorizationDirection::Entering,
        AuthorizationFact::Known(false),
    );
    let mut req = request();
    req.occupancy = AuthorizationOccupancy::Zones(&zones);
    granted(&eval(&db, req));
    req.occupancy = AuthorizationOccupancy::Unknown;
    unresolved(&eval(&db, req));
    req.occupancy = AuthorizationOccupancy::Zones(&[]);
    unresolved(&eval(&db, req));
}
#[test]
fn served_enforcement_overrides_fallback_and_failure_does_not_use_it() {
    let mut db = database(None);
    add_zone(&mut db, 10, 1, 10);
    let zones = context(
        AuthorizationDirection::Entering,
        AuthorizationFact::Known(false),
    );
    let mut req = request();
    req.occupancy = AuthorizationOccupancy::Zones(&zones);
    value(
        &mut db,
        point(),
        P::OCCUPANCY_UPPER_LIMIT_ENFORCED,
        PropertyValue::Boolean(true),
    );
    denied(&eval(&db, req), AccessEvent::DENIED_UPPER_OCCUPANCY_LIMIT);
    value(
        &mut db,
        point(),
        P::OCCUPANCY_UPPER_LIMIT_ENFORCED,
        PropertyValue::Null,
    );
    unresolved(&eval(&db, req));
}
#[test]
fn missing_enforcement_count_limit_or_topology_is_unresolved() {
    let zones = context(AuthorizationDirection::Entering, AuthorizationFact::Unknown);
    let mut db = database(None);
    add_zone(&mut db, 5, 1, 10);
    let mut req = request();
    req.occupancy = AuthorizationOccupancy::Zones(&zones);
    unresolved(&eval(&db, req));
    let zones = context(
        AuthorizationDirection::Entering,
        AuthorizationFact::Known(true),
    );
    req.occupancy = AuthorizationOccupancy::Zones(&zones);
    for property in [
        P::OCCUPANCY_COUNT,
        P::OCCUPANCY_UPPER_LIMIT,
        P::ENTRY_POINTS,
        P::OCCUPANCY_COUNT_ENABLE,
    ] {
        let mut db = database(None);
        add_zone(&mut db, 5, 1, 10);
        value(&mut db, zone(), property, PropertyValue::Null);
        unresolved(&eval(&db, req));
    }
}
#[test]
fn disabled_counting_is_unknown_even_when_served_count_is_zero() {
    let mut db = database(None);
    add_zone(&mut db, 0, 1, 10);
    value(
        &mut db,
        zone(),
        P::OCCUPANCY_COUNT_ENABLE,
        PropertyValue::Boolean(false),
    );
    let zones = context(
        AuthorizationDirection::Entering,
        AuthorizationFact::Known(true),
    );
    let mut req = request();
    req.occupancy = AuthorizationOccupancy::Zones(&zones);
    unresolved(&eval(&db, req));
}
#[test]
fn direction_and_local_point_zone_must_match_topology() {
    let mut db = database(None);
    add_zone(&mut db, 5, 1, 10);
    let zones = context(
        AuthorizationDirection::Entering,
        AuthorizationFact::Known(true),
    );
    let mut req = request();
    req.occupancy = AuthorizationOccupancy::Zones(&zones);
    value(
        &mut db,
        point(),
        P::ZONE_TO,
        encoded_reference(&zone().into()),
    );
    granted(&eval(&db, req));
    value(
        &mut db,
        point(),
        P::ZONE_TO,
        encoded_reference(&oid(ObjectType::ACCESS_ZONE, 2).into()),
    );
    unresolved(&eval(&db, req));
    value(
        &mut db,
        point(),
        P::ZONE_TO,
        encoded_reference(&zone().into()),
    );
    value(
        &mut db,
        zone(),
        P::ENTRY_POINTS,
        PropertyValue::List(vec![]),
    );
    unresolved(&eval(&db, req));
}
#[test]
fn occupancy_exemption_only_bypasses_occupancy() {
    let mut db = database(None);
    add_zone(&mut db, 10, 1, 10);
    let zones = context(
        AuthorizationDirection::Entering,
        AuthorizationFact::Known(true),
    );
    let mut req = request();
    req.occupancy = AuthorizationOccupancy::Zones(&zones);
    for raw in [0, 2, 3, 4, 5, 6] {
        exempt(&mut db, 1, &[Exemption::from_raw(raw)]);
        denied(&eval(&db, req), AccessEvent::DENIED_UPPER_OCCUPANCY_LIMIT);
    }
    exempt(&mut db, 1, &[Exemption::OCCUPANCY_CHECK]);
    granted(&eval(&db, req));
    req.fallback_lockout = AuthorizationFact::Known(true);
    denied(&eval(&db, req), AccessEvent::DENIED_LOCKOUT);
}
#[test]
fn every_relevant_zone_is_checked_and_duplicate_context_is_unresolved() {
    let mut db = database(None);
    add_zone(&mut db, 5, 1, 10);
    let mut second = AccessZoneObject::new(2, "second").unwrap();
    second.set_entry_points(vec![point()]).unwrap();
    second.set_occupancy_count_enable(true);
    second.set_occupancy_count(10);
    second.set_occupancy_limits(1, 10).unwrap();
    db.add(Box::new(second)).unwrap();
    let first = context(
        AuthorizationDirection::Entering,
        AuthorizationFact::Known(true),
    )[0];
    let zones = [
        first,
        AuthorizationOccupancyZone {
            zone: oid(ObjectType::ACCESS_ZONE, 2),
            ..first
        },
    ];
    let mut req = request();
    req.occupancy = AuthorizationOccupancy::Zones(&zones);
    denied(&eval(&db, req), AccessEvent::DENIED_UPPER_OCCUPANCY_LIMIT);
    let duplicates = [first, first];
    req.occupancy = AuthorizationOccupancy::Zones(&duplicates);
    unresolved(&eval(&db, req));
}
