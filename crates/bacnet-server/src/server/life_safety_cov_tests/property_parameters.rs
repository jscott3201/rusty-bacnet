use super::*;
use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::RejectReason;

// Raw service fields deliberately bypass the typed encoder for invalid wire cases.
fn raw_request(confirmed: Option<bool>, lifetime: Option<u32>) -> Bytes {
    let mut buf = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut buf, 0, 12);
    primitives::encode_ctx_object_id(&mut buf, 1, &point_oid());
    if let Some(value) = confirmed {
        primitives::encode_ctx_boolean(&mut buf, 2, value);
    }
    if let Some(value) = lifetime {
        primitives::encode_ctx_unsigned(&mut buf, 3, u64::from(value));
    }
    tags::encode_opening_tag(&mut buf, 4);
    primitives::encode_ctx_unsigned(&mut buf, 0, PropertyIdentifier::SILENCED.to_raw() as u64);
    tags::encode_closing_tag(&mut buf, 4);
    primitives::encode_ctx_real(&mut buf, 5, 0.25);
    buf.freeze()
}

#[tokio::test]
async fn subscribe_cov_property_invalid_resubscription_keeps_state_and_emits_only_error() {
    for (confirmed, lifetime, reject) in [
        (Some(false), None, true),
        (None, Some(300), true),
        (None, Some(0), true),
        (Some(false), Some(0), false),
    ] {
        let fixture = DispatchFixture::new(life_safety_db(), []).await;
        fixture
            .dispatch(
                1,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
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
            .get_subscription(&crate::cov::CovSubscriptionKey::Property {
                endpoint: crate::cov::SubscriberEndpoint::new(&fixture.source_mac, None),
                process_id: 12,
                object: point_oid(),
                property: PropertyIdentifier::SILENCED,
                index: None,
            })
            .unwrap()
            .clone();
        fixture
            .dispatch(
                2,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
                raw_request(confirmed, lifetime),
            )
            .await;
        let apdus = fixture.take_apdus();
        assert_eq!(
            apdus.len(),
            1,
            "invalid request must not emit an initial notification: {apdus:?}"
        );
        if reject {
            assert!(matches!(&apdus[0], Apdu::Reject(pdu)
                if pdu.invoke_id == 2 && pdu.reject_reason == RejectReason::INCONSISTENT_PARAMETERS));
        } else {
            assert!(matches!(&apdus[0], Apdu::Error(pdu)
                if pdu.invoke_id == 2 && pdu.error_class == ErrorClass::SERVICES
                    && pdu.error_code == ErrorCode::VALUE_OUT_OF_RANGE));
        }
        let table = fixture.cov_table.read().await;
        assert_eq!(table.len(), 1);
        let after = table
            .get_subscription(&crate::cov::CovSubscriptionKey::Property {
                endpoint: crate::cov::SubscriberEndpoint::new(&fixture.source_mac, None),
                process_id: 12,
                object: point_oid(),
                property: PropertyIdentifier::SILENCED,
                index: None,
            })
            .unwrap();
        assert_eq!(after.expires_at, before.expires_at);
        assert_eq!(
            after.issue_confirmed_notifications,
            before.issue_confirmed_notifications
        );
        assert_eq!(after.cov_increment, before.cov_increment);
        assert_eq!(
            after.monitored_property_array_index,
            before.monitored_property_array_index
        );
        assert_eq!(after.last_notified_value, before.last_notified_value);
    }
}
