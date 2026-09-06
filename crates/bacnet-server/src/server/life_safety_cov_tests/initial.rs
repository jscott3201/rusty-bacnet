use super::*;

use bacnet_services::common::PropertyReference;
use bacnet_services::cov::SubscribeCOVPropertyRequest;
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};

#[tokio::test]
async fn subscribe_initial_and_resub_ack_precede_notification_and_cancel_is_quiet() {
    let fixture = DispatchFixture::new(life_safety_db(), []).await;
    let encode_request = |confirmed: Option<bool>, lifetime: Option<u32>| {
        let request = SubscribeCOVPropertyRequest {
            subscriber_process_identifier: 12,
            monitored_object_identifier: point_oid(),
            issue_confirmed_notifications: confirmed,
            lifetime,
            monitored_property_identifier: PropertyIdentifier::SILENCED,
            monitored_property_array_index: None,
            cov_increment: None,
        };
        let mut encoded = BytesMut::new();
        request.encode(&mut encoded);
        encoded.freeze()
    };

    for (invoke_id, confirmed, lifetime, expected_frames) in [
        (1, Some(false), Some(300), 2),
        (2, Some(false), Some(600), 2),
        (3, None, None, 1),
    ] {
        fixture
            .dispatch(
                invoke_id,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
                encode_request(confirmed, lifetime),
            )
            .await;
        let apdus = fixture.take_apdus();
        assert_eq!(apdus.len(), expected_frames);
        assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
        if expected_frames == 2 {
            assert_eq!(
                single_properties(&apdus[1]),
                vec![
                    PropertyIdentifier::SILENCED,
                    PropertyIdentifier::STATUS_FLAGS,
                ]
            );
        }
    }
}

fn multiple_request(lifetime: Option<u32>, max_notification_delay: Option<u32>) -> Bytes {
    let request = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 13,
        issue_confirmed_notifications: false,
        lifetime,
        max_notification_delay,
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: point_oid(),
            list_of_cov_references: vec![COVReference {
                monitored_property: PropertyReference {
                    property_identifier: PropertyIdentifier::SILENCED,
                    property_array_index: None,
                },
                cov_increment: None,
                timestamped: false,
            }],
        }],
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);
    encoded.freeze()
}

#[tokio::test]
async fn multiple_initial_and_resub_ack_precede_payload_and_cancel_is_quiet() {
    let fixture = DispatchFixture::new(life_safety_db(), []).await;

    for (invoke_id, lifetime, delay, expected_frames) in [
        (4, Some(300), Some(10), 2),
        (5, Some(600), Some(10), 2),
        (6, None, None, 1),
    ] {
        fixture
            .dispatch(
                invoke_id,
                ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
                multiple_request(lifetime, delay),
            )
            .await;
        let apdus = fixture.take_apdus();
        assert_eq!(apdus.len(), expected_frames);
        assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
        if expected_frames == 2 {
            let Apdu::UnconfirmedRequest(notification) = &apdus[1] else {
                panic!("expected unconfirmed COVNotificationMultiple");
            };
            let notification =
                COVNotificationMultipleRequest::decode(&notification.service_request).unwrap();
            let properties: Vec<_> = notification.list_of_cov_notifications[0]
                .list_of_values
                .iter()
                .map(|value| value.property_identifier)
                .collect();
            assert_eq!(
                properties,
                vec![
                    PropertyIdentifier::SILENCED,
                    PropertyIdentifier::STATUS_FLAGS,
                ]
            );
        }
    }
    assert!(fixture.cov_table.read().await.is_empty());
}
