//! Received event notifications through the Notification Forwarder objects
//! (#1225, Clause 12.51): the destinations each list names, the filters, the
//! loop rules, expiry and chains within the device.

use super::event_forwarding::Reception;
use super::event_recipient_routing_tests::{LITERAL_BROADCAST_MAC, address_recipient};
use super::*;
use crate::server::test_transport::{BIP_LOCAL_MAC, SendLog, TestTransport, TestTransportHandle};
use bacnet_encoding::constructed::{
    encode_event_notification, encode_event_notification_subscription_list,
};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_objects::traits::{BACnetObject, MonotonicClock};
use bacnet_services::alarm_event::{ForwardedEventNotification, NotificationParameters};
use bacnet_transport::port::TransportProvenance;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetDestination, BACnetEventNotificationSubscription, BACnetPortPermission, BACnetRecipient,
};
use bacnet_types::enums::{EventState, EventType};
use bacnet_types::primitives::{BACnetTimeStamp, StatusFlags, Time};

/// The forwarding device's Device instance.
pub(super) const LOCAL_DEVICE: u32 = 1;
pub(super) const PEER_A: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC0];
pub(super) const PEER_B: [u8; 6] = [10, 0, 0, 3, 0xBA, 0xC0];
const REMOTE_MAC: [u8; 2] = [0x0A, 0x0B];

/// Where a sent copy went, read back from its NPDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum To {
    Local(Vec<u8>),
    LocalBroadcast,
    Remote(u16, Vec<u8>),
    RemoteBroadcast(u16),
    Global,
}

/// One forwarded copy as it left the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Copy {
    pub(super) to: To,
    pub(super) confirmed: bool,
    pub(super) process_identifier: u32,
}

/// An alarm from a remote device, addressed to process `process_identifier`.
pub(super) fn notification(process_identifier: u32) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 50).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(9),
        notification_class: 4,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("hot".into()),
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: Some(NotificationParameters::OutOfRange {
            exceeding_value: 81.0,
            status_flags: StatusFlags::IN_ALARM,
            deadband: 1.0,
            exceeded_limit: 80.0,
        }),
    }
}

pub(super) fn encoded(request: &EventNotificationRequest) -> Bytes {
    let mut buf = BytesMut::new();
    encode_event_notification(request, &mut buf).unwrap();
    buf.freeze()
}

/// A destination open on every day, at every time, for every transition.
pub(super) fn destination(
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

pub(super) fn subscribe(
    forwarder: &mut NotificationForwarderObject,
    subscriptions: &[BACnetEventNotificationSubscription],
) {
    let mut buf = BytesMut::new();
    encode_event_notification_subscription_list(&mut buf, subscriptions).unwrap();
    forwarder
        .write_property(
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
            None,
            PropertyValue::ApplicationData(buf.to_vec()),
            None,
        )
        .unwrap();
}

pub(super) fn local_device() -> DeviceObject {
    DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(LOCAL_DEVICE, 0)
            .expect("valid Device identity")
            .with_name("Forwarding device"),
        ..DeviceConfig::default()
    })
    .unwrap()
}

pub(super) fn database(forwarders: Vec<NotificationForwarderObject>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(local_device())).unwrap();
    for forwarder in forwarders {
        db.add(Box::new(forwarder)).unwrap();
    }
    db
}

pub(super) fn forwarding_transport() -> TestTransport {
    TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast_mac(LITERAL_BROADCAST_MAC)
        .build()
}

/// Read the copies a transport sent, checking that each carries the original
/// notification with only the process identifier changed.
pub(super) fn copies(sent: &SendLog, original: &EventNotificationRequest) -> Vec<Copy> {
    sent.take()
        .into_iter()
        .map(|frame| {
            let npdu = frame.decode_npdu();
            let to = match (npdu.destination, frame.broadcast) {
                (None, false) => To::Local(frame.mac.to_vec()),
                (None, true) => To::LocalBroadcast,
                (Some(dest), _) if dest.network == 0xFFFF => To::Global,
                (Some(dest), _) if dest.mac_address.is_empty() => To::RemoteBroadcast(dest.network),
                (Some(dest), _) => To::Remote(dest.network, dest.mac_address.to_vec()),
            };
            let (confirmed, service) = match frame.apdu() {
                Apdu::UnconfirmedRequest(request) => {
                    assert_eq!(
                        request.service_choice,
                        UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION
                    );
                    (false, request.service_request)
                }
                Apdu::ConfirmedRequest(request) => {
                    assert_eq!(
                        request.service_choice,
                        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION
                    );
                    (true, request.service_request)
                }
                other => panic!("expected an event notification, got {other:?}"),
            };
            let forwarded = ForwardedEventNotification::decode(&service).unwrap();
            assert_eq!(
                forwarded.encode_for(original.process_identifier),
                encoded(original),
                "a copy differs from the original only in its process identifier"
            );
            Copy {
                to,
                confirmed,
                process_identifier: forwarded.process_identifier,
            }
        })
        .collect()
}

pub(super) fn unconfirmed(to: To, process_identifier: u32) -> Copy {
    Copy {
        to,
        confirmed: false,
        process_identifier,
    }
}

/// A server's unconfirmed-request handles around one forwarding database.
pub(super) struct Forwarding {
    pub(super) db: Arc<RwLock<ObjectDatabase>>,
    pub(super) sent: SendLog,
    pub(super) handle: TestTransportHandle,
    services: UnconfirmedServices<TestTransport>,
}

impl Forwarding {
    pub(super) fn new(db: ObjectDatabase) -> Self {
        let transport = forwarding_transport();
        let sent = transport.sent();
        let handle = transport.handle();
        let db = Arc::new(RwLock::new(db));
        let services = UnconfirmedServices {
            db: Arc::clone(&db),
            ..UnconfirmedServices::for_test(
                Arc::new(NetworkLayer::new(transport)),
                ServerConfig::default(),
            )
        };
        Self {
            db,
            sent,
            handle,
            services,
        }
    }

    /// Hand the server one UnconfirmedEventNotification addressed as
    /// `reception` says, and return the copies it sent.
    pub(super) async fn receive(
        &self,
        request: &EventNotificationRequest,
        reception: Reception,
    ) -> Vec<Copy> {
        self.deliver(request, reception).await;
        copies(&self.sent, request)
    }

    /// [`Self::receive`], leaving what the server sent in `sent`: a Who-Is
    /// for an unbound Device destination (#1368) is no copy.
    pub(super) async fn deliver(&self, request: &EventNotificationRequest, reception: Reception) {
        BACnetServer::<TestTransport>::handle_unconfirmed_request(
            &self.services,
            UnconfirmedRequestPdu {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
                service_request: encoded(request),
            },
            &bacnet_network::layer::ReceivedApdu {
                router_response: None,
                direct_response: None,
                apdu: Bytes::new(),
                source_mac: MacAddr::from_slice(&PEER_B),
                ingress_network: None,
                source_network: None,
                link_layer_group: reception.group,
                is_group: reception.group,
                global_broadcast: reception.global,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            },
        )
        .await;
        // Confirmed copies are sent from spawned workers.
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    /// Record Device `instance`'s I-Am from `mac` on this network, as the
    /// I-Am handler does, and let what waited for it go out.
    pub(super) async fn hear_i_am(&self, instance: u32, mac: &[u8]) {
        let device = ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
        self.services.device_bindings.write().await.observe_i_am_at(
            device,
            mac,
            None,
            Instant::now(),
            |_| false,
        );
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    pub(super) fn counters(&self) -> EventNotificationCounters {
        self.services.event_suppressions.snapshot()
    }

    /// The server's throttle on the forwarding-cap warning.
    pub(super) fn cap_warning(&self) -> &WarnThrottle {
        &self.services.event_suppressions.cap_warning
    }

    /// Give the network layer `number` as the local network's number, as
    /// the server's startup or its Number worker publishes it.
    pub(super) fn set_local_network(&self, number: u16) {
        let state = bacnet_types::network_number::NetworkNumber::configured(number).unwrap();
        self.services.network.local_network_number().publish(state);
    }

    /// Configure Device `instance` as the local node at `mac`.
    pub(super) async fn bind_device(&self, instance: u32, mac: &[u8]) {
        let device = ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
        self.services
            .device_bindings
            .write()
            .await
            .insert_configured(DeviceBinding::local(device, mac).unwrap(), |_| false)
            .unwrap();
    }
}

/// A forwarder with one destination of each address shape: a local unicast,
/// a local broadcast, a remote unicast, a remote broadcast and the global
/// broadcast, processes 1 to 5.
fn every_route() -> NotificationForwarderObject {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    for (process, recipient) in [
        address_recipient(0, &PEER_A),
        address_recipient(0, &[]),
        address_recipient(5, &REMOTE_MAC),
        address_recipient(6, &[]),
        address_recipient(0xFFFF, &[]),
    ]
    .into_iter()
    .enumerate()
    {
        nf.add_destination(destination(recipient, process as u32 + 1, false))
            .unwrap();
    }
    nf
}

#[tokio::test]
async fn received_notification_goes_to_recipient_list_and_subscriptions() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, false))
        .unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_B), 41, true))
        .unwrap();
    subscribe(
        &mut nf,
        &[BACnetEventNotificationSubscription {
            recipient: address_recipient(7, &REMOTE_MAC),
            process_identifier: 50,
            issue_confirmed_notifications: false,
            time_remaining: 30,
        }],
    );
    let forwarding = Forwarding::new(database(vec![nf]));
    let sent = forwarding
        .receive(&notification(5), Reception::UNICAST)
        .await;
    assert_eq!(
        sent,
        [
            unconfirmed(To::Local(PEER_A.to_vec()), 40),
            unconfirmed(To::Remote(7, REMOTE_MAC.to_vec()), 50),
            Copy {
                to: To::Local(PEER_B.to_vec()),
                confirmed: true,
                process_identifier: 41,
            },
        ]
    );
}

#[tokio::test]
async fn a_forwarded_copy_whose_send_fails_is_counted_once() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, false))
        .unwrap();
    nf.add_destination(destination(address_recipient(0, &PEER_B), 41, false))
        .unwrap();
    let forwarding = Forwarding::new(database(vec![nf]));
    forwarding.handle.fail_next_send();
    // A failed send is still logged: both copies were attempted, and the
    // second went out after the first failed.
    assert_eq!(
        forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await,
        [
            unconfirmed(To::Local(PEER_A.to_vec()), 40),
            unconfirmed(To::Local(PEER_B.to_vec()), 41),
        ]
    );
    assert_eq!(
        forwarding.counters(),
        EventNotificationCounters {
            unconfirmed_send_failed: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn process_identifier_filter_and_out_of_service_gate_forwarding() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.set_process_identifier_filter(Some(7));
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, false))
        .unwrap();
    let forwarding = Forwarding::new(database(vec![nf]));
    assert!(
        forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await
            .is_empty()
    );
    assert_eq!(
        forwarding
            .receive(&notification(7), Reception::UNICAST)
            .await,
        [unconfirmed(To::Local(PEER_A.to_vec()), 40)]
    );
    let forwarder = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap();
    forwarding
        .db
        .write()
        .await
        .get_mut(&forwarder)
        .unwrap()
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert!(
        forwarding
            .receive(&notification(7), Reception::UNICAST)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn local_forwarding_only_ignores_received_notifications() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.set_local_forwarding_only(true);
    nf.add_destination(destination(address_recipient(0, &PEER_A), 40, false))
        .unwrap();
    let forwarding = Forwarding::new(database(vec![nf]));
    assert!(
        forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn loop_rules_keep_copies_off_the_receiving_network_and_global_broadcast() {
    let forwarding = Forwarding::new(database(vec![every_route()]));
    // Addressed to this device: no copy is broadcast back onto its network,
    // and none goes by global broadcast.
    assert_eq!(
        forwarding
            .receive(&notification(9), Reception::UNICAST)
            .await,
        [
            unconfirmed(To::Local(PEER_A.to_vec()), 1),
            unconfirmed(To::Remote(5, REMOTE_MAC.to_vec()), 3),
            unconfirmed(To::RemoteBroadcast(6), 4),
        ]
    );
    // Broadcast on this network: every node here already has it.
    let broadcast = Reception {
        group: true,
        global: false,
    };
    assert_eq!(
        forwarding.receive(&notification(9), broadcast).await,
        [
            unconfirmed(To::Remote(5, REMOTE_MAC.to_vec()), 3),
            unconfirmed(To::RemoteBroadcast(6), 4),
        ]
    );
    // A global broadcast is not forwarded at all.
    let global = Reception {
        group: true,
        global: true,
    };
    assert!(
        forwarding
            .receive(&notification(9), global)
            .await
            .is_empty()
    );
    assert_eq!(
        forwarding.counters(),
        EventNotificationCounters::default(),
        "loop rules are configured behaviour, not failed deliveries"
    );
}

#[tokio::test]
async fn port_filter_disabling_port_zero_stops_received_notifications() {
    let mut nf = every_route();
    nf.set_port_filter(Some(vec![BACnetPortPermission {
        port_id: 0,
        enabled: false,
    }]));
    let forwarding = Forwarding::new(database(vec![nf]));
    assert!(
        forwarding
            .receive(&notification(9), Reception::UNICAST)
            .await
            .is_empty()
    );
    let forwarder = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap();
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_port_permission(
        &mut buf,
        &BACnetPortPermission {
            port_id: 0,
            enabled: true,
        },
    );
    forwarding
        .db
        .write()
        .await
        .get_mut(&forwarder)
        .unwrap()
        .write_property(
            PropertyIdentifier::PORT_FILTER,
            Some(1),
            PropertyValue::ApplicationData(buf.to_vec()),
            None,
        )
        .unwrap();
    assert_eq!(
        forwarding
            .receive(&notification(9), Reception::UNICAST)
            .await
            .len(),
        3
    );
}

#[tokio::test]
async fn a_lapsed_subscription_is_not_forwarded_to() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    subscribe(
        &mut nf,
        &[
            BACnetEventNotificationSubscription {
                recipient: address_recipient(0, &PEER_A),
                process_identifier: 60,
                issue_confirmed_notifications: false,
                time_remaining: 1,
            },
            BACnetEventNotificationSubscription {
                recipient: address_recipient(0, &PEER_B),
                process_identifier: 61,
                issue_confirmed_notifications: false,
                time_remaining: 5,
            },
        ],
    );
    let mut db = database(vec![nf]);
    let nanos = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let read = Arc::clone(&nanos);
    let clock: Arc<MonotonicClock> =
        Arc::new(move || Duration::from_nanos(read.load(Ordering::SeqCst)));
    db.set_monotonic_clock_internal(Some(clock));
    let forwarding = Forwarding::new(db);
    assert_eq!(
        forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await
            .len(),
        2
    );
    nanos.store(60_000_000_000, Ordering::SeqCst);
    assert_eq!(
        forwarding
            .receive(&notification(5), Reception::UNICAST)
            .await,
        [unconfirmed(To::Local(PEER_B.to_vec()), 61)]
    );
}

#[tokio::test]
async fn forwarders_chain_within_the_device_and_each_takes_a_notification_once() {
    let local =
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, LOCAL_DEVICE).unwrap());
    // A takes process 1 and hands it on to this device as process 2; B takes
    // process 2, sends it out as process 3 and hands it back as process 1.
    let mut a = NotificationForwarderObject::new(1, "A").unwrap();
    a.set_process_identifier_filter(Some(1));
    a.add_destination(destination(local.clone(), 2, false))
        .unwrap();
    let mut b = NotificationForwarderObject::new(2, "B").unwrap();
    b.set_process_identifier_filter(Some(2));
    b.add_destination(destination(address_recipient(0, &PEER_A), 3, false))
        .unwrap();
    b.add_destination(destination(local, 1, false)).unwrap();
    let forwarding = Forwarding::new(database(vec![a, b]));
    assert_eq!(
        forwarding
            .receive(&notification(1), Reception::UNICAST)
            .await,
        [unconfirmed(To::Local(PEER_A.to_vec()), 3)]
    );
    assert_eq!(forwarding.counters(), EventNotificationCounters::default());
}

#[tokio::test]
async fn malformed_unconfirmed_notification_is_ignored() {
    let forwarding = Forwarding::new(database(vec![every_route()]));
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &forwarding.services,
        UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
            service_request: encoded(&notification(9)).slice(..10),
        },
        &bacnet_network::layer::ReceivedApdu {
            router_response: None,
            direct_response: None,
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(&PEER_B),
            ingress_network: None,
            source_network: None,
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        },
    )
    .await;
    assert!(forwarding.sent.is_empty());
}
