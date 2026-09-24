use super::*;
use bacnet_encoding::{primitives, tags};
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use std::time::{Duration, Instant};

fn object(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn raw(oid: ObjectIdentifier, confirmed: Option<bool>, lifetime: Option<u32>) -> BytesMut {
    let mut bytes = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut bytes, 0, 7);
    primitives::encode_ctx_object_id(&mut bytes, 1, &oid);
    if let Some(value) = confirmed {
        primitives::encode_ctx_boolean(&mut bytes, 2, value);
    }
    if let Some(value) = lifetime {
        primitives::encode_ctx_unsigned(&mut bytes, 3, u64::from(value));
    }
    tags::encode_opening_tag(&mut bytes, 4);
    primitives::encode_ctx_unsigned(
        &mut bytes,
        0,
        PropertyIdentifier::PRESENT_VALUE.to_raw() as u64,
    );
    tags::encode_closing_tag(&mut bytes, 4);
    primitives::encode_ctx_real(&mut bytes, 5, 0.25);
    bytes
}

#[test]
fn subscribe_cov_property_parameter_errors_precede_lookup_and_expired_entry_purge() {
    let db = make_db_with_ai();
    let mac = [1];
    let mut table = CovSubscriptionTable::new();
    let seeded = handle_subscribe_cov_property_with_initial(
        &mut table,
        &db,
        &mac,
        &raw(object(1), Some(true), Some(300)),
    )
    .unwrap();
    let mut expired = seeded[0].clone();
    expired.expires_at = Some(Instant::now() - Duration::from_secs(1));
    table.subscribe(expired.clone());
    for target in [object(1), object(999)] {
        for (confirmed, lifetime, reject) in [
            (Some(false), None, true),
            (None, Some(1), true),
            (None, Some(0), true),
            (Some(false), Some(0), false),
        ] {
            let error = handle_subscribe_cov_property_with_initial(
                &mut table,
                &db,
                &mac,
                &raw(target, confirmed, lifetime),
            )
            .unwrap_err();
            if reject {
                assert!(matches!(error, Error::Reject { reason }
                    if reason == RejectReason::INCONSISTENT_PARAMETERS.to_raw()));
            } else {
                assert!(matches!(error, Error::Protocol { class, code }
                    if class == ErrorClass::SERVICES.to_raw() as u32
                    && code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32));
            }
            assert_eq!(
                table.len(),
                1,
                "invalid request must not purge even expired entries"
            );
            let current = table
                .get_subscription(
                    &MacAddr::from_slice(&mac),
                    None,
                    7,
                    object(1),
                    Some(PropertyIdentifier::PRESENT_VALUE),
                )
                .unwrap();
            assert_eq!(current.expires_at, expired.expires_at);
            assert_eq!(
                current.issue_confirmed_notifications,
                expired.issue_confirmed_notifications
            );
            assert_eq!(current.cov_increment, expired.cov_increment);
            assert_eq!(table.peer_subscription_count(&expired.peer_key()), 1);
            assert_eq!(table.peer_indefinite_count(&expired.peer_key()), 0);
        }
    }
    assert_eq!(table.purge_expired(), 1);
}

#[tokio::test]
async fn subscribe_cov_property_positive_renewal_expiry_and_nonexistent_cancel() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let first = handle_subscribe_cov_property_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(1), Some(false), Some(1)),
    )
    .unwrap()
    .remove(0);
    let renewed = handle_subscribe_cov_property_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(1), Some(true), Some(28800)),
    )
    .unwrap()
    .remove(0);
    assert!(renewed.expires_at.unwrap() > first.expires_at.unwrap() + Duration::from_secs(28000));
    assert!(renewed.issue_confirmed_notifications);
    assert_eq!(table.len(), 1);
    assert_eq!(table.peer_indefinite_count(&renewed.peer_key()), 0);
    assert!(handle_subscribe_cov_property_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(999), None, None)
    )
    .unwrap()
    .is_empty());
    assert_eq!(table.len(), 1);
    // A real one-second renewal must expire through the handler-created deadline.
    handle_subscribe_cov_property_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(1), Some(false), Some(1)),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(table.purge_expired(), 1);
    assert!(table.is_empty());
    assert!(handle_subscribe_cov_property_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(1), None, None)
    )
    .unwrap()
    .is_empty());
}

#[test]
fn subscribe_cov_ordinary_none_and_zero_lifetimes_remain_indefinite() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    for lifetime in [None, Some(0)] {
        let request = SubscribeCOVRequest {
            subscriber_process_identifier: 9,
            monitored_object_identifier: object(1),
            issue_confirmed_notifications: Some(false),
            lifetime,
        };
        let mut bytes = BytesMut::new();
        request.encode(&mut bytes);
        let initial = handle_subscribe_cov_with_initial(&mut table, &db, &[1], &bytes).unwrap();
        assert_eq!(initial.len(), 1);
        assert!(initial[0].expires_at.is_none());
        assert_eq!(table.peer_indefinite_count(&initial[0].peer_key()), 1);
    }
    assert_eq!(table.len(), 1);
    // Single-property decode remains structural even for the service-invalid form.
    assert!(SubscribeCOVPropertyRequest::decode(&raw(object(1), Some(false), Some(0))).is_ok());
}
