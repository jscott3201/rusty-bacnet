//! The client executes both COV-multiple notification services (#2654,
//! Clauses 13.17 and 13.18): each is published on `cov_notifications()` as
//! [`COVEvent::Multiple`], in arrival order beside the single-object ones,
//! and the confirmed form is answered as a ConfirmedCOVNotification is.
use super::cov_notification_tests::{
    cov_notification, receive_peer_npdu_apdu, send_cov_notification,
};
use super::*;
use bacnet_encoding::apdu::UnconfirmedRequest;
use bacnet_services::cov_multiple::{
    COVNotificationItem, COVNotificationMultipleRequest, COVNotificationValue,
};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{AbortReason, ObjectType, PropertyIdentifier, RejectReason};
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

const CHANGED: Time = Time {
    hour: 9,
    minute: 30,
    second: 5,
    hundredths: 25,
};

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn value(property: PropertyIdentifier, octets: &[u8], timestamped: bool) -> COVNotificationValue {
    COVNotificationValue {
        property_identifier: property,
        property_array_index: None,
        value: octets.to_vec(),
        time_of_change: timestamped.then_some(CHANGED),
    }
}

/// A COVNotificationMultiple from device 100 for `process`: AI 1's
/// Present_Value and Status_Flags, then AV 2's Present_Value, each with a
/// time of change and the notification with a timestamp when `timestamped`.
fn multiple(process: u32, timestamped: bool) -> COVNotificationMultipleRequest {
    COVNotificationMultipleRequest {
        subscriber_process_identifier: process,
        initiating_device_identifier: oid(ObjectType::DEVICE, 100),
        time_remaining: 120,
        timestamp: timestamped.then_some((
            Date {
                year: 126,
                month: 11,
                day: 16,
                day_of_week: 1,
            },
            CHANGED,
        )),
        list_of_cov_notifications: vec![
            COVNotificationItem {
                monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
                list_of_values: vec![
                    value(
                        PropertyIdentifier::PRESENT_VALUE,
                        &[0x44, 0x42, 0x48, 0x00, 0x00],
                        timestamped,
                    ),
                    value(
                        PropertyIdentifier::STATUS_FLAGS,
                        &[0x82, 0x04, 0x00],
                        timestamped,
                    ),
                ],
            },
            COVNotificationItem {
                monitored_object_identifier: oid(ObjectType::ANALOG_VALUE, 2),
                list_of_values: vec![value(
                    PropertyIdentifier::PRESENT_VALUE,
                    &[0x44, 0x41, 0x20, 0x00, 0x00],
                    timestamped,
                )],
            },
        ],
    }
}

fn encoded(notification: &COVNotificationMultipleRequest) -> Bytes {
    let mut octets = BytesMut::new();
    notification.encode(&mut octets).unwrap();
    octets.freeze()
}

fn confirmed(invoke_id: u8, service_choice: ConfirmedServiceChoice, body: Bytes) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_request: body,
    })
}

fn unconfirmed(service_choice: UnconfirmedServiceChoice, body: Bytes) -> Apdu {
    Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice,
        service_request: body,
    })
}

/// A started client at MAC 0x61 and the peer at 0x62 that talks to it.
struct Pair {
    client: BACnetClient<LoopbackTransport>,
    peer: LoopbackTransport,
    peer_rx: mpsc::Receiver<ReceivedNpdu>,
    events: broadcast::Receiver<COVEvent>,
}

const CLIENT: [u8; 1] = [0x61];
const PEER: [u8; 1] = [0x62];

async fn pair(options: ClientOptions) -> Pair {
    let (client_transport, mut peer) = LoopbackTransport::pair(CLIENT.to_vec(), PEER.to_vec());
    let peer_rx = peer.start().await.unwrap();
    let client =
        BACnetClient::start_with_options(ClientConfig::default(), client_transport, options)
            .await
            .unwrap();
    let events = client.cov_notifications();
    Pair {
        client,
        peer,
        peer_rx,
        events,
    }
}

impl Pair {
    async fn send(&self, apdu: Apdu, routed: Option<(u16, &[u8])>) {
        send_cov_notification(&self.peer, &CLIENT, apdu, routed).await;
    }

    async fn next_event(&mut self) -> COVEvent {
        timeout(Duration::from_secs(2), self.events.recv())
            .await
            .expect("timed out waiting for a COV notification")
            .expect("COV channel closed")
    }

    async fn next_multiple(&mut self) -> ReceivedCOVNotificationMultiple {
        match self.next_event().await {
            COVEvent::Multiple(received) => received,
            other => panic!("expected a COV-multiple notification, got {other:?}"),
        }
    }

    async fn answer(&mut self) -> (bacnet_encoding::npdu::Npdu, Apdu) {
        receive_peer_npdu_apdu(&mut self.peer_rx).await
    }

    /// Nothing more is published, and nothing more is sent to the peer.
    async fn assert_quiet(&mut self) {
        assert!(
            timeout(Duration::from_millis(100), self.events.recv())
                .await
                .is_err(),
            "a further COV notification"
        );
        assert!(
            timeout(Duration::from_millis(100), self.peer_rx.recv())
                .await
                .is_err(),
            "a further answer"
        );
    }

    async fn stop(mut self) {
        self.client.stop().await.unwrap();
        self.peer.stop().await.unwrap();
    }
}

fn assert_simple_ack(apdu: Apdu, invoke_id: u8) {
    let Apdu::SimpleAck(ack) = apdu else {
        panic!("expected SimpleAck, got {apdu:?}");
    };
    assert_eq!(ack.invoke_id, invoke_id);
    assert_eq!(
        ack.service_choice,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
    );
}

#[tokio::test]
async fn a_confirmed_multiple_is_acknowledged_and_published_once() {
    let mut pair = pair(ClientOptions::default()).await;
    let sent = multiple(31, false);
    pair.send(
        confirmed(
            0x51,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            encoded(&sent),
        ),
        None,
    )
    .await;

    let received = pair.next_multiple().await;
    assert_eq!(received.notification, sent);
    assert_eq!(received.delivery, COVNotificationDelivery::Confirmed);
    assert_eq!(&received.source_mac[..], &PEER);
    assert_eq!(
        (received.source_network, received.source_address),
        (None, None)
    );
    // Two objects, two values on the first, none timestamped.
    let objects = &received.notification.list_of_cov_notifications;
    assert_eq!(
        objects
            .iter()
            .map(|o| o.list_of_values.len())
            .collect::<Vec<_>>(),
        [2, 1]
    );
    assert_eq!(received.notification.timestamp, None);

    let (npdu, apdu) = pair.answer().await;
    assert!(npdu.destination.is_none());
    assert_simple_ack(apdu, 0x51);
    pair.assert_quiet().await;
    pair.stop().await;
}

#[tokio::test]
async fn a_routed_confirmed_multiple_with_timestamps_is_answered_through_its_router() {
    let mut pair = pair(ClientOptions::default()).await;
    let station = [0x0a, 0x0b];
    let sent = multiple(32, true);
    pair.send(
        confirmed(
            0x52,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            encoded(&sent),
        ),
        Some((7, &station)),
    )
    .await;

    let event = pair.next_event().await;
    assert_eq!(event.source_mac(), &MacAddr::from_slice(&PEER));
    assert_eq!(event.source_network(), Some(7));
    assert_eq!(event.source_address(), Some(&MacAddr::from_slice(&station)));
    assert_eq!(event.subscriber_process_identifier(), 32);
    assert_eq!(event.time_remaining(), 120);
    let received = event.multiple().expect("a COV-multiple notification");
    assert_eq!(received.notification, sent);
    assert_eq!(
        received.notification.timestamp.map(|(_, time)| time),
        Some(CHANGED)
    );
    assert!(
        received
            .notification
            .list_of_cov_notifications
            .iter()
            .flat_map(|object| &object.list_of_values)
            .all(|value| value.time_of_change == Some(CHANGED))
    );

    // The answer goes back through the router to the station.
    let (npdu, apdu) = pair.answer().await;
    assert_eq!(
        npdu.destination,
        Some(NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&station),
        })
    );
    assert_simple_ack(apdu, 0x52);
    pair.stop().await;
}

#[tokio::test]
async fn an_unconfirmed_multiple_is_published_with_or_without_timestamps() {
    let mut pair = pair(ClientOptions::default()).await;
    for (process, timestamped) in [(41, false), (42, true)] {
        let sent = multiple(process, timestamped);
        pair.send(
            unconfirmed(
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
                encoded(&sent),
            ),
            None,
        )
        .await;
        let received = pair.next_multiple().await;
        assert_eq!(received.notification, sent);
        assert_eq!(received.delivery, COVNotificationDelivery::Unconfirmed);
    }
    // Nothing answers an unconfirmed request.
    pair.assert_quiet().await;
    pair.stop().await;
}

#[tokio::test]
async fn a_retransmitted_confirmed_notification_is_acknowledged_and_published_again() {
    // The client keeps no record of the requests it answered, so a copy the
    // sender sends again, its answer lost, is executed again: both forms
    // alike.
    let mut pair = pair(ClientOptions::default()).await;
    let mut single = BytesMut::new();
    cov_notification(33).encode(&mut single).unwrap();
    for (service, body) in [
        (
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            single.freeze(),
        ),
        (
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            encoded(&multiple(33, false)),
        ),
    ] {
        for _ in 0..2 {
            pair.send(confirmed(0x53, service, body.clone()), None)
                .await;
            let event = pair.next_event().await;
            assert_eq!(
                event.multiple().is_some(),
                service == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
            );
            let Apdu::SimpleAck(ack) = pair.answer().await.1 else {
                panic!("expected SimpleAck");
            };
            assert_eq!((ack.invoke_id, ack.service_choice), (0x53, service));
        }
    }
    pair.assert_quiet().await;
    pair.stop().await;
}

/// A COVNotificationMultiple whose timestamp is present though no value has
/// a time of change.
fn timestamp_without_times() -> Bytes {
    let mut octets = encoded(&multiple(34, false)).to_vec();
    // Insert [3] { Date, Time } after the time remaining (tag 2).
    let at = octets.iter().position(|octet| *octet == 0x4e).unwrap();
    octets.splice(
        at..at,
        [0x3e, 0xa4, 126, 11, 16, 1, 0xb4, 9, 30, 5, 25, 0x3f],
    );
    Bytes::from(octets)
}

/// A COVNotificationMultiple whose values carry a time of change though the
/// notification has no timestamp.
fn times_without_timestamp() -> Bytes {
    let mut octets = encoded(&multiple(35, true)).to_vec();
    // [3] { Date, Time }: an opening tag, two 5-octet values, a closing tag.
    let at = octets.iter().position(|octet| *octet == 0x3e).unwrap();
    octets.drain(at..at + 12);
    Bytes::from(octets)
}

#[tokio::test]
async fn a_timestamp_and_time_of_change_mismatch_is_acknowledged_and_published() {
    let mut pair = pair(ClientOptions::default()).await;
    for (invoke_id, (body, has_timestamp, has_times)) in (0x70..).zip([
        (timestamp_without_times(), true, false),
        (times_without_timestamp(), false, true),
    ]) {
        pair.send(
            confirmed(
                invoke_id,
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
                body.clone(),
            ),
            None,
        )
        .await;
        let received = pair.next_multiple().await;
        assert_eq!(received.delivery, COVNotificationDelivery::Confirmed);
        assert_eq!(received.notification.timestamp.is_some(), has_timestamp);
        assert!(
            received
                .notification
                .list_of_cov_notifications
                .iter()
                .flat_map(|o| &o.list_of_values)
                .all(|v| v.time_of_change.is_some() == has_times)
        );
        assert_simple_ack(pair.answer().await.1, invoke_id);

        pair.send(
            unconfirmed(
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
                body,
            ),
            None,
        )
        .await;
        let received = pair.next_multiple().await;
        assert_eq!(received.delivery, COVNotificationDelivery::Unconfirmed);
    }
    pair.assert_quiet().await;
    pair.stop().await;
}

#[tokio::test]
async fn a_malformed_multiple_is_rejected_or_dropped_as_the_single_form_is() {
    let mut pair = pair(ClientOptions::default()).await;
    let cases = [
        // Nothing at all, and a request cut short after its first member.
        (Bytes::new(), RejectReason::MISSING_REQUIRED_PARAMETER),
        (
            Bytes::from_static(&[0x09, 0x22]),
            RejectReason::MISSING_REQUIRED_PARAMETER,
        ),
        // A device identifier three octets long.
        (
            Bytes::from_static(&[0x09, 0x22, 0x1B, 0x02, 0x00, 0x64]),
            RejectReason::INVALID_DATA_ENCODING,
        ),
    ];
    for (invoke_id, (body, reason)) in (0x60..).zip(cases.clone()) {
        pair.send(
            confirmed(
                invoke_id,
                ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
                body,
            ),
            None,
        )
        .await;
        let Apdu::Reject(reject) = pair.answer().await.1 else {
            panic!("expected Reject for invoke ID {invoke_id}");
        };
        assert_eq!(
            (reject.invoke_id, reject.reject_reason),
            (invoke_id, reason)
        );
    }
    // The unconfirmed form has no answer: each is dropped.
    for (body, _) in cases {
        pair.send(
            unconfirmed(
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
                body,
            ),
            None,
        )
        .await;
    }
    pair.assert_quiet().await;
    pair.stop().await;
}

#[tokio::test]
async fn the_ack_policy_answers_both_confirmed_forms() {
    let options = ClientOptions::default().with_confirmed_cov_notification_ack_policy(|event| {
        assert_eq!(event.delivery(), COVNotificationDelivery::Confirmed);
        match event {
            COVEvent::Object(_) => ConfirmedCOVNotificationResponse::Ack,
            COVEvent::Multiple(received)
                if received.notification.subscriber_process_identifier == 35 =>
            {
                ConfirmedCOVNotificationResponse::Reject(RejectReason::OTHER)
            }
            COVEvent::Multiple(_) => ConfirmedCOVNotificationResponse::NoResponse,
        }
    });
    let mut pair = pair(options).await;
    pair.send(
        confirmed(
            0x54,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            encoded(&multiple(35, false)),
        ),
        None,
    )
    .await;
    assert_eq!(pair.next_event().await.subscriber_process_identifier(), 35);
    let Apdu::Reject(reject) = pair.answer().await.1 else {
        panic!("expected Reject");
    };
    assert_eq!(
        (reject.invoke_id, reject.reject_reason),
        (0x54, RejectReason::OTHER)
    );

    // Published, and left unanswered.
    pair.send(
        confirmed(
            0x55,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            encoded(&multiple(36, false)),
        ),
        None,
    )
    .await;
    assert_eq!(pair.next_event().await.subscriber_process_identifier(), 36);
    pair.assert_quiet().await;
    pair.stop().await;
}

#[tokio::test]
async fn a_segmented_confirmed_multiple_is_aborted_unread() {
    // The client can't reassemble a segmented confirmed request yet (#2646):
    // its first segment is answered with an Abort and nothing is published.
    let mut pair = pair(ClientOptions::default()).await;
    let Apdu::ConfirmedRequest(mut first) = confirmed(
        0x56,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        encoded(&multiple(37, false)).slice(..8),
    ) else {
        unreachable!();
    };
    first.segmented = true;
    first.more_follows = true;
    first.sequence_number = Some(0);
    first.proposed_window_size = Some(1);
    pair.send(Apdu::ConfirmedRequest(first), None).await;

    let Apdu::Abort(abort) = pair.answer().await.1 else {
        panic!("expected Abort");
    };
    assert!(abort.sent_by_server);
    assert_eq!(
        (abort.invoke_id, abort.abort_reason),
        (0x56, AbortReason::SEGMENTATION_NOT_SUPPORTED)
    );
    pair.assert_quiet().await;
    pair.stop().await;
}

#[tokio::test]
async fn both_kinds_arrive_on_one_stream_in_order() {
    let mut pair = pair(ClientOptions::default()).await;
    let mut single = BytesMut::new();
    cov_notification(50).encode(&mut single).unwrap();
    let single = single.freeze();
    let sends = [
        unconfirmed(
            UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
            encoded(&multiple(51, false)),
        ),
        unconfirmed(
            UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
            single.clone(),
        ),
        confirmed(
            0x57,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            encoded(&multiple(52, true)),
        ),
        confirmed(
            0x58,
            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
            single,
        ),
    ];
    for apdu in sends {
        pair.send(apdu, None).await;
    }
    let mut heard = Vec::new();
    for _ in 0..4 {
        let event = pair.next_event().await;
        heard.push((
            event.multiple().is_some(),
            event.delivery(),
            event.subscriber_process_identifier(),
        ));
    }
    use COVNotificationDelivery::{Confirmed, Unconfirmed};
    assert_eq!(
        heard,
        [
            (true, Unconfirmed, 51),
            (false, Unconfirmed, 50),
            (true, Confirmed, 52),
            (false, Confirmed, 50),
        ]
    );
    pair.stop().await;
}

#[test]
fn a_station_matches_a_multiple_by_its_source_as_it_matches_one_object() {
    let source = Some(NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[0x0a]),
    });
    let event = COVEvent::Multiple(ReceivedCOVNotificationMultiple::new(
        multiple(1, false),
        &[0x09],
        &source,
        COVNotificationDelivery::Unconfirmed,
    ));
    assert!(Station::routed(&[0x09], 7, &[0x0a]).is_source_of(&event, None));
    assert!(Station::remote(7, &[0x0a]).is_source_of(&event, None));
    assert!(!Station::remote(7, &[0x0b]).is_source_of(&event, None));
    assert!(!Station::local(&[0x09]).is_source_of(&event, None));
    let local = COVEvent::Multiple(ReceivedCOVNotificationMultiple::new(
        multiple(1, false),
        &[0x0a],
        &None,
        COVNotificationDelivery::Unconfirmed,
    ));
    assert!(Station::local(&[0x0a]).is_source_of(&local, None));
    assert!(Station::remote(7, &[0x0a]).is_source_of(&local, Some(7)));
    assert!(!Station::remote(7, &[0x0a]).is_source_of(&local, None));
}

#[test]
fn the_event_reports_the_initiating_device_of_either_kind() {
    for instance in [7, 4_000] {
        let device = oid(ObjectType::DEVICE, instance);
        let object = COVEvent::Object(ReceivedCOVNotification::new(
            COVNotificationRequest {
                initiating_device_identifier: device,
                ..cov_notification(1)
            },
            &[],
            &None,
            COVNotificationDelivery::Unconfirmed,
        ));
        let several = COVEvent::Multiple(ReceivedCOVNotificationMultiple::new(
            COVNotificationMultipleRequest {
                initiating_device_identifier: device,
                ..multiple(1, false)
            },
            &[],
            &None,
            COVNotificationDelivery::Unconfirmed,
        ));
        assert_eq!(object.initiating_device_identifier(), device);
        assert_eq!(several.initiating_device_identifier(), device);
        // The by-value accessors split the kinds the same way.
        assert!(object.clone().into_multiple().is_none());
        assert!(object.into_object().is_some());
        assert!(several.clone().into_object().is_none());
        assert_eq!(
            several
                .into_multiple()
                .unwrap()
                .notification
                .initiating_device_identifier,
            device
        );
    }
}
