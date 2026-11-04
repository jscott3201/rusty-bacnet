use super::*;
use crate::cov::{CovNotificationKind, CovSample};
use crate::cov::{CovSubscription, Delivery};
use alloc::vec;
use bacnet_time::Duration;
use bacnet_types::{
    MacAddr,
    enums::{ObjectType, PropertyIdentifier},
    primitives::{ObjectIdentifier, PropertyValue},
};

const NOW: Instant = Instant::from_secs(1_000);
fn proposal(kind: CovNotificationKind, confirmed: bool) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[1]),
        subscriber_network: None,
        subscriber_process_identifier: 1,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        issue_confirmed_notifications: confirmed,
        expires_at: Some(NOW + Duration::from_secs(60)),
        last_notified_observation: None,
        monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: kind,
        timestamped: false,
    }
}
/// Admit one Multiple reference as a one-reference request.
fn admit(table: &mut CovTable, sub: CovSubscription) -> CovSubscriptionSnapshot {
    let context = sub.key().unwrap().multiple_context().unwrap().clone();
    let (route, expires) = (sub.endpoint(), sub.expires_at.unwrap());
    let mut accepted = table
        .subscribe_multiple(&context, &route, expires, 0, None, vec![sub], NOW)
        .unwrap();
    accepted.remove(0)
}
fn value(v: f32) -> CovObservation {
    CovObservation::new(CovSample::new(&PropertyValue::Real(v)).unwrap(), None).unwrap()
}
#[test]
fn cov_order_counter_checked_exhaustion_covers_both_modes() {
    let mut table = CovTable::default();
    let sub = table
        .subscribe(proposal(CovNotificationKind::Single, false), NOW)
        .unwrap();
    let mut other = proposal(CovNotificationKind::Single, true);
    other.subscriber_process_identifier = 2;
    let confirmed = table.subscribe(other, NOW).unwrap();
    let first = table.prepare_completion(&confirmed).unwrap();
    assert!(
        matches!(first, PreparedCovCompletion::Confirmed(ticket) if ticket.get() == 1),
        "confirmed reports draw from the same counter (#896)"
    );
    table.issued = u64::MAX - 1;
    let final_ticket = table.prepare_completion(&sub).unwrap();
    assert!(table.complete_observation(&sub, final_ticket, value(10.0), NOW));
    assert_eq!(
        table
            .get_subscription(sub.key())
            .unwrap()
            .last_successful_ticket,
        u64::MAX
    );
    assert!(table.prepare_completion(&sub).is_none());
    assert!(table.prepare_completion(&confirmed).is_none());
    assert_eq!(table.issued, u64::MAX);
    assert_eq!(
        table
            .get_subscription(sub.key())
            .unwrap()
            .last_notified_observation,
        Some(value(10.0))
    );
    let flight = table
        .begin_confirmed(first, [(&confirmed, value(0.0))], NOW)
        .unwrap();
    assert!(
        !table.complete_observation(&confirmed, final_ticket, value(30.0), NOW),
        "mode mismatch fails closed"
    );
    assert!(table.complete_observation(&confirmed, first, value(20.0), NOW));
    table.delivered(flight, Delivery::NotSent, NOW);
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
    let mut table = CovTable::default();
    let p = proposal(CovNotificationKind::Single, false);
    let first = table.subscribe(p.clone(), NOW).unwrap();
    let older = table.prepare_completion(&first).unwrap();
    let newer = table.prepare_completion(&first).unwrap();
    assert!(table.complete_observation(&first, newer, value(20.0), NOW));
    assert!(!table.complete_observation(&first, older, value(10.0), NOW));
    assert!(
        !table.complete_observation(&first, newer, value(30.0), NOW),
        "duplicate success cannot republish"
    );
    let clone = first.clone(); // Its copied successful marker remains stale/empty.
    let latest = table.prepare_completion(&clone).unwrap();
    assert!(table.complete_observation(&clone, latest, value(40.0), NOW));
    assert_eq!(
        table
            .get_subscription(first.key())
            .unwrap()
            .last_notified_observation,
        Some(value(40.0))
    );
    let replacement = table.subscribe(p, NOW).unwrap();
    assert_eq!(replacement.last_successful_ticket, 0);
    assert!(!table.complete_observation(&first, latest, value(50.0), NOW));
    let ticket = table.prepare_completion(&replacement).unwrap();
    assert!(table.complete_observation(&replacement, ticket, value(60.0), NOW));
    let mut other_proposal = proposal(CovNotificationKind::Single, false);
    other_proposal.subscriber_process_identifier = 2;
    let sibling = table.subscribe(other_proposal, NOW).unwrap();
    let a = table.prepare_completion(&sibling).unwrap();
    let b = table.prepare_completion(&replacement).unwrap();
    assert!(table.complete_observation(&replacement, b, value(70.0), NOW));
    assert!(
        table.complete_observation(&sibling, a, value(5.0), NOW),
        "other references do not share successful progress"
    );
    let mut foreign = CovTable::default();
    let foreign_sub = foreign
        .subscribe(proposal(CovNotificationKind::Single, false), NOW)
        .unwrap();
    assert!(!table.complete_observation(
        &foreign_sub,
        foreign.prepare_completion(&foreign_sub).unwrap(),
        value(100.0),
        NOW
    ));
}
#[test]
fn cov_order_multiple_expiry_refresh_retains_progress_and_route_change_fences() {
    let mut table = CovTable::default();
    let mut p = proposal(CovNotificationKind::Multiple, false);
    p.subscriber_network = Some(bacnet_encoding::npdu::NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[9]),
    });
    let first = admit(&mut table, p.clone());
    let older = table.prepare_completion(&first).unwrap();
    let newer = table.prepare_completion(&first).unwrap();
    assert!(table.complete_observation(&first, newer, value(20.0), NOW));
    let context = first.key().multiple_context().unwrap().clone();
    table
        .subscribe_multiple(
            &context,
            &first.endpoint(),
            NOW + Duration::from_secs(120),
            5,
            None,
            vec![],
            NOW,
        )
        .unwrap();
    assert!(!table.complete_observation(&first, older, value(10.0), NOW));
    assert_eq!(
        table
            .get_subscription(first.key())
            .unwrap()
            .last_notified_observation,
        Some(value(20.0))
    );
    let issued = table.prepare_completion(&first).unwrap();
    let mut route = first.endpoint();
    route.mac = MacAddr::from_slice(&[2]);
    table
        .subscribe_multiple(
            &context,
            &route,
            NOW + Duration::from_secs(120),
            5,
            None,
            vec![],
            NOW,
        )
        .unwrap();
    assert!(!table.complete_observation(&first, issued, value(30.0), NOW));
    let moved = table.get_subscription(first.key()).unwrap().clone();
    let ticket = table.prepare_completion(&moved).unwrap();
    assert!(table.complete_observation(&moved, ticket, value(40.0), NOW));
    assert!(table.unsubscribe(moved.key()));
    let ticket = table.prepare_completion(&moved).unwrap();
    assert!(!table.complete_observation(&moved, ticket, value(50.0), NOW));
    p.subscriber_mac = route.mac;
    let recreated = admit(&mut table, p);
    assert_eq!(recreated.last_successful_ticket, 0);
    let ticket = table.prepare_completion(&moved).unwrap();
    assert!(!table.complete_observation(&moved, ticket, value(50.0), NOW));
}
