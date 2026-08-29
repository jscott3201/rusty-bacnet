//! Confirmed event notifications to remote-network recipients (#375).
//!
//! Clause 6.3 permits a confirmed PDU on a broadcast link DA when the
//! DNET/DADR restricts the destination to one device, and Clause 6.5.3 names
//! the broadcast DA as the send form while "the address of the router is
//! initially unknown". What used to block this was correlation: the ack
//! arrives from whichever router delivers it, so the transaction is keyed by
//! routed identity with an empty local half, and the router's MAC is learned
//! from the ack per Clause 6.5.3 method 4 ("noting the SA associated with any
//! subsequent responses from the remote device").
//!
//! The tests drive the real distribution path over a recording transport and
//! feed acks through the same correlation entry point the dispatch loop uses.

use super::device_bindings::{DeviceBindingTable, OBSERVED_BINDING_TTL};
use super::event_notifications::CommittedIntrinsicTransition;
use super::event_notifications_tests::local_broadcast_destination;
use super::event_recipient_routing_tests::{address_recipient, destination_for};
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::event::EventStateChange;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{EventState, EventType};
use bytes::Bytes;
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::mpsc;

#[derive(Clone, Default)]
struct RecordingTransport {
    broadcasts: StdArc<StdMutex<Vec<Bytes>>>,
    unicasts: StdArc<StdMutex<Vec<(Vec<u8>, Bytes)>>>,
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
        self.unicasts
            .lock()
            .unwrap()
            .push((mac.to_vec(), Bytes::copy_from_slice(npdu)));
        Ok(())
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.broadcasts
            .lock()
            .unwrap()
            .push(Bytes::copy_from_slice(npdu));
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0xBA, 0xC0]
    }
}

/// A live distribution fixture: the same database, network and TSM across
/// multiple distributions, so router learning is observable between them.
struct Harness {
    db: Arc<RwLock<ObjectDatabase>>,
    network: Arc<NetworkLayer<RecordingTransport>>,
    server_tsm: Arc<Mutex<ServerTsm>>,
    notification_transactions: Arc<NotificationTransactions>,
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    comm_state: Arc<AtomicU8>,
    broadcasts: StdArc<StdMutex<Vec<Bytes>>>,
    unicasts: StdArc<StdMutex<Vec<(Vec<u8>, Bytes)>>>,
    retry_timeout_ms: u64,
}

impl Harness {
    async fn new(destinations: Vec<BACnetDestination>, retry_timeout_ms: u64) -> Self {
        Self::new_with_bindings(destinations, retry_timeout_ms, DeviceBindingTable::new()).await
    }

    async fn new_with_bindings(
        destinations: Vec<BACnetDestination>,
        retry_timeout_ms: u64,
        device_bindings: DeviceBindingTable,
    ) -> Self {
        let transport = RecordingTransport::default();
        let broadcasts = StdArc::clone(&transport.broadcasts);
        let unicasts = StdArc::clone(&transport.unicasts);
        let network = Arc::new(NetworkLayer::new(transport));
        let comm_state = Arc::new(AtomicU8::new(0));
        let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
        let notification_transactions = NotificationTransactions::new();

        let mut db = clocked_test_database();
        let mut nc = NotificationClass::new(0, "NC-0").unwrap();
        nc.priority = [255, 255, 255];
        for destination in destinations {
            nc.add_destination(destination);
        }
        db.add(Box::new(nc)).unwrap();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 1,
                name: "Dev".into(),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let mut ai = AnalogInputObject::new(1, "AI-1", 0).unwrap();
        ai.write_property(
            PropertyIdentifier::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
            None,
        )
        .unwrap();
        db.add(Box::new(ai)).unwrap();

        Self {
            db: Arc::new(RwLock::new(db)),
            network,
            server_tsm,
            notification_transactions,
            device_bindings: Arc::new(RwLock::new(device_bindings)),
            comm_state,
            broadcasts,
            unicasts,
            retry_timeout_ms,
        }
    }

    async fn distribute(&self) {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        BACnetServer::<RecordingTransport>::build_and_send_event_notification_with_bindings(
            &self.db,
            &self.network,
            &self.comm_state,
            &self.server_tsm,
            &self.notification_transactions,
            &self.device_bindings,
            &oid,
            (
                EventStateChange {
                    from: EventState::NORMAL,
                    to: EventState::HIGH_LIMIT,
                },
                EventType::OUT_OF_RANGE,
            ),
            self.retry_timeout_ms,
        )
        .await;
        // The confirmed path spawns its send; yield until it reaches the
        // transport.
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    async fn commit_transition(
        &self,
        from: EventState,
        to: EventState,
    ) -> CommittedIntrinsicTransition {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let mut db = self.db.write().await;
        BACnetServer::<RecordingTransport>::commit_intrinsic_transition(
            &mut db,
            &oid,
            bacnet_objects::event::TransitionOutcome {
                change: EventStateChange { from, to },
                event_type: EventType::OUT_OF_RANGE,
                distribute: true,
            },
        )
        .expect("the built-in transition must commit")
    }

    async fn distribute_committed(&self) -> ObjectIdentifier {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let committed = self
            .commit_transition(EventState::NORMAL, EventState::HIGH_LIMIT)
            .await;
        BACnetServer::<RecordingTransport>::build_and_send_event_notification_with_bindings(
            &self.db,
            &self.network,
            &self.comm_state,
            &self.server_tsm,
            &self.notification_transactions,
            &self.device_bindings,
            &oid,
            committed,
            self.retry_timeout_ms,
        )
        .await;
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        oid
    }

    fn broadcast_frames(&self) -> Vec<Bytes> {
        self.broadcasts.lock().unwrap().clone()
    }

    fn unicast_frames(&self) -> Vec<(Vec<u8>, Bytes)> {
        self.unicasts.lock().unwrap().clone()
    }

    /// Deliver an ack the way the dispatch loop would: through the tiered
    /// correlation, carrying the delivering router's MAC and the recipient's
    /// routed identity.
    async fn ack_routed(
        &self,
        router_mac: &[u8],
        network: u16,
        dadr: &[u8],
        invoke_id: u8,
    ) -> bool {
        self.dispatch_terminal(
            router_mac,
            Some(NpduAddress {
                network,
                mac_address: MacAddr::from_slice(dadr),
            }),
            Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            }),
        )
        .await
    }

    async fn dispatch_terminal(
        &self,
        source_mac: &[u8],
        source_network: Option<NpduAddress>,
        apdu: Apdu,
    ) -> bool {
        let active_before = self.notification_transactions.active_count();
        BACnetServer::<RecordingTransport>::dispatch(
            &self.db,
            &self.network,
            &Arc::new(RwLock::new(CovSubscriptionTable::new())),
            &Arc::new(Mutex::new(HashMap::new())),
            &Arc::new(Semaphore::new(MAX_SEG_SENDERS)),
            &Arc::new(Semaphore::new(255)),
            &self.server_tsm,
            &self.notification_transactions,
            &self.device_bindings,
            &self.comm_state,
            &Arc::new(Mutex::new(None::<JoinHandle<()>>)),
            &Arc::new(ServerConfig::default()),
            &None,
            source_mac,
            apdu,
            bacnet_network::layer::ReceivedApdu {
                apdu: Bytes::new(),
                source_mac: MacAddr::from_slice(source_mac),
                source_network,
                link_layer_group: false,
                is_group: false,
                data_attributes: Vec::new(),
                reply_tx: None,
            },
        )
        .await;
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        self.notification_transactions.active_count() < active_before
    }
}

#[tokio::test]
async fn configured_routed_device_retries_unicast_to_router_and_correlates_by_final_peer() {
    let identifier = ObjectIdentifier::new(ObjectType::DEVICE, 90).unwrap();
    let mut bindings = DeviceBindingTable::new();
    bindings
        .insert_configured(
            DeviceBinding::routed(identifier, 1000, RECIPIENT, ROUTER_A).unwrap(),
            |_| false,
        )
        .unwrap();
    let harness = Harness::new_with_bindings(
        vec![destination_for(BACnetRecipient::Device(identifier), true)],
        75,
        bindings,
    )
    .await;
    harness.distribute().await;

    assert!(harness.broadcast_frames().is_empty());
    let first = harness.unicast_frames();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].0.as_slice(), ROUTER_A);
    let (npdu, request) = decode_confirmed(&first[0].1);
    let destination = npdu.destination.unwrap();
    assert_eq!(destination.network, 1000);
    assert_eq!(destination.mac_address.as_ref() as &[u8], RECIPIENT);

    tokio::time::sleep(Duration::from_millis(190)).await;
    let retried = harness.unicast_frames();
    assert!(retried.len() >= 2, "silence triggers a retry");
    for (router, frame) in &retried {
        assert_eq!(router.as_slice(), ROUTER_A, "every attempt uses the router");
        let (npdu, retry) = decode_confirmed(frame);
        let destination = npdu.destination.unwrap();
        assert_eq!(destination.network, 1000);
        assert_eq!(destination.mac_address.as_ref() as &[u8], RECIPIENT);
        assert_eq!(retry.invoke_id, request.invoke_id);
    }
    assert!(
        harness.broadcast_frames().is_empty(),
        "retries never broadcast"
    );
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await,
        "terminal correlation uses the final routed peer identity"
    );
}

#[tokio::test(start_paused = true)]
async fn confirmed_retry_reuses_committed_message_bytes_after_history_changes() {
    let identifier = ObjectIdentifier::new(ObjectType::DEVICE, 92).unwrap();
    let mut bindings = DeviceBindingTable::new();
    bindings
        .insert_configured(
            DeviceBinding::routed(identifier, 1002, RECIPIENT, ROUTER_A).unwrap(),
            |_| false,
        )
        .unwrap();
    let harness = Harness::new_with_bindings(
        vec![destination_for(BACnetRecipient::Device(identifier), true)],
        1_000,
        bindings,
    )
    .await;
    let oid = harness.distribute_committed().await;

    let first = harness.unicast_frames();
    assert_eq!(first.len(), 1);
    let first_frame = first[0].1.clone();
    let (_, first_request) = decode_confirmed(&first_frame);
    let notification = EventNotificationRequest::decode(&first_request.service_request).unwrap();
    assert_eq!(
        notification.message_text,
        Some("ANALOG_INPUT,1: NORMAL -> HIGH_LIMIT".into())
    );
    assert_eq!(
        notification.event_values,
        Some(
            bacnet_services::alarm_event::NotificationParameters::OutOfRange {
                exceeding_value: 0.0,
                status_flags: 0b1000,
                deadband: 1.0,
                exceeded_limit: 100.0,
            }
        )
    );

    {
        let mut db = harness.db.write().await;
        let source = db.get_mut(&oid).unwrap();
        source
            .write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        source
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(99.0),
                None,
            )
            .unwrap();
        source
            .write_property(
                PropertyIdentifier::HIGH_LIMIT,
                None,
                PropertyValue::Real(101.0),
                None,
            )
            .unwrap();
    }

    harness
        .commit_transition(EventState::HIGH_LIMIT, EventState::NORMAL)
        .await;
    harness
        .commit_transition(EventState::NORMAL, EventState::LOW_LIMIT)
        .await;
    let PropertyValue::CharacterString(current_message) = harness
        .db
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_MESSAGE_TEXTS, Some(1))
        .unwrap()
    else {
        panic!("Event_Message_Texts coordinate must be a character string");
    };
    assert_eq!(current_message, "ANALOG_INPUT,1: NORMAL -> LOW_LIMIT");

    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    let retried = harness.unicast_frames();
    assert!(retried.len() >= 2, "silence must trigger a retry");
    assert!(
        retried.iter().all(|(_, frame)| frame == &first_frame),
        "every retry must reuse the originally committed encoded bytes"
    );
    assert!(
        harness
            .ack_routed(ROUTER_A, 1002, RECIPIENT, first_request.invoke_id)
            .await
    );
}

#[tokio::test(start_paused = true)]
async fn observed_routed_device_stops_emitting_when_retry_reaches_expiry() {
    let identifier = ObjectIdentifier::new(ObjectType::DEVICE, 91).unwrap();
    let mut bindings = DeviceBindingTable::new();
    let observed_at = Instant::now() - OBSERVED_BINDING_TTL + Duration::from_millis(500);
    let source = NpduAddress {
        network: 1001,
        mac_address: MacAddr::from_slice(RECIPIENT),
    };
    assert_eq!(
        bindings.observe_i_am_at(identifier, ROUTER_A, Some(&source), observed_at, |_| false),
        super::device_bindings::ObservationOutcome::Inserted
    );
    let harness = Harness::new_with_bindings(
        vec![destination_for(BACnetRecipient::Device(identifier), true)],
        1_000,
        bindings,
    )
    .await;
    harness.distribute().await;

    assert_eq!(harness.unicast_frames().len(), 1);
    tokio::time::advance(Duration::from_secs(1)).await;
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }

    assert_eq!(
        harness.unicast_frames().len(),
        1,
        "an observed route cannot emit at or after its expiry boundary"
    );
    assert!(harness.broadcast_frames().is_empty());
}

/// Decode a captured frame into its NPDU and the confirmed request inside.
fn decode_confirmed(frame: &Bytes) -> (bacnet_encoding::npdu::Npdu, ConfirmedRequestPdu) {
    let npdu = bacnet_encoding::npdu::decode_npdu(frame.clone()).expect("decode NPDU");
    match apdu::decode_apdu(npdu.payload.clone()).expect("decode APDU") {
        Apdu::ConfirmedRequest(req) => (npdu, req),
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    }
}

const ROUTER_A: &[u8] = &[10, 0, 0, 1, 0xBA, 0xC0];
const RECIPIENT: &[u8] = &[0x0A; 6];

/// #375's headline: a confirmed notification to a remote recipient goes out
/// as a routed NPDU on the broadcast DA (Clause 6.5.3 unknown-router form)
/// with 'data_expecting_reply' set, and the routed-identity ack completes the
/// transaction — no retries, no duplicate deliveries.
#[tokio::test]
async fn confirmed_remote_recipient_delivers_and_correlates() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        150,
    )
    .await;
    harness.distribute().await;

    let broadcasts = harness.broadcast_frames();
    assert_eq!(broadcasts.len(), 1, "one routed send on the broadcast DA");
    assert!(harness.unicast_frames().is_empty());
    let (npdu, req) = decode_confirmed(&broadcasts[0]);
    assert!(npdu.expecting_reply, "a confirmed send expects its ack");
    let destination = npdu.destination.expect("routed NPDU names the recipient");
    assert_eq!(destination.network, 1000);
    assert_eq!(destination.mac_address.as_ref() as &[u8], RECIPIENT);
    assert_eq!(
        req.service_choice,
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION
    );

    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, req.invoke_id)
            .await,
        "the routed-identity ack must find the transaction"
    );

    // Past the retry timeout: an unacknowledged transaction would have
    // retried by now.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        harness.broadcast_frames().len(),
        1,
        "an acknowledged notification must not retry"
    );
}

/// The issue's discrimination note: a single transaction cannot tell routed
/// keying from the legacy wildcard fallback, so two concurrent routed
/// recipients must correlate independently — acking one leaves the other
/// retrying.
#[tokio::test]
async fn two_routed_recipients_correlate_independently() {
    let harness = Harness::new(
        vec![
            destination_for(address_recipient(1000, RECIPIENT), true),
            destination_for(address_recipient(2000, &[0x0B; 6]), true),
        ],
        200,
    )
    .await;
    harness.distribute().await;

    let initial = harness.broadcast_frames();
    assert_eq!(initial.len(), 2, "both recipients get their send");
    let by_net: Vec<(u16, u8)> = initial
        .iter()
        .map(|f| {
            let (npdu, req) = decode_confirmed(f);
            (npdu.destination.unwrap().network, req.invoke_id)
        })
        .collect();
    let invoke_2000 = by_net.iter().find(|(n, _)| *n == 2000).unwrap().1;

    assert!(
        harness
            .ack_routed(ROUTER_A, 2000, &[0x0B; 6], invoke_2000)
            .await
    );

    // Past one retry timeout: the unacknowledged 1000-network transaction
    // retries; the acknowledged 2000-network one must not.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = harness.broadcast_frames();
    let retries_1000 = after
        .iter()
        .skip(2)
        .filter(|f| decode_confirmed(f).0.destination.unwrap().network == 1000)
        .count();
    let retries_2000 = after
        .iter()
        .skip(2)
        .filter(|f| decode_confirmed(f).0.destination.unwrap().network == 2000)
        .count();
    assert!(retries_1000 >= 1, "the unacked recipient keeps retrying");
    assert_eq!(retries_2000, 0, "the acked recipient must not retry");
}

/// An ack naming a different routed identity must not complete the
/// transaction — the tiers are exact lookups, not a wildcard.
#[tokio::test]
async fn ack_with_wrong_routed_identity_does_not_complete() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    harness.distribute().await;
    let (_, req) = decode_confirmed(&harness.broadcast_frames()[0]);

    assert!(
        !harness
            .ack_routed(ROUTER_A, 2000, RECIPIENT, req.invoke_id)
            .await,
        "wrong DNET must miss"
    );
    assert!(
        !harness
            .ack_routed(ROUTER_A, 1000, &[0x0C; 6], req.invoke_id)
            .await,
        "wrong DADR must miss"
    );
    assert_eq!(
        harness.server_tsm.lock().await.cached_router(1000),
        None,
        "mismatched routed traffic must not teach the router cache"
    );
    assert!(
        !harness
            .dispatch_terminal(
                ROUTER_A,
                Some(NpduAddress {
                    network: 1000,
                    mac_address: MacAddr::from_slice(RECIPIENT),
                }),
                Apdu::SimpleAck(SimpleAck {
                    invoke_id: req.invoke_id,
                    service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                }),
            )
            .await,
        "wrong service must not complete"
    );
    assert_eq!(harness.server_tsm.lock().await.cached_router(1000), None);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, req.invoke_id)
            .await,
        "the true identity still completes"
    );
}

/// Clause 6.5.3 method 4: the ack's source MAC is the router for that DNET,
/// so the next confirmed notification to the same network unicasts to it —
/// carrying the same DNET/DADR NPDU — instead of broadcasting.
#[tokio::test]
async fn learned_router_unicasts_the_next_notification() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        150,
    )
    .await;
    harness.distribute().await;
    let (_, req) = decode_confirmed(&harness.broadcast_frames()[0]);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, req.invoke_id)
            .await
    );

    harness.distribute().await;
    let unicasts = harness.unicast_frames();
    assert_eq!(
        unicasts.len(),
        1,
        "the second notification unicasts to the learned router"
    );
    let (router_mac, frame) = &unicasts[0];
    assert_eq!(router_mac.as_slice(), ROUTER_A);
    let (npdu, req2) = decode_confirmed(frame);
    assert!(npdu.expecting_reply);
    let destination = npdu.destination.unwrap();
    assert_eq!(destination.network, 1000);
    assert_eq!(destination.mac_address.as_ref() as &[u8], RECIPIENT);

    // Cleanup so the retry task ends promptly.
    let _ = harness
        .ack_routed(ROUTER_A, 1000, RECIPIENT, req2.invoke_id)
        .await;
}

/// A learned router can vanish: the first attempt unicasts to it, and the
/// retry after silence falls back to the always-correct broadcast DA.
#[tokio::test]
async fn retry_after_silent_router_falls_back_to_broadcast() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        150,
    )
    .await;
    harness.distribute().await;
    let (_, req) = decode_confirmed(&harness.broadcast_frames()[0]);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, req.invoke_id)
            .await
    );

    // Second notification: attempt 0 unicasts to the learned router, which
    // stays silent; the retry must arrive as a broadcast-DA frame.
    harness.distribute().await;
    assert_eq!(harness.unicast_frames().len(), 1);
    assert_eq!(harness.notification_transactions.active_count(), 1);
    let broadcasts_before = harness.broadcast_frames().len();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let broadcasts = harness.broadcast_frames();
    assert!(
        broadcasts.len() > broadcasts_before,
        "the retry falls back to the broadcast DA"
    );
    let (npdu, req2) = decode_confirmed(broadcasts.last().unwrap());
    let destination = npdu.destination.unwrap();
    assert_eq!(destination.network, 1000);
    assert_eq!(
        req2.invoke_id,
        decode_confirmed(&harness.unicast_frames()[0].1).1.invoke_id
    );
    assert_eq!(
        harness.notification_transactions.active_count(),
        1,
        "the retry must retain one active lease"
    );

    let _ = harness
        .ack_routed(ROUTER_A, 1000, RECIPIENT, req2.invoke_id)
        .await;
}

/// A local-unicast confirmed recipient still works exactly as before — the
/// route split must not disturb the existing path.
#[tokio::test]
async fn local_unicast_confirmed_recipient_still_unicasts() {
    let target = [192, 168, 1, 50, 0xBA, 0xC0];
    let harness = Harness::new(
        vec![BACnetDestination {
            recipient: address_recipient(0, &target),
            issue_confirmed_notifications: true,
            ..local_broadcast_destination()
        }],
        60_000,
    )
    .await;
    harness.distribute().await;

    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 1);
    assert_eq!(unicasts[0].0.as_slice(), &target);
    let (npdu, req) = decode_confirmed(&unicasts[0].1);
    assert!(npdu.expecting_reply);
    assert!(npdu.destination.is_none(), "a local unicast is not routed");
    assert!(harness.broadcast_frames().is_empty());

    // Complete it the way the dispatch loop would for a local peer.
    let hit = self_ack_local(&harness, &target, req.invoke_id).await;
    assert!(hit, "the local exact key still correlates");
}

async fn self_ack_local(harness: &Harness, mac: &[u8], invoke_id: u8) -> bool {
    harness
        .dispatch_terminal(
            mac,
            None,
            Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
            }),
        )
        .await
}
