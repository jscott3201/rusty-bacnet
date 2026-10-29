use bacnet_types::error::RemoteError;
use std::borrow::Cow;

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::traits::BACnetObject;

use super::*;

struct MalformedAlarmObject {
    oid: ObjectIdentifier,
    name: String,
    malformed: bool,
}

impl BACnetObject for MalformedAlarmObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::EVENT_STATE => Ok(if self.malformed {
                PropertyValue::Boolean(true)
            } else {
                PropertyValue::Enumerated(2)
            }),
            PropertyIdentifier::NOTIFY_TYPE => {
                Ok(PropertyValue::Enumerated(NotifyType::ALARM.to_raw()))
            }
            PropertyIdentifier::ACKED_TRANSITIONS => Ok(PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe0],
            }),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::ACKED_TRANSITIONS,
        ])
    }
}

#[tokio::test]
async fn projection_operational_problem_dispatches_error_apdu() {
    let response = alarm_summary_response(1, true, ServerConfig::default(), 480, false).await;
    let Apdu::Error(error) = response else {
        panic!("expected GetAlarmSummary Error APDU");
    };
    assert_eq!(error.invoke_id, 0x50);
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::GET_ALARM_SUMMARY
    );
    assert_eq!(error.error_class, ErrorClass::DEVICE);
    assert_eq!(error.error_code, ErrorCode::OPERATIONAL_PROBLEM);
}

#[tokio::test]
async fn alarm_summary_default_work_budget_precedes_projection() {
    let response = alarm_summary_response(4097, true, ServerConfig::default(), 480, false).await;
    let Apdu::Abort(abort) = response else {
        panic!("expected work-budget Abort before malformed projection, got {response:?}");
    };
    assert!(abort.sent_by_server);
    assert_eq!(abort.invoke_id, 0x50);
    assert_eq!(abort.abort_reason, AbortReason::OUT_OF_RESOURCES);
}

#[tokio::test]
async fn alarm_summary_wire_byte_budget_is_not_peer_apdu_size() {
    for peer in [50, 480] {
        for segmented in [false, true] {
            for (cap, expected) in [(19, Some(AbortReason::BUFFER_OVERFLOW)), (20, None)] {
                let config = ServerConfig {
                    get_alarm_summary_budget: GetAlarmSummaryBudget {
                        max_objects: 2,
                        max_service_ack_bytes: cap,
                    },
                    ..Default::default()
                };
                let response = alarm_summary_response(2, false, config, peer, segmented).await;
                match (response, expected) {
                    (Apdu::Abort(a), Some(reason)) => {
                        assert!(a.sent_by_server);
                        assert_eq!(a.invoke_id, 0x50);
                        assert_eq!(a.abort_reason, reason);
                    }
                    (Apdu::ComplexAck(a), None) => assert_eq!(a.service_ack.len(), 20),
                    other => panic!("unexpected response: {other:?}"),
                }
            }
        }
    }
}

async fn alarm_summary_response(
    count: u32,
    malformed: bool,
    config: ServerConfig,
    max_apdu_length: u16,
    segmented_response_accepted: bool,
) -> Apdu {
    summary_response(
        count,
        malformed,
        config,
        max_apdu_length,
        segmented_response_accepted,
        ConfirmedServiceChoice::GET_ALARM_SUMMARY,
        Bytes::new(),
    )
    .await
}

async fn summary_response(
    count: u32,
    malformed: bool,
    config: ServerConfig,
    max_apdu_length: u16,
    segmented_response_accepted: bool,
    service_choice: ConfirmedServiceChoice,
    service_request: Bytes,
) -> Apdu {
    let mut database = ObjectDatabase::new();
    for instance in 1..=count {
        database
            .add(Box::new(MalformedAlarmObject {
                oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
                name: format!("MALFORMED-ALARM-{instance}"),
                malformed,
            }))
            .unwrap();
    }
    let db = Arc::new(RwLock::new(database));
    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let confirmed_request_tracker = Arc::new(ConfirmedRequestTracker::default());
    let confirmed = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted,
        max_segments: None,
        max_apdu_length,
        invoke_id: 0x50,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_request,
    };
    let (tx, rx) = oneshot::channel();
    let route = Some(NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[9]),
    });

    BACnetServer::<BipTransport>::handle_confirmed_request(
        &RequestServices {
            db: Arc::clone(&db),
            ..RequestServices::for_test(Arc::clone(&network), config.clone())
        },
        &confirmed_request_tracker,
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        &MacAddr::from_slice(&[1]),
        route.clone(),
        confirmed,
        Some(tx),
    )
    .await;

    let npdu = decode_npdu(rx.await.unwrap()).unwrap();
    assert_eq!(npdu.destination, route);
    decode_apdu(npdu.payload).unwrap()
}

#[tokio::test]
async fn enrollment_summary_routed_work_abort_and_decode_error_precedence() {
    for peer in [50, 480] {
        for segmented in [false, true] {
            let response = summary_response(
                4097,
                false,
                ServerConfig::default(),
                peer,
                segmented,
                ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
                Bytes::from_static(&[9, 0]),
            )
            .await;
            assert!(
                matches!(response, Apdu::Abort(a) if a.sent_by_server && a.invoke_id == 0x50 && a.abort_reason == AbortReason::OUT_OF_RESOURCES)
            );
            for request in [&[0x19, 0][..], &[9, 3], &[9, 0, 0x4e, 9, 2, 0x19, 1, 0x4f]] {
                let baseline = summary_response(
                    0,
                    false,
                    ServerConfig::default(),
                    peer,
                    segmented,
                    ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
                    Bytes::copy_from_slice(request),
                )
                .await;
                let over = summary_response(
                    4097,
                    false,
                    ServerConfig::default(),
                    peer,
                    segmented,
                    ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
                    Bytes::copy_from_slice(request),
                )
                .await;
                assert_eq!(format!("{baseline:?}"), format!("{over:?}"));
                assert!(!matches!(over, Apdu::ComplexAck(_) | Apdu::Abort(_)));
            }
        }
    }
}

#[tokio::test]
async fn alarm_summary_within_budget_segmented_bip_roundtrip() {
    use bacnet_client::client::BACnetClient;
    for (work, bytes, expected) in [
        (20, 200, None),
        (20, 199, Some(AbortReason::BUFFER_OVERFLOW)),
        (19, 200, Some(AbortReason::OUT_OF_RESOURCES)),
    ] {
        let mut database = ObjectDatabase::new();
        for instance in 1..=20 {
            database
                .add(Box::new(MalformedAlarmObject {
                    oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
                    name: format!("alarm-{instance}"),
                    malformed: false,
                }))
                .unwrap();
        }
        let mut server = BACnetServer::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .database(database)
            .device(bacnet_objects::DeviceIdentity::default().with_segmentation(Segmentation::BOTH))
            .get_alarm_summary_budget(GetAlarmSummaryBudget {
                max_objects: work,
                max_service_ack_bytes: bytes,
            })
            .build()
            .await
            .unwrap();
        let mut client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .max_apdu_length(50)
            .build()
            .await
            .unwrap();
        let result = client
            .confirmed_request(
                server.local_mac(),
                ConfirmedServiceChoice::GET_ALARM_SUMMARY,
                &[],
            )
            .await;
        client.stop().await.unwrap();
        server.stop().await.unwrap();
        match expected {
            Some(reason) => {
                assert!(
                    matches!(result, Err(Error::Remote(RemoteError::Abort { reason: actual })) if actual == reason)
                )
            }
            None => {
                let response = result.unwrap();
                assert_eq!(
                    response.len(),
                    200,
                    "must exceed the peer's 50-byte APDU and reassemble completely"
                );
                let ack =
                    bacnet_services::alarm_summary::GetAlarmSummaryAck::decode(&response).unwrap();
                assert_eq!(ack.entries.len(), 20);
            }
        }
    }
}
