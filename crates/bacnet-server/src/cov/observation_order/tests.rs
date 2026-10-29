use super::*;
use crate::cov::{CovNotificationKind, CovSample, CovSubscription};
use bacnet_types::{
    MacAddr,
    enums::{ObjectType, PropertyIdentifier},
    primitives::{ObjectIdentifier, PropertyValue},
};
use std::time::{Duration, Instant};
fn proposal(kind: CovNotificationKind, confirmed: bool) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[1]),
        subscriber_network: None,
        subscriber_process_identifier: 1,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        issue_confirmed_notifications: confirmed,
        expires_at: Some(Instant::now() + Duration::from_secs(60)),
        last_notified_observation: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: kind,
        timestamped: false,
    }
}
fn value(v: f32) -> CovObservation {
    CovObservation::new(CovSample::new(&PropertyValue::Real(v)).unwrap(), None).unwrap()
}
#[test]
fn cov_order_counter_checked_exhaustion_covers_both_modes() {
    let mut table = CovSubscriptionTable::new();
    let sub = table
        .subscribe(proposal(CovNotificationKind::Single, false))
        .unwrap();
    let mut other = proposal(CovNotificationKind::Single, true);
    other.subscriber_process_identifier = 2;
    let confirmed = table.subscribe(other).unwrap();
    let first = confirmed.prepare_completion().unwrap();
    assert!(
        matches!(first, PreparedCovCompletion::Confirmed(ticket) if ticket.get() == 1),
        "confirmed reports draw from the same counter (#896)"
    );
    table.owner.issued.store(u64::MAX - 1, Ordering::Relaxed);
    let final_ticket = sub.prepare_completion().unwrap();
    assert!(table.complete_observation(&sub, final_ticket, value(10.0)));
    assert_eq!(
        table
            .get_subscription(sub.key())
            .unwrap()
            .last_successful_ticket,
        u64::MAX
    );
    assert!(sub.prepare_completion().is_none());
    assert!(confirmed.prepare_completion().is_none());
    assert_eq!(table.owner.issued.load(Ordering::Relaxed), u64::MAX);
    assert_eq!(
        table
            .get_subscription(sub.key())
            .unwrap()
            .last_notified_observation,
        Some(value(10.0))
    );
    let flight = table.begin_confirmed(first, [&confirmed]).unwrap();
    assert!(
        !table.complete_observation(&confirmed, final_ticket, value(30.0)),
        "mode mismatch fails closed"
    );
    assert!(table.complete_observation(&confirmed, first, value(20.0)));
    drop(flight);
    assert_eq!(
        table
            .get_subscription(confirmed.key())
            .unwrap()
            .last_notified_observation,
        Some(value(20.0))
    );
}
#[test]
fn cov_order_successful_marker_is_live_per_reference_and_resets_on_replacement() {
    let mut table = CovSubscriptionTable::new();
    let p = proposal(CovNotificationKind::Single, false);
    let first = table.subscribe(p.clone()).unwrap();
    let older = first.prepare_completion().unwrap();
    let newer = first.prepare_completion().unwrap();
    assert!(table.complete_observation(&first, newer, value(20.0)));
    assert!(!table.complete_observation(&first, older, value(10.0)));
    assert!(
        !table.complete_observation(&first, newer, value(30.0)),
        "duplicate success cannot republish"
    );
    let clone = first.clone(); // Its copied successful marker remains stale/empty.
    let latest = clone.prepare_completion().unwrap();
    assert!(table.complete_observation(&clone, latest, value(40.0)));
    assert_eq!(
        table
            .get_subscription(first.key())
            .unwrap()
            .last_notified_observation,
        Some(value(40.0))
    );
    let replacement = table.subscribe(p).unwrap();
    assert_eq!(replacement.last_successful_ticket, 0);
    assert!(!table.complete_observation(&first, latest, value(50.0)));
    assert!(table.complete_observation(
        &replacement,
        replacement.prepare_completion().unwrap(),
        value(60.0)
    ));
    let mut other_proposal = proposal(CovNotificationKind::Single, false);
    other_proposal.subscriber_process_identifier = 2;
    let sibling = table.subscribe(other_proposal).unwrap();
    let a = sibling.prepare_completion().unwrap();
    let b = replacement.prepare_completion().unwrap();
    assert!(table.complete_observation(&replacement, b, value(70.0)));
    assert!(
        table.complete_observation(&sibling, a, value(5.0)),
        "other references do not share successful progress"
    );
    let mut foreign = CovSubscriptionTable::new();
    let foreign_sub = foreign
        .subscribe(proposal(CovNotificationKind::Single, false))
        .unwrap();
    assert!(!table.complete_observation(
        &foreign_sub,
        foreign_sub.prepare_completion().unwrap(),
        value(100.0)
    ));
}
#[test]
fn cov_order_multiple_expiry_refresh_retains_progress_and_route_change_fences() {
    let mut table = CovSubscriptionTable::new();
    let mut p = proposal(CovNotificationKind::Multiple, false);
    p.subscriber_network = Some(bacnet_encoding::npdu::NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[9]),
    });
    let first = table.admit_for_test(p.clone(), 0).unwrap();
    let older = first.prepare_completion().unwrap();
    let newer = first.prepare_completion().unwrap();
    assert!(table.complete_observation(&first, newer, value(20.0)));
    let context = first.key().multiple_context().unwrap().clone();
    table
        .subscribe_multiple(
            &context,
            &first.endpoint(),
            Instant::now() + Duration::from_secs(120),
            5,
            None,
            vec![],
        )
        .unwrap();
    assert!(!table.complete_observation(&first, older, value(10.0)));
    assert_eq!(
        table
            .get_subscription(first.key())
            .unwrap()
            .last_notified_observation,
        Some(value(20.0))
    );
    let issued = first.prepare_completion().unwrap();
    let mut route = first.endpoint();
    route.mac = MacAddr::from_slice(&[2]);
    table
        .subscribe_multiple(
            &context,
            &route,
            Instant::now() + Duration::from_secs(120),
            5,
            None,
            vec![],
        )
        .unwrap();
    assert!(!table.complete_observation(&first, issued, value(30.0)));
    let moved = table.get_subscription(first.key()).unwrap().clone();
    assert!(table.complete_observation(&moved, moved.prepare_completion().unwrap(), value(40.0)));
    assert!(table.unsubscribe(moved.key()));
    assert!(!table.complete_observation(&moved, moved.prepare_completion().unwrap(), value(50.0)));
    p.subscriber_mac = route.mac;
    let recreated = table.admit_for_test(p, 0).unwrap();
    assert_eq!(recreated.last_successful_ticket, 0);
    assert!(!table.complete_observation(&moved, moved.prepare_completion().unwrap(), value(50.0)));
}
