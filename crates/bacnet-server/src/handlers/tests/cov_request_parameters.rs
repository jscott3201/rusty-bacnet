use super::*;
use bacnet_encoding::primitives;
use std::time::{Duration, Instant};

fn object(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn raw(oid: ObjectIdentifier, mode: Option<bool>, lifetime: Option<u32>) -> BytesMut {
    let mut bytes = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut bytes, 0, 7);
    primitives::encode_ctx_object_id(&mut bytes, 1, &oid);
    if let Some(value) = mode {
        primitives::encode_ctx_boolean(&mut bytes, 2, value);
    }
    if let Some(value) = lifetime {
        primitives::encode_ctx_unsigned(&mut bytes, 3, u64::from(value));
    }
    bytes
}

#[test]
fn subscribe_cov_lifetime_only_preserves_whole_table_and_quota_before_lookup_or_purge() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let finite = handle_subscribe_cov_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(1), Some(true), Some(28800)),
    )
    .unwrap()
    .remove(0);
    let mut expired = (*finite).clone();
    expired.subscriber_mac = MacAddr::from_slice(&[2]);
    expired.subscriber_process_identifier = 99;
    expired.expires_at = Some(Instant::now() - Duration::from_secs(1));
    table.subscribe(expired.clone()).unwrap();
    let counters = table.counters().snapshot();
    for target in [object(1), object(999)] {
        for lifetime in [0, 300] {
            let error = handle_subscribe_cov_with_initial(
                &mut table,
                &db,
                &[1],
                &raw(target, None, Some(lifetime)),
            )
            .unwrap_err();
            assert!(matches!(error, Error::Reject { reason }
                if reason == RejectReason::INCONSISTENT_PARAMETERS.to_raw()));
            assert_eq!(table.len(), 2);
            assert_eq!(table.counters().snapshot(), counters);
            for before in [&finite, &expired] {
                let after = table
                    .get_subscription(&crate::cov::CovSubscriptionKey::Object {
                        endpoint: crate::cov::SubscriberEndpoint::new(&before.subscriber_mac, None),
                        process_id: before.subscriber_process_identifier,
                        object: before.monitored_object_identifier,
                    })
                    .unwrap();
                assert_eq!(after.expires_at, before.expires_at);
                assert_eq!(
                    after.issue_confirmed_notifications,
                    before.issue_confirmed_notifications
                );
                assert_eq!(after.last_notified_sample, before.last_notified_sample);
                assert_eq!(after.cov_increment, before.cov_increment);
                assert_eq!(table.peer_subscription_count(&before.peer_key()), 1);
                assert_eq!(table.peer_indefinite_count(&before.peer_key()), 0);
            }
        }
    }
    assert_eq!(table.purge_expired(), 1);
}

#[test]
fn subscribe_cov_lifetime_only_on_empty_table_consumes_no_capacity() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let before = table.counters().snapshot();
    for lifetime in [0, 300] {
        assert!(handle_subscribe_cov_with_initial(
            &mut table,
            &db,
            &[1],
            &raw(object(1), None, Some(lifetime))
        )
        .is_err());
        assert!(table.is_empty());
        assert_eq!(table.counters().snapshot(), before);
    }
}

#[tokio::test]
async fn subscribe_cov_valid_renewal_expiry_and_nonexistent_cancel_remain_supported() {
    let db = make_db_with_ai();
    let mut table = CovSubscriptionTable::new();
    let first = handle_subscribe_cov_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(1), Some(false), Some(1)),
    )
    .unwrap()
    .remove(0);
    let renewed = handle_subscribe_cov_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(1), Some(true), Some(28800)),
    )
    .unwrap()
    .remove(0);
    assert!(renewed.expires_at.unwrap() > first.expires_at.unwrap() + Duration::from_secs(28000));
    assert_eq!(table.len(), 1);
    assert!(handle_subscribe_cov_with_initial(
        &mut table,
        &db,
        &[1],
        &raw(object(999), None, None)
    )
    .unwrap()
    .is_empty());
    assert_eq!(table.len(), 1);
    handle_subscribe_cov_with_initial(&mut table, &db, &[1], &raw(object(1), Some(false), Some(1)))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(table.purge_expired(), 1);
    assert!(table.is_empty());
}
