use super::super::*;
use bacnet_encoding::constructed::decode_event_notification;

use crate::server::test_transport::{SendLog, TestTransport, BIP_LOCAL_MAC};
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::{EventStateChange, EventTransition, EventTransitionCommit};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::{AcknowledgeAlarmRequest, EventNotificationRequest};
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{EngineeringUnits, EventState, EventType};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};
use bytes::Bytes;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

const REQUESTER: &[u8] = &[10, 0, 0, 1, 0xba, 0xc0];
const UNCONFIRMED_RECIPIENT: &[u8] = &[10, 0, 0, 2, 0xba, 0xc0];
const CONFIRMED_RECIPIENT: &[u8] = &[10, 0, 0, 3, 0xba, 0xc0];
type FailedPeers = StdArc<StdMutex<Vec<Vec<u8>>>>;

/// Records every send; a unicast to a MAC listed in `failures` fails after it
/// is recorded.
fn recording_transport(failures: &FailedPeers) -> TestTransport {
    let failures = StdArc::clone(failures);
    TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .on_send(move |frame| {
            let failed = !frame.broadcast
                && failures
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|item| item.as_slice() == frame.mac.as_slice());
            async move {
                if failed {
                    Err(Error::Transport(std::io::Error::other(
                        "injected send failure",
                    )))
                } else {
                    Ok(())
                }
            }
        })
        .build()
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
        valid_days: DaysOfWeek::all(),
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
        transitions: EventTransitionBits::all(),
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
    network: Arc<NetworkLayer<TestTransport>>,
    tracker: Arc<ConfirmedRequestTracker>,
    transactions: Arc<NotificationTransactions>,
    bindings: Arc<RwLock<DeviceBindingTable>>,
    comm_state: Arc<CommState>,
    config: ServerConfig,
    sent: SendLog,
    failures: FailedPeers,
    oid: ObjectIdentifier,
    acknowledged_state: EventState,
}

impl Harness {
    fn new(
        destinations: Vec<BACnetDestination>,
        event_enable: EventTransitionBits,
        retry_ms: u64,
    ) -> Self {
        let failures = FailedPeers::default();
        let transport = recording_transport(&failures);
        let sent = transport.sent();
        let mut db = ObjectDatabase::new();
        db.set_clock_reader(Some(StdArc::new(FixedClock)));

        let mut class = NotificationClass::new(7, "NC-ack").unwrap();
        class.priority = [11, 22, 33];
        for destination in destinations {
            class.add_destination(destination).unwrap();
        }
        db.add(Box::new(class)).unwrap();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(44, 0)
                    .expect("valid Device identity")
                    .with_name("Device"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();

        let mut object =
            AnalogInputObject::new(1, "AI-ack", EngineeringUnits::DEGREES_CELSIUS).unwrap();
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
                    data: vec![event_enable.to_bacnet()],
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
            comm_state: Arc::new(CommState::default()),
            config: ServerConfig {
                cov_retry_timeout_ms: retry_ms,
                ..ServerConfig::default()
            },
            sent,
            failures,
            oid,
            acknowledged_state: EventState::HIGH_LIMIT,
        }
    }

    fn request(&self, invoke_id: u8) -> ConfirmedRequestPdu {
        let request = AcknowledgeAlarmRequest {
            acknowledging_process_identifier: 71,
            event_object_identifier: self.oid,
            event_state_acknowledged: self.acknowledged_state,
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
        BACnetServer::<TestTransport>::handle_confirmed_request(
            &RequestServices {
                db: Arc::clone(&self.db),
                notification_transactions: Arc::clone(&self.transactions),
                device_bindings: Arc::clone(&self.bindings),
                comm_state: Arc::clone(&self.comm_state),
                ..RequestServices::for_test(Arc::clone(&self.network), self.config.clone())
            },
            &self.tracker,
            &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
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
        self.sent
            .frames()
            .into_iter()
            .map(|frame| (frame.mac.to_vec(), frame.npdu))
            .collect()
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
        EventTransitionBits::from_bacnet(&data).contains(EventTransitionBits::TO_OFFNORMAL)
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
            decode_event_notification(&request.service_request).unwrap(),
        ),
        Apdu::UnconfirmedRequest(request) => (
            false,
            None,
            decode_event_notification(&request.service_request).unwrap(),
        ),
        other => panic!("expected EventNotification request, got {other:?}"),
    }
}

#[tokio::test]
async fn simple_ack_precedes_fresh_exact_unconfirmed_ack_notification() {
    let harness = Harness::new(
        vec![local_recipient(UNCONFIRMED_RECIPIENT, 101, false)],
        EventTransitionBits::all(),
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
    assert_eq!(notification.notify_type, NotifyType::ACK_NOTIFICATION);
    assert_eq!(notification.event_type, EventType::OUT_OF_RANGE);
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
    assert_eq!(notification.from_state, EventState::NORMAL);
    assert_eq!(notification.to_state, EventState::HIGH_LIMIT);
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
        EventTransitionBits::all(),
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
            EventTransitionBits::TO_FAULT | EventTransitionBits::TO_NORMAL,
            1_000,
        ),
        Harness::new(Vec::new(), EventTransitionBits::all(), 1_000),
        Harness::new(
            vec![destination(
                BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 999).unwrap()),
                2,
                false,
            )],
            EventTransitionBits::all(),
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
        // The unresolved Device is looked for with a Who-Is (#1368), which
        // is no notification.
        let (who_is, notifications): (Vec<_>, Vec<_>) = harness
            .sent
            .frames()
            .into_iter()
            .partition(super::event_recipient_routing_tests::is_who_is);
        assert!(notifications.is_empty(), "case {index} must not fall back");
        assert_eq!(who_is.len(), usize::from(index == 2));
    }

    let disable_initiation = Harness::new(
        vec![local_recipient(UNCONFIRMED_RECIPIENT, 3, false)],
        EventTransitionBits::all(),
        1_000,
    );
    disable_initiation
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    assert_ack(
        disable_initiation.dispatch_with_reply(0x43).await.unwrap(),
        0x43,
    );
    assert!(disable_initiation.acknowledged().await);
    assert!(disable_initiation.frames().is_empty());
}

#[tokio::test]
async fn recipient_send_and_reservation_failures_do_not_retract_ack_or_block_other_recipients() {
    let send_failure = Harness::new(
        vec![
            local_recipient(UNCONFIRMED_RECIPIENT, 1, false),
            local_recipient(CONFIRMED_RECIPIENT, 2, false),
        ],
        EventTransitionBits::all(),
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
        EventTransitionBits::all(),
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

// Keep the worker's 20 ms retry deadline ahead of the 30 ms observation without
// depending on platform timer granularity or a fixed number of scheduler yields.
#[tokio::test(start_paused = true)]
async fn pending_duplicate_is_silent_post_issuance_reuse_notifies_and_retry_is_immutable() {
    let duplicate = Harness::new(
        vec![local_recipient(UNCONFIRMED_RECIPIENT, 17, false)],
        EventTransitionBits::all(),
        1_000,
    );
    // Poll the real handler into its database wait. Before response issuance,
    // an exact duplicate neither acknowledges nor starts another notification.
    let gate = duplicate.db.write().await;
    let first = duplicate.dispatch_with_reply(0x60);
    tokio::pin!(first);
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(first.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert!(duplicate.dispatch_with_reply(0x60).await.is_err());
    assert!(duplicate.frames().is_empty());
    drop(gate);
    assert_ack(first.await.unwrap(), 0x60);
    assert_eq!(duplicate.frames().len(), 1);
    assert!(duplicate.acknowledged().await);
    // After reply handoff, reusing the Invoke ID matches the existing new-ID
    // behavior: idempotent acknowledgment state, but another ACK_NOTIFICATION.
    assert_ack(duplicate.dispatch_with_reply(0x60).await.unwrap(), 0x60);
    assert_eq!(duplicate.frames().len(), 2);
    assert!(duplicate.acknowledged().await);
    assert_ack(duplicate.dispatch_with_reply(0x61).await.unwrap(), 0x61);
    assert_eq!(duplicate.frames().len(), 3);
    assert!(duplicate.acknowledged().await);

    let retry = Harness::new(
        vec![local_recipient(CONFIRMED_RECIPIENT, 18, true)],
        EventTransitionBits::all(),
        20,
    );
    assert_ack(retry.dispatch_with_reply(0x62).await.unwrap(), 0x62);
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
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        }),
    ));
}
