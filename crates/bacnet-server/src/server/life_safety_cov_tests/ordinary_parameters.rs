use super::*;
use bacnet_encoding::primitives;
use bacnet_types::enums::RejectReason;

fn raw_request(confirmed: Option<bool>, lifetime: Option<u32>) -> Bytes {
    let mut bytes = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut bytes, 0, 14);
    primitives::encode_ctx_object_id(&mut bytes, 1, &point_oid());
    if let Some(value) = confirmed {
        primitives::encode_ctx_boolean(&mut bytes, 2, value);
    }
    if let Some(value) = lifetime {
        primitives::encode_ctx_unsigned(&mut bytes, 3, u64::from(value));
    }
    bytes.freeze()
}

async fn invalid_renewal(lifetime: u32) {
    let fixture = DispatchFixture::new(life_safety_db(), []).await;
    fixture
        .dispatch(
            1,
            ConfirmedServiceChoice::SUBSCRIBE_COV,
            raw_request(Some(false), Some(28800)),
        )
        .await;
    let initial = fixture.take_apdus();
    assert_eq!(initial.len(), 2);
    assert!(matches!(&initial[0], Apdu::SimpleAck(_)));
    let before = fixture
        .cov_table
        .read()
        .await
        .get_subscription(&crate::cov::CovSubscriptionKey::Object {
            endpoint: crate::cov::SubscriberEndpoint::new(&fixture.source_mac, None),
            process_id: 14,
            object: point_oid(),
        })
        .unwrap()
        .clone();
    fixture
        .dispatch(
            2,
            ConfirmedServiceChoice::SUBSCRIBE_COV,
            raw_request(None, Some(lifetime)),
        )
        .await;
    let apdus = fixture.take_apdus();
    assert_eq!(
        apdus.len(),
        1,
        "malformed renewal emitted an initial COV notification: {apdus:?}"
    );
    assert!(matches!(&apdus[0], Apdu::Reject(pdu)
        if pdu.invoke_id == 2 && pdu.reject_reason == RejectReason::INCONSISTENT_PARAMETERS));
    let table = fixture.cov_table.read().await;
    assert_eq!(table.len(), 1);
    let after = table
        .get_subscription(&crate::cov::CovSubscriptionKey::Object {
            endpoint: crate::cov::SubscriberEndpoint::new(&fixture.source_mac, None),
            process_id: 14,
            object: point_oid(),
        })
        .unwrap();
    assert_eq!(after.expires_at, before.expires_at);
    assert_eq!(
        after.issue_confirmed_notifications,
        before.issue_confirmed_notifications
    );
    assert_eq!(after.last_notified_value, before.last_notified_value);
    assert_eq!(after.cov_increment, before.cov_increment);
    assert_eq!(table.peer_subscription_count(&before.peer_key()), 1);
    assert_eq!(table.peer_indefinite_count(&before.peer_key()), 0);
}

#[tokio::test]
async fn subscribe_cov_lifetime_only_zero_renewal_rejects_without_initial_or_state_change() {
    invalid_renewal(0).await;
}

#[tokio::test]
async fn subscribe_cov_lifetime_only_positive_renewal_rejects_without_initial_or_state_change() {
    invalid_renewal(300).await;
}
