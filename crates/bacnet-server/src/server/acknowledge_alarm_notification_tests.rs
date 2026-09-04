use super::super::*;

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::{AcknowledgeAlarmRequest, EventNotificationRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};
use bytes::Bytes;
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::mpsc;

const REQUESTER: &[u8] = &[10, 0, 0, 1, 0xba, 0xc0];
const UNCONFIRMED_RECIPIENT: &[u8] = &[10, 0, 0, 2, 0xba, 0xc0];
const CONFIRMED_RECIPIENT: &[u8] = &[10, 0, 0, 3, 0xba, 0xc0];
type RecordedFrames = StdArc<StdMutex<Vec<(Vec<u8>, Bytes)>>>;
type FailedPeers = StdArc<StdMutex<Vec<Vec<u8>>>>;

#[derive(Clone, Default)]
struct RecordingTransport {
    sends: RecordedFrames,
    failures: FailedPeers,
}

impl TransportPort for RecordingTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sends
            .lock()
            .unwrap()
            .push((mac.to_vec(), Bytes::copy_from_slice(npdu)));
        if self.failures.lock().unwrap().iter().any(|item| item == mac) {
            Err(Error::Transport(std::io::Error::other(
                "injected send failure",
            )))
        } else {
            Ok(())
        }
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.sends
            .lock()
            .unwrap()
            .push((Vec::new(), Bytes::copy_from_slice(npdu)));
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0xba, 0xc0]
    }
}

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 9,
                day: 3,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn destination(
    recipient: BACnetRecipient,
    process_identifier: u32,
    confirmed: bool,
) -> BACnetDestination {
    BACnetDestination {
        valid_days: 0x7f,
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient,
        process_identifier,
        issue_confirmed_notifications: confirmed,
        transitions: 0x07,
    }
}

fn local_recipient(mac: &[u8], process_identifier: u32, confirmed: bool) -> BACnetDestination {
    destination(
        BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(mac),
        }),
        process_identifier,
        confirmed,
    )
}

struct Harness {
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<RecordingTransport>>,
    tracker: Arc<ConfirmedRequestTracker>,
    transactions: Arc<NotificationTransactions>,
    bindings: Arc<RwLock<DeviceBindingTable>>,
    comm_state: Arc<AtomicU8>,
    config: ServerConfig,
    sends: RecordedFrames,
    failures: FailedPeers,
    oid: ObjectIdentifier,
    acknowledged_state: EventState,
}

impl Harness {
    fn new(destinations: Vec<BACnetDestination>, event_enable: u8, retry_ms: u64) -> Self {
        let transport = RecordingTransport::default();
        let sends = StdArc::clone(&transport.sends);
        let failures = StdArc::clone(&transport.failures);
        let mut db = ObjectDatabase::new();
        db.set_clock_reader(Some(StdArc::new(FixedClock)));

        let mut class = NotificationClass::new(7, "NC-ack").unwrap();
        class.priority = [11, 22, 33];
        for destination in destinations {
            class.add_destination(destination);
        }
        db.add(Box::new(class)).unwrap();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 44,
                name: "Device".into(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();

        let mut object = AnalogInputObject::new(1, "AI-ack", 62).unwrap();
        object
            .write_property(
                PropertyIdentifier::NOTIFICATION_CLASS,
                None,
                PropertyValue::Unsigned(7),
                None,
            )
            .unwrap();
        object
            .write_property(
                PropertyIdentifier::NOTIFY_TYPE,
                None,
                PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
                None,
            )
            .unwrap();
        object
            .write_property(
                PropertyIdentifier::EVENT_ENABLE,
                None,
                PropertyValue::BitString {
                    unused_bits: 5,
                    data: vec![bacnet_types::bitstring::pack_octet(event_enable)],
                },
                None,
            )
            .unwrap();
        object
            .commit_event_transition_internal(EventTransitionCommit {
                change: EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::HIGH_LIMIT,
                },
                coordinate: EventTransition::ToOffnormal,
                ack_required: true,
                timestamp: BACnetTimeStamp::SequenceNumber(42),
                message_text: Some("original event".into()),
            })
            .unwrap();
        let oid = object.object_identifier();
        db.add(Box::new(object)).unwrap();

        Self {
            db: Arc::new(RwLock::new(db)),
            network: Arc::new(NetworkLayer::new(transport)),
            tracker: Arc::new(ConfirmedRequestTracker::default()),
            transactions: NotificationTransactions::new(),
            bindings: Arc::new(RwLock::new(DeviceBindingTable::new())),
            comm_state: Arc::new(AtomicU8::new(0)),
            config: ServerConfig {
                cov_retry_timeout_ms: retry_ms,
                ..ServerConfig::default()
            },
            sends,
            failures,
            oid,
            acknowledged_state: EventState::HIGH_LIMIT,
        }
    }

    fn request(&self, invoke_id: u8) -> ConfirmedRequestPdu {
        let request = AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 71,
            event_object_identifier: self.oid,
            event_state_acknowledged: self.acknowledged_state.to_raw(),
            timestamp: BACnetTimeStamp::SequenceNumber(42),
            acknowledgment_source: "operator".into(),
            time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(77),
        };
        let mut service_request = BytesMut::new();
        request.encode(&mut service_request).unwrap();
        ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::ACKNOWLEDGE_ALARM,
            service_request: service_request.freeze(),
        }
    }

    async fn dispatch(&self, invoke_id: u8, reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>) {
        BACnetServer::<RecordingTransport>::handle_confirmed_request(
            &self.db,
            &self.network,
            &Arc::new(RwLock::new(CovSubscriptionTable::new())),
            &Arc::new(Mutex::new(HashMap::new())),
            &Arc::new(Semaphore::new(MAX_SEG_SENDERS)),
            &Arc::new(Semaphore::new(1)),
            &Arc::new(Mutex::new(ServerTsm::new())),
            &self.transactions,
            &self.tracker,
            &self.bindings,
            &self.comm_state,
            &Arc::new(Mutex::new(None::<JoinHandle<()>>)),
            &self.config,
            REQUESTER,
            None,
            self.request(invoke_id),
            reply_tx,
        )
        .await;
    }

    async fn dispatch_with_reply(
        &self,
        invoke_id: u8,
    ) -> Result<Apdu, tokio::sync::oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        self.dispatch(invoke_id, Some(tx)).await;
        rx.await.map(|bytes| {
            let npdu = decode_npdu(bytes).unwrap();
            decode_apdu(npdu.payload).unwrap()
        })
    }

    fn frames(&self) -> Vec<(Vec<u8>, Bytes)> {
        self.sends.lock().unwrap().clone()
    }

    async fn acknowledged(&self) -> bool {
        let db = self.db.read().await;
        let PropertyValue::BitString { data, .. } = db
            .get(&self.oid)
            .unwrap()
            .read_property(PropertyIdentifier::ACKED_TRANSITIONS, None)
            .unwrap()
        else {
            panic!("Acked_Transitions must be a bit string");
        };
        bacnet_types::bitstring::unpack_octet(&data, 3) & 0x01 != 0
    }
}

fn assert_ack(apdu: Apdu, invoke_id: u8) {
    let Apdu::SimpleAck(ack) = apdu else {
        panic!("expected requester SimpleACK");
    };
    assert_eq!(ack.invoke_id, invoke_id);
    assert_eq!(
        ack.service_choice,
        ConfirmedServiceChoice::ACKNOWLEDGE_ALARM
    );
}

fn decode_notification(frame: &Bytes) -> (bool, Option<u8>, EventNotificationRequest) {
    let npdu = decode_npdu(frame.clone()).unwrap();
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::ConfirmedRequest(request) => (
            true,
            Some(request.invoke_id),
            EventNotificationRequest::decode(&request.service_request).unwrap(),
        ),
        Apdu::UnconfirmedRequest(request) => (
            false,
            None,
            EventNotificationRequest::decode(&request.service_request).unwrap(),
        ),
        other => panic!("expected EventNotification request, got {other:?}"),
    }
}

#[tokio::test]
async fn simple_ack_precedes_fresh_exact_unconfirmed_ack_notification() {
    let harness = Harness::new(
        vec![local_recipient(UNCONFIRMED_RECIPIENT, 101, false)],
        0x07,
        1_000,
    );

    harness.dispatch(0x31, None).await;

    let frames = harness.frames();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].0, REQUESTER);
    let requester_npdu = decode_npdu(frames[0].1.clone()).unwrap();
    assert_ack(decode_apdu(requester_npdu.payload).unwrap(), 0x31);
    assert_eq!(frames[1].0, UNCONFIRMED_RECIPIENT);
    let (confirmed, invoke_id, notification) = decode_notification(&frames[1].1);
    assert!(!confirmed);
    assert_eq!(invoke_id, None);
    assert_eq!(notification.process_identifier, 101);
    assert_eq!(
        notification.notify_type,
        NotifyType::ACK_NOTIFICATION.to_raw()
    );
    assert_eq!(notification.event_type, EventType::OUT_OF_RANGE.to_raw());
    assert_eq!(notification.priority, 11);
    assert_eq!(notification.message_text, None);
    assert_eq!(
        notification.timestamp,
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 126,
                month: 9,
                day: 3,
                day_of_week: 4
            },
            time: Time {
                hour: 12,
                minute: 34,
                second: 56,
                hundredths: 78
            },
        }
    );
    assert_ne!(notification.timestamp, BACnetTimeStamp::SequenceNumber(42));
    assert!(!notification.ack_required);
    assert_eq!(notification.from_state, 0);
    assert_eq!(notification.to_state, EventState::HIGH_LIMIT.to_raw());
    assert!(notification.event_values.is_none());
    assert!(harness.acknowledged().await);
}

#[tokio::test]
async fn recipient_policy_selects_confirmed_and_unconfirmed_services_with_process_ids() {
    let harness = Harness::new(
        vec![
            local_recipient(UNCONFIRMED_RECIPIENT, 101, false),
            local_recipient(CONFIRMED_RECIPIENT, 202, true),
        ],
        0x07,
        60_000,
    );

    assert_ack(harness.dispatch_with_reply(0x32).await.unwrap(), 0x32);
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }

    let frames = harness.frames();
    assert_eq!(frames.len(), 2);
    let mut seen = frames
        .iter()
        .map(|(mac, frame)| {
            let (confirmed, invoke_id, notification) = decode_notification(frame);
            (
                mac.clone(),
                confirmed,
                invoke_id,
                notification.process_identifier,
            )
        })
        .collect::<Vec<_>>();
    seen.sort_by_key(|item| item.3);
    assert_eq!(seen[0], (UNCONFIRMED_RECIPIENT.to_vec(), false, None, 101));
    assert_eq!(seen[1].0, CONFIRMED_RECIPIENT);
    assert!(seen[1].1);
    assert_eq!(seen[1].3, 202);

    let confirmed_invoke = seen[1].2.unwrap();
    assert!(harness.transactions.admit_terminal(
        CONFIRMED_RECIPIENT,
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: confirmed_invoke,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        }),
    ));
}

#[tokio::test]
async fn event_enable_dcc_empty_and_unresolved_recipients_preserve_acceptance_without_fallback() {
    let cases = [
        Harness::new(
            vec![local_recipient(UNCONFIRMED_RECIPIENT, 1, false)],
            0x06,
            1_000,
        ),
        Harness::new(Vec::new(), 0x07, 1_000),
        Harness::new(
            vec![destination(
                BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 999).unwrap()),
                2,
                false,
            )],
            0x07,
            1_000,
        ),
    ];

    for (index, harness) in cases.into_iter().enumerate() {
        assert_ack(
            harness
                .dispatch_with_reply(0x40 + index as u8)
                .await
                .unwrap(),
            0x40 + index as u8,
        );
        assert!(harness.acknowledged().await);
        assert!(
            harness.frames().is_empty(),
            "case {index} must not fall back"
        );
    }

    let disable_initiation = Harness::new(
        vec![local_recipient(UNCONFIRMED_RECIPIENT, 3, false)],
        0x07,
        1_000,
    );
    disable_initiation.comm_state.store(2, Ordering::Release);
    assert_ack(
        disable_initiation.dispatch_with_reply(0x43).await.unwrap(),
        0x43,
    );
    assert!(disable_initiation.acknowledged().await);
    assert!(disable_initiation.frames().is_empty());
}

#[tokio::test]
async fn full_dcc_disable_drops_acknowledgment_before_response_or_mutation() {
    let full_disable = Harness::new(
        vec![local_recipient(UNCONFIRMED_RECIPIENT, 3, false)],
        0x07,
        1_000,
    );
    full_disable.comm_state.store(1, Ordering::Release);

    assert!(full_disable.dispatch_with_reply(0x44).await.is_err());
    assert!(!full_disable.acknowledged().await);
    assert!(full_disable.frames().is_empty());
}

#[tokio::test]
async fn recipient_send_and_reservation_failures_do_not_retract_ack_or_block_other_recipients() {
    let send_failure = Harness::new(
        vec![
            local_recipient(UNCONFIRMED_RECIPIENT, 1, false),
            local_recipient(CONFIRMED_RECIPIENT, 2, false),
        ],
        0x07,
        1_000,
    );
    send_failure
        .failures
        .lock()
        .unwrap()
        .push(UNCONFIRMED_RECIPIENT.to_vec());
    assert_ack(send_failure.dispatch_with_reply(0x50).await.unwrap(), 0x50);
    assert!(send_failure.acknowledged().await);
    let attempts = send_failure.frames();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].0, UNCONFIRMED_RECIPIENT);
    assert_eq!(attempts[1].0, CONFIRMED_RECIPIENT);
    assert_eq!(decode_notification(&attempts[1].1).2.process_identifier, 2);

    let reservation_failure = Harness::new(
        vec![local_recipient(CONFIRMED_RECIPIENT, 9, true)],
        0x07,
        1_000,
    );
    let mut leases = Vec::new();
    for _ in 0..=u8::MAX {
        let (operation, receiver) = reservation_failure
            .transactions
            .reserve(
                canonical_direct_peer(CONFIRMED_RECIPIENT),
                ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            )
            .unwrap();
        leases.push((operation, receiver));
    }
    assert_ack(
        reservation_failure.dispatch_with_reply(0x51).await.unwrap(),
        0x51,
    );
    assert!(reservation_failure.acknowledged().await);
    assert!(reservation_failure.frames().is_empty());
    drop(leases);
}

#[tokio::test]
async fn duplicate_is_silent_new_invoke_notifies_again_and_confirmed_retry_is_immutable() {
    let duplicate = Harness::new(
        vec![local_recipient(UNCONFIRMED_RECIPIENT, 17, false)],
        0x07,
        1_000,
    );
    assert_ack(duplicate.dispatch_with_reply(0x60).await.unwrap(), 0x60);
    assert_eq!(duplicate.frames().len(), 1);
    assert!(duplicate.dispatch_with_reply(0x60).await.is_err());
    assert_eq!(duplicate.frames().len(), 1);
    assert_ack(duplicate.dispatch_with_reply(0x61).await.unwrap(), 0x61);
    assert_eq!(duplicate.frames().len(), 2);

    let retry = Harness::new(
        vec![local_recipient(CONFIRMED_RECIPIENT, 18, true)],
        0x07,
        20,
    );
    assert_ack(retry.dispatch_with_reply(0x62).await.unwrap(), 0x62);
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    let frames = retry.frames();
    assert!(
        frames.len() >= 2,
        "confirmed notification must retry after silence"
    );
    assert_eq!(
        frames[0], frames[1],
        "retry must reuse immutable encoded bytes"
    );
    let (_, invoke_id, _) = decode_notification(&frames[0].1);
    let invoke_id = invoke_id.unwrap();
    assert!(retry.transactions.admit_terminal(
        CONFIRMED_RECIPIENT,
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        }),
    ));
}
