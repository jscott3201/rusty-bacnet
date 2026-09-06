use super::*;

use bacnet_encoding::apdu::{decode_apdu, ComplexAck, SimpleAck};
use bacnet_services::audit::{
    AuditLogQueryAck, AuditLogQueryRequest, AuditNotificationRequest, AuditPropertyReference,
    BACnetAuditLogDatum, BACnetAuditLogQueryParameters, BACnetAuditLogRecord,
    BACnetAuditLogRecordResult, BACnetAuditNotification,
};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{AuditOperation, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};

fn object_identifier(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn notification_request() -> AuditNotificationRequest {
    AuditNotificationRequest {
        notifications: vec![BACnetAuditNotification {
            source_timestamp: Some(BACnetTimeStamp::SequenceNumber(101)),
            target_timestamp: Some(BACnetTimeStamp::SequenceNumber(202)),
            source_device: BACnetRecipient::Device(object_identifier(ObjectType::DEVICE, 1)),
            source_object: Some(object_identifier(ObjectType::ANALOG_INPUT, 2)),
            operation: AuditOperation::WRITE,
            source_comment: Some("source comment".into()),
            target_comment: Some("target comment".into()),
            invoke_id: Some(203),
            source_user_id: Some(40_001),
            source_user_role: Some(204),
            target_device: BACnetRecipient::Device(object_identifier(ObjectType::DEVICE, 3)),
            target_object: Some(object_identifier(ObjectType::ANALOG_OUTPUT, 4)),
            target_property: Some(AuditPropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: Some(u64::from(u32::MAX) + 1),
            }),
            target_priority: Some(16),
            target_value: Some(vec![0x00]),
            current_value: Some(vec![0x11]),
            result: Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
        }],
    }
}

fn query_request() -> AuditLogQueryRequest {
    AuditLogQueryRequest {
        audit_log: object_identifier(ObjectType::AUDIT_LOG, 11),
        query_parameters: BACnetAuditLogQueryParameters::BySource {
            source_device_identifier: object_identifier(ObjectType::DEVICE, 12),
            source_device_address: None,
            source_object_identifier: Some(object_identifier(ObjectType::ANALOG_INPUT, 13)),
            operations: None,
            successful_actions_only: false,
        },
        start_at_sequence_number: Some(0x0102_0304),
        requested_count: 513,
    }
}

fn query_ack() -> AuditLogQueryAck {
    AuditLogQueryAck {
        audit_log: object_identifier(ObjectType::AUDIT_LOG, 11),
        records: vec![BACnetAuditLogRecordResult {
            sequence_number: u64::from(u32::MAX) + 1,
            record: BACnetAuditLogRecord {
                timestamp: (
                    Date {
                        year: 126,
                        month: 9,
                        day: 4,
                        day_of_week: 5,
                    },
                    Time {
                        hour: 12,
                        minute: 34,
                        second: 56,
                        hundredths: 78,
                    },
                ),
                datum: BACnetAuditLogDatum::LogStatus(0b010),
            },
        }],
        no_more_items: true,
    }
}

#[tokio::test]
async fn confirmed_audit_notification_sends_exact_typed_payload_and_accepts_simple_ack() {
    let client_mac = vec![0x01];
    let remote_mac = vec![0x02];
    let (client_transport, remote_transport) =
        LoopbackTransport::pair(client_mac, remote_mac.clone());
    let mut remote_network = NetworkLayer::new(remote_transport);
    let mut remote_rx = remote_network.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let request = notification_request();
    let mut expected_payload = BytesMut::new();
    request.try_encode(&mut expected_payload).unwrap();
    let expected_request = request.clone();

    let responder = tokio::spawn(async move {
        let received = timeout(Duration::from_secs(1), remote_rx.recv())
            .await
            .expect("remote timed out")
            .expect("remote channel closed");
        let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu).unwrap() else {
            panic!("expected ConfirmedRequest");
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
        );
        assert_eq!(request.service_request, expected_payload.freeze());
        assert_eq!(
            AuditNotificationRequest::decode(&request.service_request).unwrap(),
            expected_request
        );

        let mut ack = BytesMut::new();
        encode_apdu(
            &mut ack,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
            }),
        )
        .unwrap();
        remote_network
            .send_apdu(&ack, &received.source_mac, false, NetworkPriority::NORMAL)
            .await
            .unwrap();
        remote_network.stop().await.unwrap();
    });

    client
        .confirmed_audit_notification(&remote_mac, &request)
        .await
        .unwrap();

    responder.await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_audit_notification_sends_exact_typed_payload_without_response() {
    let client_mac = vec![0x01];
    let remote_mac = vec![0x02];
    let (client_transport, remote_transport) =
        LoopbackTransport::pair(client_mac, remote_mac.clone());
    let mut remote_network = NetworkLayer::new(remote_transport);
    let mut remote_rx = remote_network.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let request = notification_request();
    let mut expected_payload = BytesMut::new();
    request.try_encode(&mut expected_payload).unwrap();

    client
        .unconfirmed_audit_notification(&remote_mac, &request)
        .await
        .unwrap();

    let received = timeout(Duration::from_secs(1), remote_rx.recv())
        .await
        .expect("remote timed out")
        .expect("remote channel closed");
    let Apdu::UnconfirmedRequest(sent) = decode_apdu(received.apdu).unwrap() else {
        panic!("expected UnconfirmedRequest");
    };
    assert_eq!(
        sent.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
    );
    assert_eq!(sent.service_request, expected_payload.freeze());
    assert_eq!(
        AuditNotificationRequest::decode(&sent.service_request).unwrap(),
        request
    );

    remote_network.stop().await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn audit_log_query_sends_exact_typed_payload_and_decodes_complex_ack() {
    let client_mac = vec![0x01];
    let remote_mac = vec![0x02];
    let (client_transport, remote_transport) =
        LoopbackTransport::pair(client_mac, remote_mac.clone());
    let mut remote_network = NetworkLayer::new(remote_transport);
    let mut remote_rx = remote_network.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let request = query_request();
    let expected_request = request.clone();
    let mut expected_payload = BytesMut::new();
    request.try_encode(&mut expected_payload).unwrap();
    let expected_ack = query_ack();
    let mut encoded_ack = BytesMut::new();
    expected_ack.try_encode(&mut encoded_ack).unwrap();

    let responder = tokio::spawn(async move {
        let received = timeout(Duration::from_secs(1), remote_rx.recv())
            .await
            .expect("remote timed out")
            .expect("remote channel closed");
        let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu).unwrap() else {
            panic!("expected ConfirmedRequest");
        };
        assert_eq!(
            request.service_choice,
            ConfirmedServiceChoice::AUDIT_LOG_QUERY
        );
        assert_eq!(request.service_request, expected_payload.freeze());
        assert_eq!(
            AuditLogQueryRequest::decode(&request.service_request).unwrap(),
            expected_request
        );

        let mut ack = BytesMut::new();
        encode_apdu(
            &mut ack,
            &Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: request.service_choice,
                service_ack: encoded_ack.freeze(),
            }),
        )
        .unwrap();
        remote_network
            .send_apdu(&ack, &received.source_mac, false, NetworkPriority::NORMAL)
            .await
            .unwrap();
        remote_network.stop().await.unwrap();
    });

    let ack = client.audit_log_query(&remote_mac, &request).await.unwrap();

    assert_eq!(ack, expected_ack);
    responder.await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn audit_log_query_rejects_malformed_trailing_and_missing_ack_service_data() {
    let client_mac = vec![0x01];
    let remote_mac = vec![0x02];
    let (client_transport, remote_transport) =
        LoopbackTransport::pair(client_mac, remote_mac.clone());
    let mut remote_network = NetworkLayer::new(remote_transport);
    let mut remote_rx = remote_network.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let request = query_request();
    let mut valid_ack = BytesMut::new();
    query_ack().try_encode(&mut valid_ack).unwrap();
    let mut trailing_ack = valid_ack.to_vec();
    trailing_ack.push(0x00);
    let response_payloads = vec![vec![0xff], trailing_ack, Vec::new()];

    let responder = tokio::spawn(async move {
        for service_ack in response_payloads {
            let received = timeout(Duration::from_secs(1), remote_rx.recv())
                .await
                .expect("remote timed out")
                .expect("remote channel closed");
            let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu).unwrap() else {
                panic!("expected ConfirmedRequest");
            };
            assert_eq!(
                request.service_choice,
                ConfirmedServiceChoice::AUDIT_LOG_QUERY
            );

            let mut ack = BytesMut::new();
            encode_apdu(
                &mut ack,
                &Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id: request.invoke_id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: request.service_choice,
                    service_ack: Bytes::from(service_ack),
                }),
            )
            .unwrap();
            remote_network
                .send_apdu(&ack, &received.source_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();
        }
        remote_network.stop().await.unwrap();
    });

    for _ in 0..3 {
        assert!(client.audit_log_query(&remote_mac, &request).await.is_err());
    }

    responder.await.unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn audit_helpers_return_encode_errors_without_emitting_a_frame() {
    let client_mac = vec![0x01];
    let remote_mac = vec![0x02];
    let original_remote_mac = remote_mac.clone();
    let (client_transport, remote_transport) =
        LoopbackTransport::pair(client_mac, remote_mac.clone());
    let mut remote_network = NetworkLayer::new(remote_transport);
    let mut remote_rx = remote_network.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let empty_notification = AuditNotificationRequest {
        notifications: Vec::new(),
    };
    let invalid_query = AuditLogQueryRequest {
        audit_log: object_identifier(ObjectType::AUDIT_LOG, 11),
        query_parameters: BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: object_identifier(ObjectType::DEVICE, 12),
            target_device_address: None,
            target_object_identifier: None,
            target_property_identifier: None,
            target_array_index: None,
            target_priority: Some(0),
            operations: None,
            successful_actions_only: true,
        },
        start_at_sequence_number: None,
        requested_count: 1,
    };

    let _: Error = client
        .confirmed_audit_notification(&remote_mac, &empty_notification)
        .await
        .unwrap_err();
    let _: Error = client
        .unconfirmed_audit_notification(&remote_mac, &empty_notification)
        .await
        .unwrap_err();
    let _: Error = client
        .audit_log_query(&remote_mac, &invalid_query)
        .await
        .unwrap_err();

    assert_eq!(remote_mac, original_remote_mac);
    assert!(
        timeout(Duration::from_millis(20), remote_rx.recv())
            .await
            .is_err(),
        "encode failures must not emit an APDU"
    );

    remote_network.stop().await.unwrap();
    client.stop().await.unwrap();
}
