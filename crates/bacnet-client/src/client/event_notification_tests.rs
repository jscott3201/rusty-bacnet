use super::*;
use bacnet_encoding::apdu::UnconfirmedRequest;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_encoding::tags::{encode_tag, TagClass};
use bacnet_services::alarm_event::NotificationParameters;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::{EventState, EventType, NotifyType, ObjectType, RejectReason};
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, StatusFlags};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

// Independent wire fixture: process 42, device 100, analog input 7, sequence 5,
// class 2, priority 16, out-of-range alarm, from normal to high limit.
const PREFIX: &[u8] = &[
    0x09, 42, 0x1c, 2, 0, 0, 100, 0x2c, 0, 0, 0, 7, 0x3e, 0x19, 5, 0x3f, 0x49, 2, 0x59, 16, 0x69, 5,
];
const SUFFIX: &[u8] = &[
    0x89, 0, 0x99, 1, 0xa9, 0, 0xb9, 3, 0xce, 0x5e, 0x0c, 0x42, 0x48, 0, 0, 0x1a, 4, 0, 0x2c, 0x3f,
    0x80, 0, 0, 0x3c, 0x42, 0x40, 0, 0, 0x5f, 0xcf,
];
const PEER: &[u8] = &[0x12];
const INVOKE: u8 = 37;

#[path = "response_warning_tests.rs"]
mod response_warnings;

fn remote() -> Option<NpduAddress> {
    Some(NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0x23, 0x24]),
    })
}

fn payload(text: Option<&[u8]>) -> Bytes {
    let mut data = BytesMut::from(PREFIX);
    if let Some(text) = text {
        encode_tag(&mut data, 7, TagClass::Context, text.len() as u32);
        data.extend_from_slice(text);
    }
    data.extend_from_slice(SUFFIX);
    data.freeze()
}

fn request(data: Bytes, confirmed: bool, segmented: bool) -> Apdu {
    if confirmed {
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented,
            more_follows: segmented,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id: INVOKE,
            sequence_number: segmented.then_some(0),
            proposed_window_size: segmented.then_some(1),
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            service_request: data,
        })
    } else {
        Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
            service_request: data,
        })
    }
}

fn inbound(apdu: Apdu, source: Option<NpduAddress>) -> ReceivedNpdu {
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source,
            expecting_reply: matches!(apdu, Apdu::ConfirmedRequest(_)),
            payload: encoded.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(PEER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

struct ReceiveTransport {
    input: Option<mpsc::Receiver<ReceivedNpdu>>,
    output: mpsc::Sender<(Bytes, MacAddr)>,
}

impl TransportPort for ReceiveTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.input.take().unwrap())
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.output
            .send((Bytes::copy_from_slice(npdu), MacAddr::from_slice(mac)))
            .await
            .unwrap();
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        panic!("client must not broadcast an event response")
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &[0x11]
    }
}

struct Harness {
    client: BACnetClient<ReceiveTransport>,
    input: mpsc::Sender<ReceivedNpdu>,
    output: mpsc::Receiver<(Bytes, MacAddr)>,
}

impl Harness {
    async fn new(capacity: usize) -> Self {
        let (input, input_rx) = mpsc::channel(8);
        let (output_tx, output) = mpsc::channel(8);
        let client = BACnetClient::generic_builder()
            .transport(ReceiveTransport {
                input: Some(input_rx),
                output: output_tx,
            })
            .event_channel_capacity(capacity)
            .confirmed_cov_notification_ack_policy(|_| {
                panic!("event notifications must not invoke the COV policy")
            })
            .build()
            .await
            .unwrap();
        Self {
            client,
            input,
            output,
        }
    }

    async fn send(&self, received: ReceivedNpdu) {
        self.input.send(received).await.unwrap();
    }

    async fn reply(&mut self, source: Option<NpduAddress>, expected: &[u8]) {
        let (data, mac) = timeout(Duration::from_secs(2), self.output.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mac.as_ref(), PEER);
        assert_reply(data, source, expected);
    }

    /// Require a Reject, and return its reason.
    async fn reject(&mut self, source: Option<NpduAddress>) -> RejectReason {
        let (data, mac) = timeout(Duration::from_secs(2), self.output.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mac.as_ref(), PEER);
        let npdu = decode_npdu(data).unwrap();
        assert_eq!(npdu.destination, source);
        let [0x60, INVOKE, reason] = npdu.payload[..] else {
            panic!("expected a Reject, got {:02X?}", npdu.payload);
        };
        RejectReason::from_raw(reason)
    }

    // An ordered request/response fence proves prior dispatch completed without
    // depending on sleeps or treating a timeout as evidence of wire silence.
    async fn fence(&mut self) {
        let Apdu::ConfirmedRequest(mut barrier) = request(Bytes::new(), true, false) else {
            unreachable!()
        };
        barrier.invoke_id = 0xee;
        barrier.service_choice = ConfirmedServiceChoice::READ_PROPERTY;
        self.send(inbound(Apdu::ConfirmedRequest(barrier), None))
            .await;
        self.reply(None, &[0x60, 0xee, 9]).await;
        assert!(matches!(
            self.output.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}

fn assert_reply(data: Bytes, source: Option<NpduAddress>, expected: &[u8]) {
    let npdu = decode_npdu(data).unwrap();
    assert_eq!(npdu.destination, source);
    assert!(npdu.source.is_none());
    assert!(!npdu.expecting_reply);
    assert_eq!(npdu.priority, NetworkPriority::Normal);
    assert_eq!(npdu.payload.as_ref(), expected);
}

fn assert_notification(
    received: ReceivedEventNotification,
    source: Option<NpduAddress>,
    confirmed: bool,
    text: Option<&str>,
) {
    assert_eq!(received.source_mac.as_ref(), PEER);
    assert_eq!(received.source_network, source.as_ref().map(|s| s.network));
    assert_eq!(received.source_address, source.map(|s| s.mac_address));
    assert_eq!(
        received.delivery,
        if confirmed {
            EventNotificationDelivery::Confirmed
        } else {
            EventNotificationDelivery::Unconfirmed
        }
    );
    let notification = received.notification;
    assert_eq!(notification.process_identifier, 42);
    assert_eq!(
        notification.initiating_device_identifier,
        ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap()
    );
    assert_eq!(
        notification.event_object_identifier,
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap()
    );
    assert_eq!(notification.timestamp, BACnetTimeStamp::SequenceNumber(5));
    assert_eq!(notification.notification_class, 2);
    assert_eq!(notification.priority, 16);
    assert_eq!(notification.event_type, EventType::OUT_OF_RANGE);
    assert_eq!(notification.message_text.as_deref(), text);
    assert_eq!(notification.notify_type, NotifyType::ALARM);
    assert!(notification.ack_required);
    assert_eq!(notification.from_state, EventState::NORMAL);
    assert_eq!(notification.to_state, EventState::HIGH_LIMIT);
    assert_eq!(
        notification.event_values,
        Some(NotificationParameters::OutOfRange {
            exceeding_value: 50.0,
            status_flags: StatusFlags::empty(),
            deadband: 1.0,
            exceeded_limit: 48.0,
        })
    );
}

#[tokio::test]
async fn valid_events_preserve_payload_metadata_and_service_specific_replies() {
    let mut h = Harness::new(64).await;
    let mut rx = h.client.event_notifications();
    let mut cov = h.client.cov_notifications();
    let cases: &[(Option<&[u8]>, Option<&str>)] = &[
        (None, None),
        (Some(&[0]), Some("")),
        (Some(&[0, b'O', b'K']), Some("OK")),
        (Some(&[4, 0, b'O', 0, b'K']), Some("OK")),
        (Some(&[5, 0xe9]), Some("é")),
    ];
    for confirmed in [true, false] {
        for source in [None, remote()] {
            for &(text, expected) in cases {
                let data = payload(text);
                assert!(decode_event_notification(&data).is_ok());
                h.send(inbound(request(data, confirmed, false), source.clone()))
                    .await;
                if confirmed {
                    h.reply(source.clone(), &[0x20, INVOKE, 2]).await;
                }
                h.fence().await;
                assert_notification(rx.try_recv().unwrap(), source.clone(), confirmed, expected);
                assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
                assert_eq!(cov.try_recv().unwrap_err(), TryRecvError::Empty);
            }
        }
    }
    h.client.stop().await.unwrap();
}

fn invalid_text_cases() -> Vec<Vec<u8>> {
    let mut cases = vec![
        vec![0, 0xff],    // Invalid UTF-8.
        vec![4, 0],       // Odd UCS-2 length.
        vec![4, 0xd8, 0], // UCS-2 surrogate.
        vec![1, b'A'],
        vec![2, b'A'],
        vec![3, 0, 0, 0, b'A'], // Unsupported charsets.
        vec![0xfe, b'A'],       // Unknown charset.
    ];
    let mut extended = vec![0; 300];
    extended[299] = 0xff;
    cases.push(extended);
    cases
}

#[tokio::test]
async fn bad_message_text_is_dropped_only_by_client_for_both_services() {
    let mut h = Harness::new(64).await;
    let mut rx = h.client.event_notifications();
    for confirmed in [true, false] {
        for text in invalid_text_cases() {
            let data = payload(Some(&text));
            assert!(decode_event_notification(&data).is_err());
            h.send(inbound(request(data.clone(), confirmed, false), remote()))
                .await;
            if confirmed {
                h.reply(remote(), &[0x20, INVOKE, 2]).await;
            }
            h.fence().await;
            assert_notification(rx.try_recv().unwrap(), remote(), confirmed, None);
            assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
            assert!(decode_event_notification(&data).is_err());
        }
    }
    h.client.stop().await.unwrap();
}

/// Each malformed request with the Reject reason it draws (#1446).
fn malformed_cases() -> Vec<(Bytes, RejectReason)> {
    use RejectReason as R;
    let bad_text = payload(Some(&[0, 0xff]));
    // Nothing at all, and a text with no character set octet.
    let mut cases = vec![
        (Bytes::new(), R::MISSING_REQUIRED_PARAMETER),
        (payload(Some(&[])), R::INVALID_DATA_ENCODING),
    ];
    for (text_tlv, reason) in [
        // Constructed text is not a character string.
        (&[0x7e, 0x7f][..], R::INVALID_TAG),
        // Wrong tag class: an application CharacterString whose UTF-8 the
        // request's value check refuses first.
        (&[0x72, 0, 0xff], R::INVALID_DATA_ENCODING),
        // Impossible declared length.
        (&[0x7d, 0xff, 0xff, 0xff, 0xff, 0xff], R::BUFFER_OVERFLOW),
        // Truncated extended header: the next octet reads as its length.
        (&[0x7d], R::MISSING_REQUIRED_PARAMETER),
        // Duplicate text fields, and bad text followed by valid empty text:
        // the first text's characters are refused before the second is seen.
        (&[0x7a, 0, 0xff, 0x7a, 0, 0xff], R::INVALID_DATA_ENCODING),
        (&[0x7a, 0, 0xff, 0x79, 0], R::INVALID_DATA_ENCODING),
        // Valid text followed by bad text, where the notify type is due.
        (&[0x79, 0, 0x7a, 0, 0xff], R::INVALID_TAG),
    ] {
        cases.push(([PREFIX, text_tlv, SUFFIX].concat().into(), reason));
    }
    // [1] where the [0] process identifier is due.
    let mut wrong_prefix = bad_text.to_vec();
    wrong_prefix[0] = 0x19;
    cases.push((wrong_prefix.into(), R::MISSING_REQUIRED_PARAMETER));
    // The timestamp frame closed by [4].
    let mut wrong_timestamp = bad_text.to_vec();
    wrong_timestamp[15] = 0x4f;
    cases.push((wrong_timestamp.into(), R::INVALID_TAG));
    let mut oversized_priority = bad_text.to_vec();
    oversized_priority.splice(18..20, [0x5a, 1, 0]);
    cases.push((oversized_priority.into(), R::PARAMETER_OUT_OF_RANGE));
    // A second [10] where the [11] to-state is due.
    let mut wrong_tail = bad_text.to_vec();
    wrong_tail[PREFIX.len() + 3 + 6] = 0xa9;
    cases.push((wrong_tail.into(), R::INVALID_TAG));
    // The event values closed by [13].
    let mut wrong_values = bad_text.to_vec();
    *wrong_values.last_mut().unwrap() = 0xdf;
    cases.push((wrong_values.into(), R::INVALID_TAG));
    cases
}

#[tokio::test]
async fn other_malformed_fields_are_rejected_or_silently_dropped_without_delivery() {
    let mut h = Harness::new(64).await;
    let mut rx = h.client.event_notifications();
    for confirmed in [true, false] {
        for (data, reason) in malformed_cases() {
            assert!(decode_event_notification(&data).is_err());
            let what = format!("{data:02X?}");
            h.send(inbound(request(data, confirmed, false), remote()))
                .await;
            if confirmed {
                assert_eq!(h.reject(remote()).await, reason, "{what}");
            }
            h.fence().await;
            assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
        }
    }
    h.client.stop().await.unwrap();
}

/// #1950: a burst of malformed event notifications, through both services,
/// of malformed COV notifications, I-Ams and I-Haves, and of APDUs that
/// don't decode at all, goes through the client's one decode throttle: the
/// first warns, and the rest log at debug and count toward the next warning.
#[tokio::test(start_paused = true)]
async fn a_burst_of_malformed_requests_warns_once_per_interval() {
    use bacnet_transport::warn_throttle::WARNING_INTERVAL;
    let mut h = Harness::new(64).await;
    let _i_haves = h.client.i_have_events();
    let cases = malformed_cases();
    for (data, _) in &cases {
        h.send(inbound(request(data.clone(), true, false), None))
            .await;
        h.reject(None).await;
        h.send(inbound(request(data.clone(), false, false), None))
            .await;
    }
    let Apdu::ConfirmedRequest(mut cov) = request(Bytes::new(), true, false) else {
        unreachable!()
    };
    cov.service_choice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
    h.send(inbound(Apdu::ConfirmedRequest(cov), None)).await;
    h.reject(None).await;
    for service_choice in [
        UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
        UnconfirmedServiceChoice::I_AM,
        UnconfirmedServiceChoice::I_HAVE,
    ] {
        let malformed = Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice,
            service_request: Bytes::from_static(&[0xFF]),
        });
        h.send(inbound(malformed, None)).await;
    }
    // Version 1, no control flags, then PDU type 15, which doesn't exist.
    let mut undecodable = inbound(request(Bytes::new(), false, false), None);
    undecodable.npdu = Bytes::from_static(&[0x01, 0x00, 0xF0]);
    h.send(undecodable).await;
    h.fence().await;
    let throttle = &h.client.decode_warning;
    assert_eq!(
        throttle.note(),
        None,
        "the first warned; the rest held back"
    );
    tokio::time::advance(WARNING_INTERVAL).await;
    let burst = 2 * cases.len() as u64 + 5;
    assert_eq!(
        throttle.note(),
        Some(burst + 1),
        "all but the first of the burst, the check above, and this one"
    );
    h.client.stop().await.unwrap();
}

#[tokio::test]
async fn truncations_do_not_escape_validation_after_text_tolerance() {
    let mut h = Harness::new(64).await;
    let mut rx = h.client.event_notifications();
    let data = payload(Some(&[0, 0xff]));
    for len in 0..data.len() {
        // The shared decoder permits omitted event values; that complete prefix
        // is not a truncation error and remains outside this negative matrix.
        if len == PREFIX.len() + 3 + 8 {
            continue;
        }
        h.send(inbound(request(data.slice(..len), true, false), None))
            .await;
        // Every request cut short is missing a parameter (#1446).
        assert_eq!(
            h.reject(None).await,
            RejectReason::MISSING_REQUIRED_PARAMETER,
            "cut at {len}"
        );
        assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
    }
    h.client.stop().await.unwrap();
}

#[tokio::test]
async fn group_confirmed_events_stay_silent_even_when_segmented() {
    let mut h = Harness::new(64).await;
    let mut rx = h.client.event_notifications();
    for segmented in [false, true] {
        for group in [None, Some(200), Some(0xffff)] {
            let mut received = inbound(request(payload(Some(&[0, 0xff])), true, segmented), None);
            if let Some(network) = group {
                let mut npdu = decode_npdu(received.npdu).unwrap();
                npdu.destination = Some(NpduAddress {
                    network,
                    mac_address: MacAddr::new(),
                });
                let mut buf = BytesMut::new();
                encode_npdu(&mut buf, &npdu).unwrap();
                received.npdu = buf.freeze();
            } else {
                received.link_layer_group = true;
            }
            h.send(received).await;
            h.fence().await;
            assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
        }
    }
    h.client.stop().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_group_event_is_delivered_without_reply() {
    let mut h = Harness::new(64).await;
    let mut rx = h.client.event_notifications();
    let mut received = inbound(request(payload(None), false, false), None);
    received.link_layer_group = true;
    h.send(received).await;
    h.fence().await;
    assert_notification(rx.try_recv().unwrap(), None, false, None);
    h.client.stop().await.unwrap();
}

#[tokio::test]
async fn segmented_event_request_keeps_server_abort_and_never_publishes() {
    let mut h = Harness::new(64).await;
    let mut rx = h.client.event_notifications();
    for source in [None, remote()] {
        h.send(inbound(request(payload(None), true, true), source.clone()))
            .await;
        h.reply(source, &[0x71, INVOKE, 4]).await;
        assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
    }
    h.client.stop().await.unwrap();
}

#[tokio::test]
async fn immediate_reply_and_closed_channel_fallback_preserve_event_responses() {
    let mut h = Harness::new(64).await;
    let mut rx = h.client.event_notifications();
    for source in [None, remote()] {
        for closed in [false, true] {
            for (data, segmented, expected, publish) in [
                (payload(Some(&[0, 0xff])), false, [0x20, INVOKE, 2], true),
                // Message text with no character set octet: an encoding not
                // valid for its datatype (#1446).
                (payload(Some(&[])), false, [0x60, INVOKE, 10], false),
                (payload(None), true, [0x71, INVOKE, 4], false),
            ] {
                let (tx, reply) = oneshot::channel();
                let reply = if closed {
                    drop(reply);
                    None
                } else {
                    Some(reply)
                };
                let mut received = inbound(request(data, true, segmented), source.clone());
                received.reply_tx = Some(tx);
                h.send(received).await;
                if let Some(reply) = reply {
                    let bytes = timeout(Duration::from_secs(2), reply)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_reply(bytes, source.clone(), &expected);
                } else {
                    h.reply(source.clone(), &expected).await;
                }
                h.fence().await;
                if publish {
                    assert_notification(rx.try_recv().unwrap(), source.clone(), true, None);
                }
                assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
            }
        }
    }
    h.client.stop().await.unwrap();
}

#[tokio::test]
async fn valid_confirmed_event_is_acked_without_subscribers() {
    let mut h = Harness::new(1).await;
    h.send(inbound(
        request(payload(Some(&[0, 0xff])), true, false),
        None,
    ))
    .await;
    h.reply(None, &[0x20, INVOKE, 2]).await;
    let mut late = h.client.event_notifications();
    h.fence().await;
    assert_eq!(late.try_recv().unwrap_err(), TryRecvError::Empty);
    h.client.stop().await.unwrap();
}

#[tokio::test]
async fn event_capacity_lag_and_multiple_receivers_are_independent_of_cov() {
    let mut h = Harness::new(1).await;
    let mut slow = h.client.event_notifications();
    let mut fast = h.client.event_notifications();
    let mut cov = h.client.cov_notifications();
    for _ in 0..3 {
        h.send(inbound(request(payload(None), true, false), None))
            .await;
        h.reply(None, &[0x20, INVOKE, 2]).await;
        assert_notification(fast.try_recv().unwrap(), None, true, None);
    }
    assert_eq!(slow.recv().await.unwrap_err(), RecvError::Lagged(2));
    assert_notification(slow.try_recv().unwrap(), None, true, None);
    assert_eq!(cov.try_recv().unwrap_err(), TryRecvError::Empty);
    assert_eq!(h.client.cov_tx.len(), 0);
    h.client.stop().await.unwrap();
}

#[tokio::test]
async fn event_options_defaults_bounds_and_builders() {
    assert_eq!(ClientOptions::default().event_channel_capacity, 64);
    let options = ClientOptions::default().with_event_channel_capacity(MAX_EVENT_CHANNEL_CAPACITY);
    assert!(options.validate().is_ok());
    assert_eq!(options.cov_channel_capacity, DEFAULT_COV_CHANNEL_CAPACITY);
    for capacity in [0, MAX_EVENT_CHANNEL_CAPACITY + 1, usize::MAX] {
        let options = ClientOptions::default().with_event_channel_capacity(capacity);
        assert!(matches!(options.validate(), Err(Error::Encoding(_))));
        let (input, _) = mpsc::channel(1);
        let (_, receiver) = mpsc::channel(1);
        assert!(matches!(
            BACnetClient::start_with_options(
                ClientConfig::default(),
                ReceiveTransport {
                    input: Some(receiver),
                    output: input
                },
                options,
            )
            .await,
            Err(Error::Encoding(_))
        ));
    }
    assert_eq!(
        BACnetClient::bip_builder()
            .event_channel_capacity(7)
            .options
            .event_channel_capacity,
        7
    );
    #[cfg(feature = "ipv6")]
    assert_eq!(
        BACnetClient::bip6_builder()
            .event_channel_capacity(8)
            .options
            .event_channel_capacity,
        8
    );
    #[cfg(feature = "sc-tls")]
    assert_eq!(
        BACnetClient::sc_builder()
            .event_channel_capacity(9)
            .options
            .event_channel_capacity,
        9
    );
}

#[tokio::test]
async fn stopped_then_dropped_client_closes_event_receivers() {
    let mut h = Harness::new(1).await;
    let mut rx = h.client.event_notifications();
    h.send(inbound(request(payload(None), true, false), None))
        .await;
    h.reply(None, &[0x20, INVOKE, 2]).await;
    h.client.stop().await.unwrap();
    drop(h);
    assert_notification(rx.recv().await.unwrap(), None, true, None);
    assert_eq!(rx.recv().await.unwrap_err(), RecvError::Closed);
}
