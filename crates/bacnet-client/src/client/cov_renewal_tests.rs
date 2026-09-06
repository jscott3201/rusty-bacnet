use super::*;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu, UnconfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::SubscribeCOVRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{AbortReason, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

fn analog_object(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

fn cov_notification(
    subscriber_process_identifier: u32,
    monitored_object_identifier: ObjectIdentifier,
    time_remaining: u32,
) -> COVNotificationRequest {
    COVNotificationRequest {
        subscriber_process_identifier,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        monitored_object_identifier,
        time_remaining,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x48, 0x00, 0x00],
            priority: None,
        }],
    }
}

fn received_cov_notification(
    subscriber_process_identifier: u32,
    monitored_object_identifier: ObjectIdentifier,
    time_remaining: u32,
    source_mac: &[u8],
) -> ReceivedCOVNotification {
    ReceivedCOVNotification::new(
        cov_notification(
            subscriber_process_identifier,
            monitored_object_identifier,
            time_remaining,
        ),
        source_mac,
        &None,
        COVNotificationDelivery::Unconfirmed,
    )
}

async fn send_local_simple_ack<T: TransportPort>(
    transport: &T,
    client_mac: &[u8],
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
) {
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice,
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &ack).unwrap();

    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, client_mac).await.unwrap();
}

async fn receive_local_subscribe_cov(
    peer_rx: &mut tokio::sync::mpsc::Receiver<ReceivedNpdu>,
    client_mac: &[u8],
) -> (u8, SubscribeCOVRequest) {
    let received = timeout(Duration::from_secs(2), peer_rx.recv())
        .await
        .expect("peer timed out waiting for SubscribeCOV")
        .expect("peer channel closed");
    assert_eq!(&received.source_mac[..], client_mac);

    let npdu = decode_npdu(received.npdu).unwrap();
    assert!(
        npdu.destination.is_none(),
        "managed direct COV renewal must use local addressing"
    );
    let decoded = decode_apdu(npdu.payload).unwrap();
    let Apdu::ConfirmedRequest(request) = decoded else {
        panic!("expected confirmed request, got {decoded:?}");
    };
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::SUBSCRIBE_COV
    );
    assert!(!request.segmented);

    (
        request.invoke_id,
        SubscribeCOVRequest::decode(&request.service_request).unwrap(),
    )
}

async fn send_unconfirmed_cov_notification<T: TransportPort>(
    transport: &T,
    client_mac: &[u8],
    subscriber_process_identifier: u32,
    monitored_object_identifier: ObjectIdentifier,
    time_remaining: u32,
) {
    let notification = cov_notification(
        subscriber_process_identifier,
        monitored_object_identifier,
        time_remaining,
    );
    let mut service_request = BytesMut::new();
    notification.encode(&mut service_request);
    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
        service_request: service_request.freeze(),
    });
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();

    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, client_mac).await.unwrap();
}

#[tokio::test]
async fn managed_cov_subscription_uses_notification_observed_during_initial_subscribe() {
    let client_mac = vec![0x41];
    let peer_mac = vec![0x42];
    let monitored_object = analog_object(4);
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );

    let subscriber_process_identifier = 4004;
    let client_for_task = Arc::clone(&client);
    let peer_mac_for_task = peer_mac.clone();
    let start = tokio::spawn(async move {
        client_for_task
            .manage_cov_subscription(
                &peer_mac_for_task,
                subscriber_process_identifier,
                monitored_object,
                false,
                20,
                ManagedCOVSubscriptionOptions::default()
                    .with_renewal_margin(Duration::from_secs(5)),
            )
            .await
    });

    let (invoke_id, _) = receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    assert_eq!(
        client
            .cov_tx
            .send(received_cov_notification(
                subscriber_process_identifier,
                monitored_object,
                3,
                &peer_mac,
            ))
            .unwrap(),
        1
    );
    send_local_simple_ack(
        &peer_transport,
        &client_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;

    let managed = start.await.unwrap().unwrap();
    let (renew_invoke_id, renewed_request) =
        receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    assert_eq!(
        renewed_request.subscriber_process_identifier,
        subscriber_process_identifier
    );
    assert_eq!(
        managed.last_event(),
        Some(ManagedCOVSubscriptionEvent::ImpendingExpiry { time_remaining: 3 })
    );
    send_local_simple_ack(
        &peer_transport,
        &client_mac,
        renew_invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;

    managed.stop().await;
    let mut client = match Arc::try_unwrap(client) {
        Ok(client) => client,
        Err(_) => panic!("managed subscription kept a client reference after stop"),
    };
    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn managed_cov_subscription_renews_before_requested_lifetime() {
    let client_mac = vec![0x51];
    let peer_mac = vec![0x52];
    let monitored_object = analog_object(1);
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );

    let subscriber_process_identifier = 4001;
    let client_for_task = Arc::clone(&client);
    let peer_mac_for_task = peer_mac.clone();
    let start = tokio::spawn(async move {
        client_for_task
            .manage_cov_subscription(
                &peer_mac_for_task,
                subscriber_process_identifier,
                monitored_object,
                false,
                2,
                ManagedCOVSubscriptionOptions::default()
                    .with_renewal_margin(Duration::from_millis(1500)),
            )
            .await
    });

    let (invoke_id, request) = receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    assert_eq!(
        request.subscriber_process_identifier,
        subscriber_process_identifier
    );
    assert_eq!(request.monitored_object_identifier, monitored_object);
    assert_eq!(request.issue_confirmed_notifications, Some(false));
    assert_eq!(request.lifetime, Some(2));
    send_local_simple_ack(
        &peer_transport,
        &client_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;

    let managed = start.await.unwrap().unwrap();
    let mut events = managed.events();
    let (renew_invoke_id, renewed_request) =
        receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    assert_eq!(
        renewed_request.subscriber_process_identifier,
        subscriber_process_identifier
    );
    assert_eq!(
        renewed_request.monitored_object_identifier,
        monitored_object
    );
    assert_eq!(renewed_request.lifetime, Some(2));
    send_local_simple_ack(
        &peer_transport,
        &client_mac,
        renew_invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;

    let event = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("timed out waiting for renewal event")
        .expect("renewal events closed");
    assert_eq!(
        event,
        ManagedCOVSubscriptionEvent::Renewed {
            requested_lifetime: 2,
            renew_after: Duration::from_millis(500),
        }
    );

    managed.stop().await;
    let mut client = match Arc::try_unwrap(client) {
        Ok(client) => client,
        Err(_) => panic!("managed subscription kept a client reference after stop"),
    };
    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn managed_cov_subscription_uses_observed_time_remaining() {
    let client_mac = vec![0x61];
    let peer_mac = vec![0x62];
    let monitored_object = analog_object(2);
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );

    let subscriber_process_identifier = 4002;
    let client_for_task = Arc::clone(&client);
    let peer_mac_for_task = peer_mac.clone();
    let start = tokio::spawn(async move {
        client_for_task
            .manage_cov_subscription(
                &peer_mac_for_task,
                subscriber_process_identifier,
                monitored_object,
                false,
                20,
                ManagedCOVSubscriptionOptions::default()
                    .with_renewal_margin(Duration::from_secs(5)),
            )
            .await
    });

    let (invoke_id, _) = receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    send_local_simple_ack(
        &peer_transport,
        &client_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;
    let managed = start.await.unwrap().unwrap();
    let mut events = managed.events();

    send_unconfirmed_cov_notification(
        &peer_transport,
        &client_mac,
        subscriber_process_identifier,
        monitored_object,
        3,
    )
    .await;

    let observed = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("timed out waiting for observation")
        .expect("renewal events closed");
    assert_eq!(
        observed,
        ManagedCOVSubscriptionEvent::NotificationObserved {
            time_remaining: 3,
            renew_after: Duration::from_secs(0),
        }
    );
    let expiry = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("timed out waiting for expiry warning")
        .expect("renewal events closed");
    assert_eq!(
        expiry,
        ManagedCOVSubscriptionEvent::ImpendingExpiry { time_remaining: 3 }
    );

    let (renew_invoke_id, renewed_request) =
        receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    assert_eq!(
        renewed_request.subscriber_process_identifier,
        subscriber_process_identifier
    );
    assert_eq!(
        renewed_request.monitored_object_identifier,
        monitored_object
    );
    assert_eq!(renewed_request.lifetime, Some(20));
    send_local_simple_ack(
        &peer_transport,
        &client_mac,
        renew_invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;
    let renewed = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("timed out waiting for renewal")
        .expect("renewal events closed");
    assert_eq!(
        renewed,
        ManagedCOVSubscriptionEvent::Renewed {
            requested_lifetime: 20,
            renew_after: Duration::from_secs(15),
        }
    );

    managed.stop().await;
    let mut client = match Arc::try_unwrap(client) {
        Ok(client) => client,
        Err(_) => panic!("managed subscription kept a client reference after stop"),
    };
    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn managed_cov_subscription_stop_cancels_in_flight_renewal() {
    let client_mac = vec![0x81];
    let peer_mac = vec![0x82];
    let monitored_object = analog_object(5);
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(1000)
            .apdu_retries(3)
            .build()
            .await
            .unwrap(),
    );

    let subscriber_process_identifier = 4005;
    let client_for_task = Arc::clone(&client);
    let peer_mac_for_task = peer_mac.clone();
    let start = tokio::spawn(async move {
        client_for_task
            .manage_cov_subscription(
                &peer_mac_for_task,
                subscriber_process_identifier,
                monitored_object,
                false,
                2,
                ManagedCOVSubscriptionOptions::default()
                    .with_renewal_margin(Duration::from_millis(1500)),
            )
            .await
    });

    let (invoke_id, _) = receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    send_local_simple_ack(
        &peer_transport,
        &client_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;
    let managed = start.await.unwrap().unwrap();

    let (_renew_invoke_id, renewed_request) =
        receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    assert_eq!(
        renewed_request.subscriber_process_identifier,
        subscriber_process_identifier
    );

    timeout(Duration::from_millis(500), managed.stop())
        .await
        .expect("managed stop should cancel in-flight renewal promptly");
    assert_eq!(client.tsm.lock().await.pending_count(), 0);

    let mut client = match Arc::try_unwrap(client) {
        Ok(client) => client,
        Err(_) => panic!("managed subscription kept a client reference after stop"),
    };
    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn managed_cov_subscription_records_failure_without_event_receiver() {
    let client_mac = vec![0x91];
    let peer_mac = vec![0x92];
    let monitored_object = analog_object(6);
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let mut peer_rx = peer_transport.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(100)
            .apdu_retries(0)
            .build()
            .await
            .unwrap(),
    );

    let subscriber_process_identifier = 4006;
    let client_for_task = Arc::clone(&client);
    let peer_mac_for_task = peer_mac.clone();
    let start = tokio::spawn(async move {
        client_for_task
            .manage_cov_subscription(
                &peer_mac_for_task,
                subscriber_process_identifier,
                monitored_object,
                false,
                2,
                ManagedCOVSubscriptionOptions::default()
                    .with_renewal_margin(Duration::from_millis(1500)),
            )
            .await
    });

    let (invoke_id, _) = receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    send_local_simple_ack(
        &peer_transport,
        &client_mac,
        invoke_id,
        ConfirmedServiceChoice::SUBSCRIBE_COV,
    )
    .await;
    let managed = start.await.unwrap().unwrap();

    let (_renew_invoke_id, renewed_request) =
        receive_local_subscribe_cov(&mut peer_rx, &client_mac).await;
    assert_eq!(
        renewed_request.subscriber_process_identifier,
        subscriber_process_identifier
    );

    timeout(Duration::from_secs(2), async {
        while !managed.is_finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("renewal should fail after the configured timeout");
    let Some(ManagedCOVSubscriptionEvent::RenewalFailed { error }) = managed.last_event() else {
        panic!("expected durable RenewalFailed event");
    };
    assert_eq!(
        error,
        format!("BACnet abort: reason={}", AbortReason::TSM_TIMEOUT.to_raw())
    );
    assert_eq!(client.tsm.lock().await.pending_count(), 0);

    managed.stop().await;
    let mut client = match Arc::try_unwrap(client) {
        Ok(client) => client,
        Err(_) => panic!("managed subscription kept a client reference after stop"),
    };
    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn managed_cov_subscription_validates_finite_lifetime_options() {
    let client_mac = vec![0x71];
    let peer_mac = vec![0x72];
    let monitored_object = analog_object(3);
    let (client_transport, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let _peer_rx = peer_transport.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .build()
            .await
            .unwrap(),
    );

    let zero_lifetime = match Arc::clone(&client)
        .manage_cov_subscription(
            &peer_mac,
            4003,
            monitored_object,
            false,
            0,
            ManagedCOVSubscriptionOptions::default(),
        )
        .await
    {
        Ok(_) => panic!("zero lifetime should be rejected before subscribing"),
        Err(error) => error,
    };
    assert!(zero_lifetime
        .to_string()
        .contains("finite non-zero lifetime"));

    let oversized_margin = match Arc::clone(&client)
        .manage_cov_subscription(
            &peer_mac,
            4003,
            monitored_object,
            false,
            5,
            ManagedCOVSubscriptionOptions::default().with_renewal_margin(Duration::from_secs(5)),
        )
        .await
    {
        Ok(_) => panic!("oversized renewal margin should be rejected before subscribing"),
        Err(error) => error,
    };
    assert!(oversized_margin
        .to_string()
        .contains("renewal margin must be shorter than lifetime"));

    for capacity in [0, MAX_COV_CHANNEL_CAPACITY + 1] {
        let invalid_capacity = match Arc::clone(&client)
            .manage_cov_subscription(
                &peer_mac,
                4003,
                monitored_object,
                false,
                60,
                ManagedCOVSubscriptionOptions::default().with_event_channel_capacity(capacity),
            )
            .await
        {
            Ok(_) => panic!("invalid event channel capacity should be rejected before subscribing"),
            Err(error) => error,
        };
        assert!(invalid_capacity
            .to_string()
            .contains("event channel capacity"));
    }

    let mut client = match Arc::try_unwrap(client) {
        Ok(client) => client,
        Err(_) => panic!("validation kept a client reference"),
    };
    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}
