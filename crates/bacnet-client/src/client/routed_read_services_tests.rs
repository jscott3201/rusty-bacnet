//! Routed read services retain the remote peer, request fields and reply checks.
use bacnet_encoding::apdu::{self, Apdu, ComplexAck, ConfirmedRequest};
use bacnet_encoding::constructed::encode_log_record;
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_encoding::primitives::encode_app_unsigned;
use bacnet_services::alarm_event::GetEventInformationRequest;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::read_range::{
    LogRecords, RangeSpec, ReadRangeAck, ReadRangeRequest, ReadRangeValidation, ReadRangeViolation,
};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

use super::BACnetClient;
use crate::discovery::RoutedDeviceConfig;
use crate::log_reader::LogCursor;

const INSTANCE: u32 = 1234;
const CLIENT: &[u8] = &[1];
const ROUTER: &[u8] = &[2];
const DNET: u16 = 100;
const DADR: &[u8] = &[5];

struct Peer {
    transport: LoopbackTransport,
    frames: mpsc::Receiver<ReceivedNpdu>,
    destinations: mpsc::UnboundedReceiver<MacAddr>,
    routed: bool,
}

fn routed_config() -> RoutedDeviceConfig {
    RoutedDeviceConfig {
        instance: INSTANCE,
        router_mac: ROUTER.to_vec(),
        remote_network: DNET,
        remote_mac: DADR.to_vec(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
    }
}

async fn site(routed: bool) -> (BACnetClient<LoopbackTransport>, Peer) {
    let (mut transport, mut peer) = LoopbackTransport::pair(CLIENT, ROUTER);
    let destinations = transport.record_unicast_destinations();
    let frames = peer.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(transport)
        .apdu_timeout_ms(1000)
        .apdu_retries(0)
        .build()
        .await
        .unwrap();
    if routed {
        client.add_routed_device(routed_config()).await.unwrap();
    } else {
        client.add_device(INSTANCE, ROUTER).await.unwrap();
    }
    (
        client,
        Peer {
            transport: peer,
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
            .expect("client stopped");
        assert!(!frame.link_layer_group, "confirmed request must be unicast");
        assert_eq!(
            self.destinations.recv().await.unwrap().as_slice(),
            ROUTER,
            "the immediate next hop must remain the router, not the remote station"
        );
        let npdu = decode_npdu(frame.npdu).unwrap();
        if self.routed {
            let destination = npdu.destination.expect("remote request lost DNET/DADR");
            assert_eq!(destination.network, DNET);
            assert_eq!(destination.mac_address.as_slice(), DADR);
        } else {
            assert!(npdu.destination.is_none(), "local device acquired a route");
        }
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected confirmed request");
        };
        assert_eq!(request.service_choice, service);
        request
    }

    async fn reply(&self, request: &ConfirmedRequest, data: Bytes) {
        let mut apdu = BytesMut::new();
        apdu::encode_apdu(
            &mut apdu,
            &Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: request.service_choice,
                service_ack: data,
            }),
        )
        .unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                source: self.routed.then(|| NpduAddress {
                    network: DNET,
                    mac_address: MacAddr::from_slice(DADR),
                }),
                payload: apdu.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.transport.send_unicast(&npdu, CLIENT).await.unwrap();
    }
}

fn log() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

fn request() -> ReadRangeRequest {
    ReadRangeRequest {
        object_identifier: log(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::BySequenceNumber {
            reference_seq: 1,
            count: 1,
        }),
    }
}

fn record() -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 10,
            day: 3,
            day_of_week: 6,
        },
        time: Time {
            hour: 8,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::UnsignedValue(71),
        status_flags: None,
    }
}

fn range_ack(sequence: u64) -> ReadRangeAck {
    let mut data = BytesMut::new();
    encode_log_record(&record(), &mut data).unwrap();
    ReadRangeAck {
        object_identifier: log(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (false, false, false),
        item_count: 1,
        item_data: data.to_vec(),
        first_sequence_number: Some(sequence),
    }
}

fn encode_range(ack: &ReadRangeAck) -> Bytes {
    let mut bytes = BytesMut::new();
    ack.encode(&mut bytes);
    bytes.freeze()
}

fn assert_record(records: LogRecords) {
    let LogRecords::TrendLog(records) = records else {
        panic!("expected Trend Log records")
    };
    assert_eq!(records, vec![record()]);
}

#[tokio::test(start_paused = true)]
async fn routed_event_information_preserves_cursor_and_decodes_the_ack() {
    let (mut client, mut peer) = site(true).await;
    for cursor in [None, Some(log())] {
        let (result, ()) = tokio::join!(
            client.get_event_information_from_device(INSTANCE, cursor),
            async {
                let request = peer
                    .request(ConfirmedServiceChoice::GET_EVENT_INFORMATION)
                    .await;
                assert_eq!(
                    GetEventInformationRequest::decode(&request.service_request)
                        .unwrap()
                        .last_received_object_identifier,
                    cursor
                );
                peer.reply(&request, Bytes::from_static(&[0x0e, 0x0f, 0x19, 0]))
                    .await;
            }
        );
        let ack = result.unwrap();
        assert!(ack.list_of_event_summaries.is_empty());
        assert!(!ack.more_events);
    }
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn routed_read_range_preserves_strict_lenient_and_decode_failures() {
    let (mut client, mut peer) = site(true).await;
    let expected = request();
    let (result, ()) = tokio::join!(
        client.read_range_from_device(
            INSTANCE,
            log(),
            PropertyIdentifier::LOG_BUFFER,
            None,
            expected.range.clone()
        ),
        async {
            let request = peer.request(ConfirmedServiceChoice::READ_RANGE).await;
            assert_eq!(
                ReadRangeRequest::decode(&request.service_request).unwrap(),
                expected
            );
            peer.reply(&request, encode_range(&range_ack(0))).await;
        }
    );
    assert!(matches!(
        result,
        Err(Error::ReadRangeViolation(
            ReadRangeViolation::ZeroFirstSequenceNumber
        ))
    ));
    for mode in [ReadRangeValidation::Strict, ReadRangeValidation::Lenient] {
        let (result, ()) = tokio::join!(
            client.read_range_from_device_with(INSTANCE, &expected, mode),
            async {
                let request = peer.request(ConfirmedServiceChoice::READ_RANGE).await;
                assert_eq!(
                    ReadRangeRequest::decode(&request.service_request).unwrap(),
                    expected
                );
                peer.reply(&request, encode_range(&range_ack(0))).await;
            }
        );
        if mode == ReadRangeValidation::Strict {
            assert!(matches!(
                result,
                Err(Error::ReadRangeViolation(
                    ReadRangeViolation::ZeroFirstSequenceNumber
                ))
            ));
        } else {
            let reply = result.unwrap();
            assert_eq!(
                reply.violations,
                vec![ReadRangeViolation::ZeroFirstSequenceNumber]
            );
            assert_record(reply.ack.log_records().unwrap().unwrap());
        }
        let (result, ()) = tokio::join!(
            client.read_range_from_device_with(INSTANCE, &expected, mode),
            async {
                let request = peer.request(ConfirmedServiceChoice::READ_RANGE).await;
                peer.reply(&request, Bytes::from_static(&[0x0c, 0])).await;
            }
        );
        assert!(
            matches!(
                result,
                Err(Error::Decoding(_) | Error::BufferTooShort { .. })
            ),
            "{result:?}"
        );
    }
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

/// Counts and data must keep the page's route even if discovery changes mid-page.
#[tokio::test(start_paused = true)]
async fn routed_log_page_uses_one_route_snapshot_for_counts_and_range() {
    let (mut client, mut peer) = site(true).await;
    let (result, ()) = tokio::join!(
        client.read_log_page_from_device(INSTANCE, log(), LogCursor::Oldest, 1),
        async {
            for (index, property) in [
                PropertyIdentifier::TOTAL_RECORD_COUNT,
                PropertyIdentifier::RECORD_COUNT,
                PropertyIdentifier::TOTAL_RECORD_COUNT,
            ]
            .into_iter()
            .enumerate()
            {
                let request = peer.request(ConfirmedServiceChoice::READ_PROPERTY).await;
                let decoded = ReadPropertyRequest::decode(&request.service_request).unwrap();
                assert_eq!(decoded.object_identifier, log());
                assert_eq!(decoded.property_identifier, property);
                assert_eq!(decoded.property_array_index, None);
                if index == 0 {
                    client
                        .add_routed_device(RoutedDeviceConfig {
                            router_mac: vec![9],
                            remote_network: 200,
                            remote_mac: vec![8],
                            ..routed_config()
                        })
                        .await
                        .unwrap();
                }
                reply_count(&peer, &request, property).await;
            }
            let request = peer.request(ConfirmedServiceChoice::READ_RANGE).await;
            assert_eq!(
                ReadRangeRequest::decode(&request.service_request).unwrap(),
                self::request()
            );
            peer.reply(&request, encode_range(&range_ack(1))).await;
        }
    );
    let page = result.unwrap();
    assert_record(page.records);
    assert_eq!(page.first_sequence_number, Some(1));
    assert_eq!(page.next, LogCursor::Sequence(2));
    assert!(!page.done);
    assert!(page.gap.is_none() && page.violations.is_empty());
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

async fn reply_count(peer: &Peer, request: &ConfirmedRequest, property: PropertyIdentifier) {
    let mut value = BytesMut::new();
    encode_app_unsigned(&mut value, 3);
    let mut bytes = BytesMut::new();
    ReadPropertyACK {
        object_identifier: log(),
        property_identifier: property,
        property_array_index: None,
        property_value: value.to_vec(),
    }
    .encode(&mut bytes);
    peer.reply(request, bytes.freeze()).await;
}

#[tokio::test(start_paused = true)]
async fn read_service_device_methods_keep_local_devices_direct() {
    let (mut client, mut peer) = site(false).await;
    let (result, ()) = tokio::join!(
        client.get_event_information_from_device(INSTANCE, None),
        async {
            let request = peer
                .request(ConfirmedServiceChoice::GET_EVENT_INFORMATION)
                .await;
            assert!(request.service_request.is_empty());
            peer.reply(&request, Bytes::from_static(&[0x0e, 0x0f, 0x19, 0]))
                .await;
        }
    );
    let ack = result.unwrap();
    assert!(ack.list_of_event_summaries.is_empty() && !ack.more_events);
    let (result, ()) = tokio::join!(
        client.read_range_from_device(
            INSTANCE,
            log(),
            PropertyIdentifier::LOG_BUFFER,
            None,
            request().range
        ),
        async {
            let request = peer.request(ConfirmedServiceChoice::READ_RANGE).await;
            assert_eq!(
                ReadRangeRequest::decode(&request.service_request).unwrap(),
                self::request()
            );
            peer.reply(&request, encode_range(&range_ack(1))).await;
        }
    );
    assert_record(result.unwrap().log_records().unwrap().unwrap());
    let (result, ()) = tokio::join!(
        client.read_log_page_from_device(INSTANCE, log(), LogCursor::Oldest, 1),
        async {
            for property in [
                PropertyIdentifier::TOTAL_RECORD_COUNT,
                PropertyIdentifier::RECORD_COUNT,
                PropertyIdentifier::TOTAL_RECORD_COUNT,
            ] {
                let request = peer.request(ConfirmedServiceChoice::READ_PROPERTY).await;
                assert_eq!(
                    ReadPropertyRequest::decode(&request.service_request)
                        .unwrap()
                        .property_identifier,
                    property
                );
                reply_count(&peer, &request, property).await;
            }
            let request = peer.request(ConfirmedServiceChoice::READ_RANGE).await;
            peer.reply(&request, encode_range(&range_ack(1))).await;
        }
    );
    let page = result.unwrap();
    assert_record(page.records);
    assert_eq!(page.next, LogCursor::Sequence(2));
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn missing_read_service_device_and_invalid_page_fail_without_traffic() {
    let (mut client, mut peer) = site(true).await;
    assert!(
        client
            .get_event_information_from_device(INSTANCE + 1, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("not in device table")
    );
    assert!(
        client
            .read_range_from_device(
                INSTANCE + 1,
                log(),
                PropertyIdentifier::LOG_BUFFER,
                None,
                request().range
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("not in device table")
    );
    assert!(
        client
            .read_range_from_device_with(INSTANCE + 1, &request(), ReadRangeValidation::Lenient)
            .await
            .unwrap_err()
            .to_string()
            .contains("not in device table")
    );
    assert!(
        client
            .read_log_page_from_device(INSTANCE + 1, log(), LogCursor::Oldest, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("not in device table")
    );
    assert!(
        client
            .read_log_page_from_device(INSTANCE, log(), LogCursor::Oldest, 0)
            .await
            .is_err()
    );
    assert!(peer.frames.try_recv().is_err());
    assert!(peer.destinations.try_recv().is_err());
    client.stop().await.unwrap();
    peer.transport.stop().await.unwrap();
}

/// The alarm and enrollment summaries go through the router to a routed
/// device and straight to a local one, and come back decoded (#2077).
#[tokio::test(start_paused = true)]
async fn summaries_route_when_the_device_is_routed_and_decode() {
    use bacnet_services::alarm_summary::{AlarmSummaryEntry, GetAlarmSummaryAck};
    use bacnet_services::enrollment_summary::{
        GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest,
    };
    use bacnet_types::bitstring::EventTransitionBits;
    use bacnet_types::enums::{AcknowledgmentFilter, EventState};

    let entry = AlarmSummaryEntry {
        object_identifier: log(),
        alarm_state: EventState::FAULT,
        acknowledged_transitions: EventTransitionBits::TO_NORMAL,
    };
    let filters = GetEnrollmentSummaryRequest {
        acknowledgment_filter: AcknowledgmentFilter::NOT_ACKED,
        enrollment_filter: None,
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: Some(4),
    };
    for routed in [true, false] {
        let (mut client, mut peer) = site(routed).await;
        let (result, ()) = tokio::join!(client.get_alarm_summary_from_device(INSTANCE), async {
            let request = peer
                .request(ConfirmedServiceChoice::GET_ALARM_SUMMARY)
                .await;
            assert!(request.service_request.is_empty());
            let mut ack = BytesMut::new();
            GetAlarmSummaryAck {
                entries: vec![entry.clone()],
            }
            .encode(&mut ack);
            peer.reply(&request, ack.freeze()).await;
        });
        assert_eq!(
            result.unwrap().entries,
            std::slice::from_ref(&entry),
            "routed {routed}"
        );
        let (result, ()) = tokio::join!(
            client.get_enrollment_summary_from_device(INSTANCE, &filters),
            async {
                let request = peer
                    .request(ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY)
                    .await;
                assert_eq!(
                    GetEnrollmentSummaryRequest::decode(&request.service_request).unwrap(),
                    filters
                );
                let mut ack = BytesMut::new();
                GetEnrollmentSummaryAck { entries: vec![] }.encode(&mut ack);
                peer.reply(&request, ack.freeze()).await;
            }
        );
        assert!(result.unwrap().entries.is_empty(), "routed {routed}");
        client.stop().await.unwrap();
        peer.transport.stop().await.unwrap();
    }
}
