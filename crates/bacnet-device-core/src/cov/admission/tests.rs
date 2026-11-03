//! Admission's refusals of indefinite subscriptions (#2317).
use super::*;
use crate::cov::{CovCounters, CovNotificationKind, CovPolicy};
use bacnet_types::MacAddr;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::ObjectIdentifier;

fn indefinite(process: u32) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[7]),
        subscriber_network: None,
        subscriber_process_identifier: process,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_observation: None,
        monitored_property: None,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    }
}

fn refusal(error: Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("expected an Error, got {other:?}"),
    }
}

/// A policy without indefinite subscriptions answers that the lifetime is
/// out of the device's range (Clause 13.14.1.3.1), and counts the refusal.
#[test]
fn an_indefinite_subscription_the_policy_refuses_is_out_of_range() {
    let mut table = CovTable::new(CovPolicy {
        allow_indefinite_subscriptions: false,
        ..CovPolicy::default()
    });
    let error = table.subscribe(indefinite(1), Instant::ZERO).unwrap_err();
    assert_eq!(
        refusal(error),
        (ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE)
    );
    assert!(table.is_empty());
    let counts = table.take_counts();
    assert_eq!(counts.subscriptions_rejected_indefinite, 1);
    assert_eq!(
        counts,
        CovCounters {
            subscriptions_rejected_indefinite: 1,
            ..CovCounters::default()
        }
    );
}

/// The per-peer indefinite cap is a resource limit.
#[test]
fn the_per_peer_indefinite_cap_has_no_space() {
    let mut table = CovTable::new(CovPolicy {
        max_indefinite_per_peer: 1,
        ..CovPolicy::default()
    });
    table.subscribe(indefinite(1), Instant::ZERO).unwrap();
    let error = table.subscribe(indefinite(2), Instant::ZERO).unwrap_err();
    assert_eq!(
        refusal(error),
        (
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT
        )
    );
    assert_eq!(table.take_counts().subscriptions_rejected_indefinite, 1);
}
