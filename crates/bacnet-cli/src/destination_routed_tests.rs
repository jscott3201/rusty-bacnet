//! A routed MAC target whose instance another device claims too (#2560):
//! `2:0x05` resolves to its station with nothing sent, and every service
//! family the one-shot commands forward reaches it through its router,
//! while the instance alone is refused as ambiguous. An address the
//! discovery table doesn't know needs no instance.

use bacnet_client::client::ClientConfig;
use bacnet_encoding::apdu::{Apdu, ErrorPdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_services::read_range::ReadRangeRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::enums::{
    AcknowledgmentFilter, ConfirmedServiceChoice as S, EnableDisable, ErrorClass, ErrorCode,
    EventState, ObjectType, ReinitializedState, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::*;
use bacnet_client::log_reader::LogCursor;
use bacnet_services::enrollment_summary::GetEnrollmentSummaryRequest;
use bacnet_services::file::{FileAccessMethod, FileWriteAccessMethod};
use bacnet_services::read_range::ReadRangeValidation;
use bacnet_services::wpm::WriteAccessSpecification;

pub(super) const CLIENT: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
/// The local device claiming the instance.
const LOCAL: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];
pub(super) const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];
const DNET: u16 = 2;
const STATION: u8 = 0x05;
const INSTANCE: u32 = 1234;

/// Every confirmed request answered with an Error PDU from where it went:
/// through the router, from the station it named.
pub(super) fn answer(frame: &SentFrame) -> Option<ReceivedNpdu> {
    if frame.broadcast {
        return None;
    }
    let npdu = frame.decode_npdu();
    let Ok(Apdu::ConfirmedRequest(request)) = bacnet_encoding::apdu::decode_apdu(npdu.payload)
    else {
        return None;
    };
    let error = Apdu::Error(ErrorPdu {
        invoke_id: request.invoke_id,
        service_choice: request.service_choice,
        error_class: ErrorClass::OBJECT,
        error_code: ErrorCode::UNKNOWN_OBJECT,
        error_data: Bytes::new(),
    });
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, &error).unwrap();
    let source = npdu.destination.map(|to| NpduAddress {
        network: to.network,
        mac_address: to.mac_address,
    });
    let back = Npdu {
        source,
        payload: payload.freeze(),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &back).unwrap();
    let from = frame.mac.clone();
    Some(ReceivedNpdu::unverified(
        buf.freeze(),
        from,
        false,
        Vec::new(),
        None,
    ))
}

/// A client whose table holds the instance at a local MAC and at a routed
/// station, with the scripted router answering for the station.
async fn site() -> (BACnetClient<TestTransport>, SendLog) {
    let (inbox, inbound) = mpsc::channel(32);
    let transport = TestTransport::builder()
        .local_mac(&CLIENT)
        .inbound(inbound)
        .on_send(move |frame| {
            let inbox = inbox.clone();
            async move {
                if let Some(reply) = answer(&frame) {
                    let _ = inbox.send(reply).await;
                }
                Ok(())
            }
        })
        .build();
    let sent = transport.sent();
    let config = ClientConfig {
        apdu_timeout_ms: 500,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, transport).await.unwrap();
    client.add_device(INSTANCE, &LOCAL).await.unwrap();
    client
        .add_routed_device(bacnet_client::discovery::RoutedDeviceConfig {
            instance: INSTANCE,
            router_mac: ROUTER.to_vec(),
            remote_network: DNET,
            remote_mac: vec![STATION],
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    (client, sent)
}

/// A queued routed I-Am and its local table row designate one station
/// after the client's own network number becomes known.
#[tokio::test(start_paused = true)]
async fn a_queued_routed_answer_and_its_local_alias_match_the_canonical_destination() {
    let (mut client, _) = site().await;
    let mut queued = client.get_device_at(DNET, &[STATION]).await.unwrap();
    let mut local = queued.clone();
    local.mac_address = queued.source_address.clone().unwrap();
    local.source_network = None;
    local.source_address = None;
    let expected = Destination {
        address: DeviceAddress::local(&[STATION]).unwrap(),
        instance: Some(INSTANCE),
    };
    let mac = || Station::Mac {
        mac: vec![STATION],
        one_station: false,
    };
    for target in [Station::Instance(INSTANCE), mac()] {
        assert_eq!(
            target.destination(DNET, Some(DNET), &queued),
            Some(expected.clone())
        );
        assert_eq!(
            target.known(DNET, Some(DNET), std::slice::from_ref(&local)),
            Some(Ok(expected.clone()))
        );
    }
    queued.object_identifier = ObjectIdentifier::new(ObjectType::DEVICE, INSTANCE + 1).unwrap();
    let stale = Station::Instance(INSTANCE + 1);
    assert!(stale.destination(DNET, Some(DNET), &queued).is_some());
    assert!(
        stale
            .known(DNET, Some(DNET), std::slice::from_ref(&local))
            .is_none()
    );
    assert_eq!(
        mac().known(DNET, Some(DNET), std::slice::from_ref(&local)),
        Some(Ok(expected))
    );
    let mut different = local.clone();
    different.mac_address = bacnet_types::MacAddr::from_slice(&[STATION + 1]);
    assert!(
        Station::Instance(INSTANCE)
            .known(DNET, Some(DNET), &[local, different])
            .unwrap()
            .is_err()
    );
    client.stop().await.unwrap();
}

/// The device's Error, as every call to the station should end.
pub(super) fn refused<V: std::fmt::Debug>(result: Result<V, Error>) -> bool {
    matches!(
        result.as_ref().map_err(Error::remote),
        Err(Some(bacnet_types::error::RemoteError::Error {
            code: ErrorCode::UNKNOWN_OBJECT,
            ..
        }))
    )
}

/// One unicast: its link MAC, its DNET and DADR when routed, and its
/// confirmed service.
type Sent = (Vec<u8>, Option<(u16, Vec<u8>)>, Option<S>);

/// Where each unicast went, and its service: through the router to the
/// station, never to the local device claiming the same instance.
pub(super) fn routed(sent: &SendLog) -> Vec<Sent> {
    sent.unicasts()
        .iter()
        .map(|frame| {
            let npdu = frame.decode_npdu();
            let to = npdu
                .destination
                .map(|to| (to.network, to.mac_address.to_vec()));
            let service = match bacnet_encoding::apdu::decode_apdu(npdu.payload) {
                Ok(Apdu::ConfirmedRequest(request)) => Some(request.service_choice),
                _ => None,
            };
            (frame.mac.to_vec(), to, service)
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_routed_station_with_a_contested_instance_gets_every_service() {
    let (client, sent) = site().await;
    let wait = Duration::from_secs(1);
    let ambiguous = resolve(&client, "1234", wait).await.unwrap_err();
    assert!(ambiguous.contains("2:0x05"), "{ambiguous}");
    let to = resolve(&client, "2:0x05", wait).await.unwrap();
    assert_eq!(to.address.to_string(), "2:0x05");
    assert_eq!(to.instance, Some(INSTANCE));
    assert!(sent.is_empty(), "resolving the address sent nothing");

    let object = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    let file = ObjectIdentifier::new(ObjectType::FILE, 1).unwrap();
    let log = ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let spec = ReadAccessSpecification {
        object_identifier: object,
        list_of_property_references: vec![PropertyReference {
            property_identifier: pv,
            property_array_index: None,
        }],
    };
    let read_range = ReadRangeRequest {
        object_identifier: log,
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: None,
    };
    let ack = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: object,
        event_state_acknowledged: EventState::OFFNORMAL,
        timestamp: BACnetTimeStamp::SequenceNumber(1),
        acknowledgment_source: "cli".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(2),
    };
    let enrollment = GetEnrollmentSummaryRequest {
        acknowledgment_filter: AcknowledgmentFilter::ALL,
        enrollment_filter: None,
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    };
    let stream = FileAccessMethod::Stream {
        file_start_position: 0,
        requested_octet_count: 16,
    };
    let write = FileWriteAccessMethod::Stream {
        file_start_position: 0,
        file_data: vec![1, 2, 3],
    };
    let wpm = vec![WriteAccessSpecification {
        object_identifier: object,
        list_of_properties: vec![bacnet_services::common::BACnetPropertyValue {
            property_identifier: pv,
            property_array_index: None,
            value: vec![0x44, 0, 0, 0, 0],
            priority: Some(12),
        }],
    }];
    let specifier = ObjectSpecifier::Type(ObjectType::ANALOG_VALUE);
    let real = || vec![0x44, 0, 0, 0, 0];

    // One call per forwarded method, family by family.
    let results = [
        // Files.
        refused(to.atomic_read_file(&client, file, stream).await),
        refused(to.atomic_write_file(&client, file, write).await),
        // Objects.
        refused(to.create_object(&client, specifier, Vec::new()).await),
        refused(to.delete_object(&client, object).await),
        // Device management.
        refused(
            to.device_communication_control(&client, EnableDisable::DISABLE, Some(5), None)
                .await,
        ),
        refused(
            to.reinitialize_device(&client, ReinitializedState::WARMSTART, None)
                .await,
        ),
        // Alarms and events.
        refused(to.acknowledge_alarm(&client, &ack).await),
        refused(to.get_event_information(&client, None).await),
        refused(to.get_alarm_summary(&client).await),
        refused(to.get_enrollment_summary(&client, &enrollment).await),
        // Ranges and logs.
        refused(
            to.read_range(&client, &read_range, ReadRangeValidation::Strict)
                .await,
        ),
        refused(to.read_log_page(&client, log, LogCursor::Oldest, 10).await),
        // Reads.
        refused(to.read_property(&client, object, pv, None).await),
        refused(to.read_property_multiple(&client, vec![spec]).await),
        // Writes.
        refused(
            to.write_property(&client, object, pv, None, real(), Some(12))
                .await,
        ),
        refused(to.write_property_multiple(&client, wpm).await),
        // Change of value.
        refused(to.subscribe_cov(&client, 7, object, false, Some(60)).await),
        refused(to.unsubscribe_cov(&client, 7, object).await),
    ];
    assert!(results.iter().all(|ok| *ok), "{results:?}");
    let station = Some((DNET, vec![STATION]));
    let services: Vec<S> = routed(&sent)
        .into_iter()
        .map(|(mac, to, service)| {
            assert_eq!((mac.as_slice(), &to), (ROUTER.as_slice(), &station));
            service.unwrap()
        })
        .collect();
    assert_eq!(
        services,
        [
            S::ATOMIC_READ_FILE,
            S::ATOMIC_WRITE_FILE,
            S::CREATE_OBJECT,
            S::DELETE_OBJECT,
            S::DEVICE_COMMUNICATION_CONTROL,
            S::REINITIALIZE_DEVICE,
            S::ACKNOWLEDGE_ALARM,
            S::GET_EVENT_INFORMATION,
            S::GET_ALARM_SUMMARY,
            S::GET_ENROLLMENT_SUMMARY,
            S::READ_RANGE,
            S::READ_PROPERTY,
            S::READ_PROPERTY,
            S::READ_PROPERTY_MULTIPLE,
            S::WRITE_PROPERTY,
            S::WRITE_PROPERTY_MULTIPLE,
            S::SUBSCRIBE_COV,
            S::SUBSCRIBE_COV,
        ],
        "the log page's first read is its record count"
    );

    // Time synchronization: unconfirmed, once, through the router.
    sent.clear();
    let date = Date {
        year: 126,
        month: 11,
        day: 14,
        day_of_week: 6,
    };
    let time = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    to.time_synchronization(&client, date, time, false)
        .await
        .unwrap();
    to.time_synchronization(&client, date, time, true)
        .await
        .unwrap();
    let frames = sent.unicasts();
    let kinds: Vec<UnconfirmedServiceChoice> = frames
        .iter()
        .map(|frame| {
            assert_eq!(frame.mac.as_slice(), ROUTER);
            let to = frame.decode_npdu().destination.unwrap();
            assert_eq!(
                (to.network, to.mac_address.as_slice()),
                (DNET, &[STATION][..])
            );
            match frame.apdu() {
                Apdu::UnconfirmedRequest(request) => request.service_choice,
                other => panic!("{other:?}"),
            }
        })
        .collect();
    assert_eq!(
        kinds,
        [
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
            UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
        ]
    );
}

/// An address the discovery table doesn't know needs no instance: it is
/// sent to at its MAC, with nothing sent to resolve it first.
#[tokio::test(start_paused = true)]
async fn an_address_needs_no_instance() {
    let (client, sent) = site().await;
    let to = resolve(&client, "10.0.0.7:47808", Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(to.instance, None);
    assert!(sent.is_empty());
    let object = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    let read = to
        .read_property(&client, object, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    assert!(refused(read));
    let frames = routed(&sent);
    assert_eq!(
        frames,
        [(vec![10, 0, 0, 7, 0xBA, 0xC0], None, Some(S::READ_PROPERTY))]
    );
}
