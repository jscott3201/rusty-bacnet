use super::*;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::ObjectType;

#[tokio::test]
async fn subscribe_cov_property_invalid_raw_typed_requests_never_admit_or_send() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut received = peer.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    for routed in [false, true] {
        for (confirmed, lifetime) in [
            (Some(false), None),
            (None, Some(300)),
            (None, Some(0)),
            (Some(false), Some(0)),
        ] {
            let request = SubscribeCOVPropertyRequest {
                subscriber_process_identifier: 1,
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
                    .unwrap(),
                issue_confirmed_notifications: confirmed,
                lifetime,
                monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
                monitored_property_array_index: None,
                cov_increment: None,
            };
            let target = if routed {
                ConfirmedTarget::Routed {
                    router_mac: &[2],
                    dest_network: 42,
                    dest_mac: &[3],
                }
            } else {
                ConfirmedTarget::Local { mac: &[2] }
            };
            let error = timeout(
                Duration::from_secs(1),
                client.send_subscribe_cov_property_request(target, request),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(matches!(error, Error::Encoding(_)));
            assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
            assert_eq!(client.tsm.lock().await.pending_count(), 0);
            assert!(received.try_recv().is_err());
        }
    }
    assert!(timeout(Duration::from_millis(25), received.recv())
        .await
        .is_err());
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
