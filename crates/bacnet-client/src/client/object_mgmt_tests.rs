//! Routed object and alarm mutations retain exact requests and existing replies.
use super::*;
use crate::discovery::RoutedDeviceConfig;
use bacnet_encoding::apdu::{
    self, AbortPdu, Apdu, ComplexAck, ConfirmedRequest, ErrorPdu, RejectPdu, SimpleAck,
};
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    AbortReason, ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier, RejectReason,
    Segmentation,
};
use bacnet_types::error::{ErrorDetail, RemoteError, SegmentationError};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

const INSTANCE: u32 = 1234;
const ROUTER: &[u8] = &[2];
const NETWORK: u16 = 100;
const STATION: &[u8] = &[5];

struct Peer {
    transport: LoopbackTransport,
    frames: mpsc::Receiver<ReceivedNpdu>,
    destinations: mpsc::UnboundedReceiver<MacAddr>,
    routed: bool,
}

async fn site(routed: bool) -> (BACnetClient<LoopbackTransport>, Peer) {
    let (mut local, mut remote) = LoopbackTransport::pair(&[1][..], ROUTER);
    let destinations = local.record_unicast_destinations();
    let frames = remote.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(local)
        .apdu_timeout_ms(1000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    if routed {
        client
            .add_routed_device(RoutedDeviceConfig {
                instance: INSTANCE,
                router_mac: ROUTER.to_vec(),
                remote_network: NETWORK,
                remote_mac: STATION.to_vec(),
                max_apdu_length: 1476,
                segmentation_supported: Segmentation::NONE,
                max_segments_accepted: None,
            })
            .await
            .unwrap();
    } else {
        client.add_device(INSTANCE, ROUTER).await.unwrap();
    }
    (
        client,
        Peer {
            transport: remote,
            frames,
            destinations,
            routed,
        },
    )
}

impl Peer {
    async fn request(&mut self, service: ConfirmedServiceChoice) -> ConfirmedRequest {
        let frame = timeout(Duration::from_secs(5), self.frames.recv())
            .await
            .expect("request watchdog")
            .unwrap();
        assert!(!frame.link_layer_group);
        assert_eq!(self.destinations.recv().await.unwrap().as_slice(), ROUTER);
        let npdu = decode_npdu(frame.npdu).unwrap();
        assert!(npdu.expecting_reply);
        assert_eq!(npdu.priority, NetworkPriority::Normal);
        if self.routed {
            let destination = npdu.destination.expect("object mutation lost DNET/DADR");
            assert_eq!(destination.network, NETWORK);
            assert_eq!(destination.mac_address.as_slice(), STATION);
        } else {
            assert!(npdu.destination.is_none());
        }
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected confirmed mutation");
        };
        assert_eq!(request.service_choice, service);
        request
    }
    async fn send_payload(&self, payload: Bytes) {
        let mut bytes = BytesMut::new();
        encode_npdu(
            &mut bytes,
            &Npdu {
                source: self.routed.then(|| NpduAddress {
                    network: NETWORK,
                    mac_address: MacAddr::from_slice(STATION),
                }),
                payload,
                ..Npdu::default()
            },
        )
        .unwrap();
        self.transport.send_unicast(&bytes, &[1]).await.unwrap();
    }
    async fn send(&self, apdu: Apdu) {
        let mut payload = BytesMut::new();
        apdu::encode_apdu(&mut payload, &apdu).unwrap();
        self.send_payload(payload.freeze()).await;
    }
    async fn ack(&self, request: &ConfirmedRequest) {
        self.send(Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
        }))
        .await;
    }
    async fn complex_ack(&self, request: &ConfirmedRequest, bytes: &[u8]) {
        self.send(Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: request.service_choice,
            service_ack: Bytes::copy_from_slice(bytes),
        }))
        .await;
    }
    fn empty(&mut self) {
        assert!(self.frames.try_recv().is_err());
        assert!(self.destinations.try_recv().is_err());
    }
}

fn object() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 77).unwrap()
}
fn initial_values() -> Vec<BACnetPropertyValue> {
    vec![
        BACnetPropertyValue {
            property_identifier: PropertyIdentifier::STATE_TEXT,
            property_array_index: Some(2),
            value: vec![0x73, 0, b'O', b'n'],
            priority: None,
        },
        BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x21, 2],
            priority: Some(8),
        },
    ]
}
fn time() -> BACnetTimeStamp {
    BACnetTimeStamp::Time(Time {
        hour: 13,
        minute: 27,
        second: 39,
        hundredths: 51,
    })
}
fn date_time() -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime {
        date: Date {
            year: 124,
            month: 7,
            day: 4,
            day_of_week: 4,
        },
        time: Time {
            hour: 19,
            minute: 17,
            second: 29,
            hundredths: 83,
        },
    }
}
fn alarm_request(
    index: u32,
    timestamp: BACnetTimeStamp,
    acknowledgment: BACnetTimeStamp,
) -> AcknowledgeAlarmRequest {
    AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 0x1234_5678 + index,
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 77 + index)
            .unwrap(),
        event_state_acknowledged: [
            EventState::HIGH_LIMIT,
            EventState::FAULT,
            EventState::NORMAL,
        ][index as usize],
        timestamp,
        acknowledgment_source: format!("console-{index}-λ"),
        time_of_acknowledgment: acknowledgment,
    }
}

#[tokio::test(start_paused = true)]
async fn object_mgmt_create_preserves_specifiers_initial_values_and_decodes_reply() {
    for routed in [true, false] {
        let (mut client, mut peer) = site(routed).await;
        for (specifier, values, reply) in [
            (
                ObjectSpecifier::Type(ObjectType::MULTI_STATE_VALUE),
                initial_values(),
                &[0xc4, 0x04, 0xc0, 0, 77][..],
            ),
            (
                ObjectSpecifier::Identifier(object()),
                initial_values(),
                &[0xff, 0][..],
            ),
            (ObjectSpecifier::Identifier(object()), vec![], &[][..]),
        ] {
            let (result, ()) = tokio::join!(
                client.create_object_from_device(INSTANCE, specifier.clone(), values.clone()),
                async {
                    let request = peer.request(ConfirmedServiceChoice::CREATE_OBJECT).await;
                    assert_eq!(
                        CreateObjectRequest::decode(&request.service_request).unwrap(),
                        CreateObjectRequest {
                            object_specifier: specifier,
                            list_of_initial_values: values
                        }
                    );
                    peer.complex_ack(&request, reply).await;
                }
            );
            // Only the first reply is a CreateObject-ACK: (MULTI_STATE_VALUE, 77).
            if reply.is_empty() || reply[0] != 0xc4 {
                assert!(result.is_err(), "{reply:02x?} must not decode");
            } else {
                assert_eq!(
                    result.unwrap(),
                    ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 77).unwrap()
                );
            }
        }
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn object_mgmt_delete_preserves_identity_and_local_fallback() {
    for routed in [true, false] {
        let (mut client, mut peer) = site(routed).await;
        let (result, ()) = tokio::join!(
            client.delete_object_from_device(INSTANCE, object()),
            async {
                let request = peer.request(ConfirmedServiceChoice::DELETE_OBJECT).await;
                assert_eq!(
                    DeleteObjectRequest::decode(&request.service_request).unwrap(),
                    DeleteObjectRequest {
                        object_identifier: object()
                    }
                );
                peer.ack(&request).await;
            }
        );
        result.unwrap();
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn object_mgmt_alarm_preserves_every_field_and_timestamp_choice() {
    for routed in [true, false] {
        let (mut client, mut peer) = site(routed).await;
        for (index, (timestamp, acknowledged_at)) in [
            (BACnetTimeStamp::SequenceNumber(65535), time()),
            (time(), date_time()),
            (date_time(), BACnetTimeStamp::SequenceNumber(17)),
        ]
        .into_iter()
        .enumerate()
        {
            let expected = alarm_request(index as u32, timestamp, acknowledged_at);
            let (result, ()) = tokio::join!(
                client.acknowledge_alarm_request_from_device(INSTANCE, &expected),
                async {
                    let request = peer
                        .request(ConfirmedServiceChoice::ACKNOWLEDGE_ALARM)
                        .await;
                    assert_eq!(
                        AcknowledgeAlarmRequest::decode(&request.service_request).unwrap(),
                        expected
                    );
                    peer.ack(&request).await;
                }
            );
            result.unwrap();
        }
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn object_mgmt_routed_structured_error_protocol_error_reject_and_abort_survive() {
    let (mut client, mut peer) = site(true).await;
    let (result, ()) = tokio::join!(
        client.create_object_from_device(
            INSTANCE,
            ObjectSpecifier::Identifier(object()),
            initial_values()
        ),
        async {
            let request = peer.request(ConfirmedServiceChoice::CREATE_OBJECT).await;
            // Independent error body: property/write-access-denied, initial value 2.
            let mut payload = vec![0x50, request.invoke_id, request.service_choice.to_raw()];
            payload.extend_from_slice(&[0x0e, 0x91, 2, 0x91, 40, 0x0f, 0x19, 2]);
            peer.send_payload(Bytes::from(payload)).await;
        }
    );
    let Err(Error::Structured {
        class,
        code,
        detail,
    }) = result
    else {
        panic!("expected detailed CreateObject error: {result:?}");
    };
    assert_eq!(class, ErrorClass::PROPERTY);
    assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!(*detail, ErrorDetail::FirstFailedElementNumber(2));
    let (result, ()) = tokio::join!(
        client.delete_object_from_device(INSTANCE, object()),
        async {
            let request = peer.request(ConfirmedServiceChoice::DELETE_OBJECT).await;
            peer.send(Apdu::Error(ErrorPdu {
                invoke_id: request.invoke_id,
                service_choice: request.service_choice,
                error_class: ErrorClass::OBJECT,
                error_code: ErrorCode::UNKNOWN_OBJECT,
                error_data: Bytes::new(),
            }))
            .await;
        }
    );
    assert!(matches!(
        result,
        Err(Error::Remote(RemoteError::Error {
            class: ErrorClass::OBJECT,
            code: ErrorCode::UNKNOWN_OBJECT
        }))
    ));
    let expected = alarm_request(0, time(), date_time());
    for abort in [false, true] {
        let (result, ()) = tokio::join!(
            client.acknowledge_alarm_request_from_device(INSTANCE, &expected),
            async {
                let request = peer
                    .request(ConfirmedServiceChoice::ACKNOWLEDGE_ALARM)
                    .await;
                if abort {
                    peer.send(Apdu::Abort(AbortPdu {
                        sent_by_server: true,
                        invoke_id: request.invoke_id,
                        abort_reason: AbortReason::OUT_OF_RESOURCES,
                    }))
                    .await;
                } else {
                    peer.send(Apdu::Reject(RejectPdu {
                        invoke_id: request.invoke_id,
                        reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
                    }))
                    .await;
                }
            }
        );
        if abort {
            assert!(matches!(
                result,
                Err(Error::Remote(RemoteError::Abort {
                    reason: AbortReason::OUT_OF_RESOURCES
                }))
            ));
        } else {
            assert!(matches!(
                result,
                Err(Error::Remote(RemoteError::Reject {
                    reason: RejectReason::UNRECOGNIZED_SERVICE
                }))
            ));
        }
    }
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn object_mgmt_unknown_device_and_oversized_alarm_send_nothing() {
    let (mut client, mut peer) = site(true).await;
    let mut request = alarm_request(0, time(), date_time());
    assert!(
        client
            .create_object_from_device(
                INSTANCE + 1,
                ObjectSpecifier::Identifier(object()),
                initial_values()
            )
            .await
            .is_err()
    );
    assert!(
        client
            .delete_object_from_device(INSTANCE + 1, object())
            .await
            .is_err()
    );
    assert!(
        client
            .acknowledge_alarm_request_from_device(INSTANCE + 1, &request)
            .await
            .is_err()
    );
    peer.empty();
    request.acknowledgment_source = "x".repeat(2048);
    let result = client
        .acknowledge_alarm_request_from_device(INSTANCE, &request)
        .await;
    assert!(matches!(
        result,
        Err(Error::Segmentation(SegmentationError::PeerCannotReceive {
            capability: Segmentation::NONE
        }))
    ));
    peer.empty();
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}
