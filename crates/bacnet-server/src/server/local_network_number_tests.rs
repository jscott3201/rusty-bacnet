//! The local network's number in event routing (#1298, #1299). A started
//! server takes it from a registered Network Port's Network_Number or, with
//! no port, learns it from Network-Number-Is. The forwarders' loop rules then
//! take a recipient on that network as local, and its copy, forwarded or
//! from a Notification Class, goes with no DNET; a recipient on another
//! network still gets a routed copy.

use super::device_bindings::{DeviceBinding, DeviceBindingTable};
use super::event_forwarding::Reception;
use super::event_forwarding_tests::{
    Copy, LOCAL_DEVICE, PEER_A, PEER_B, To, copies, database, destination, encoded, notification,
    unconfirmed,
};
use super::event_recipient_routing_tests::{
    LITERAL_BROADCAST_MAC, address_recipient, distribute_counted_through,
};
use super::test_transport::{BIP_LOCAL_MAC, SendLog, SentFrame, TestTransport};
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_objects::network_port::{BipPortConfig, NetworkPortObject};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::notification_forwarder::NotificationForwarderObject;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::BACnetRecipient;
use std::net::SocketAddrV4;

/// The number of the network this device is attached to.
const THIS_NETWORK: u16 = 7;
const REMOTE_NETWORK: u16 = 5;
const REMOTE_MAC: [u8; 2] = [0x0E, 0x0F];
const PEER_C: [u8; 6] = [10, 0, 0, 4, 0xBA, 0xC0];
const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];
const PORT_IP: [u8; 4] = [127, 0, 0, 1];

const BROADCAST: Reception = Reception {
    group: true,
    global: false,
};

fn port_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

async fn bounded<F: std::future::Future>(f: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(5), f)
        .await
        .expect("the server made no progress")
}

/// A forwarder sending to the broadcast on this network and to a node on
/// it, both named by number, and last to a node on another network,
/// processes 1 to 3.
fn numbered_recipients() -> NotificationForwarderObject {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    for (process, recipient) in [
        (1, address_recipient(THIS_NETWORK, &[])),
        (2, address_recipient(THIS_NETWORK, &PEER_A)),
        (3, address_recipient(REMOTE_NETWORK, &REMOTE_MAC)),
    ] {
        nf.add_destination(destination(recipient, process, false))
            .unwrap();
    }
    nf
}

fn remote_copy() -> Copy {
    unconfirmed(To::Remote(REMOTE_NETWORK, REMOTE_MAC.to_vec()), 3)
}

/// A server started over a link that runs the Network Number controls.
struct Started {
    server: BACnetServer<TestTransport>,
    inbound: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
}

impl Started {
    /// Start a server over `db`, registering a B/IP Network Port configured
    /// with `port_number` when one is given.
    async fn new(mut db: ObjectDatabase, port_number: Option<u16>) -> Self {
        let (inbound, rx) = mpsc::channel(8);
        let mut transport = TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .broadcast_mac(LITERAL_BROADCAST_MAC)
            .number_controls()
            .inbound(rx);
        let mut config = ServerConfig::default();
        if let Some(network_number) = port_number {
            let config_port = BipPortConfig {
                network_number,
                ip_address: PORT_IP,
                ..BipPortConfig::default()
            };
            db.add(Box::new(
                NetworkPortObject::new_bip(1, "Port", config_port).unwrap(),
            ))
            .unwrap();
            config.registered_network_port = Some(port_oid());
            transport = transport.normal_bip(SocketAddrV4::new(PORT_IP.into(), 47808));
        }
        let transport = transport.build();
        let sent = transport.sent();
        let server = BACnetServer::start(config, db, transport).await.unwrap();
        Self {
            server,
            inbound,
            sent,
        }
    }

    /// Hand the server one NPDU from `PEER_B`, by link broadcast when `group`.
    async fn feed(&self, npdu: &[u8], group: bool) {
        self.inbound
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: Bytes::copy_from_slice(npdu),
                source_mac: MacAddr::from_slice(&PEER_B),
                link_layer_group: group,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
    }

    /// Broadcast Network-Number-Is for `number` with configured flag `flag`,
    /// then return the number the server gives when asked. Its worker takes
    /// controls in order, so the answer comes after the announcement applied.
    async fn announce(&self, number: u16, flag: u8) -> u16 {
        let [high, low] = number.to_be_bytes();
        self.feed(&[1, 0x80, 0x13, high, low, flag], true).await;
        self.feed(&[1, 0x80, 0x12], false).await;
        bounded(self.sent.wait_for_len(1)).await;
        let frames = self.sent.take();
        match &frames[..] {
            [
                SentFrame {
                    npdu,
                    broadcast: true,
                    ..
                },
            ] if npdu.len() == 6 && npdu[..3] == [1, 0x80, 0x13] => {
                u16::from_be_bytes([npdu[3], npdu[4]])
            }
            _ => panic!("expected one Network-Number-Is broadcast, got {frames:?}"),
        }
    }

    /// Hand the server `request` as an UnconfirmedEventNotification from
    /// `PEER_B`, addressed as `reception` says, and return the copies sent.
    /// The copies go out in list order and the last destination is always
    /// on the remote network, so they are all sent once that one is.
    async fn receive(&self, request: &EventNotificationRequest, reception: Reception) -> Vec<Copy> {
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
                service_request: encoded(request),
            }),
        )
        .unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                payload: apdu.freeze(),
                ..Default::default()
            },
        )
        .unwrap();
        self.feed(&npdu, reception.group).await;
        let remote_sent = |frames: Vec<SentFrame>| {
            frames.iter().any(|frame| {
                frame
                    .decode_npdu()
                    .destination
                    .is_some_and(|to| to.network == REMOTE_NETWORK)
            })
        };
        bounded(async {
            while !remote_sent(self.sent.frames()) {
                let len = self.sent.len();
                self.sent.wait_for_len(len + 1).await;
            }
        })
        .await;
        copies(&self.sent, request)
    }

    /// The registered port's Network_Number and Network_Number_Quality.
    async fn port_number(&self) -> (u16, u8) {
        let db = self.server.database().read().await;
        let port = db.get(&port_oid()).unwrap();
        let read = |property| match port.read_property(property, None) {
            Ok(PropertyValue::Unsigned(value)) => value as u16,
            Ok(PropertyValue::Enumerated(value)) => value as u16,
            other => panic!("unexpected {property:?}: {other:?}"),
        };
        (
            read(PropertyIdentifier::NETWORK_NUMBER),
            read(PropertyIdentifier::NETWORK_NUMBER_QUALITY) as u8,
        )
    }

    async fn stop(mut self) {
        self.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_number_learned_without_a_network_port_makes_this_network_local() {
    let started = Started::new(database(vec![numbered_recipients()]), None).await;
    assert_eq!(started.announce(THIS_NETWORK, 0).await, THIS_NETWORK);
    // By broadcast on this network: no copy goes back onto it, neither as a
    // broadcast nor to a node there.
    assert_eq!(
        started.receive(&notification(9), BROADCAST).await,
        [remote_copy()]
    );
    // To this device alone: the node here gets its copy with no DNET.
    assert_eq!(
        started.receive(&notification(9), Reception::UNICAST).await,
        [unconfirmed(To::Local(PEER_A.to_vec()), 2), remote_copy()]
    );
    // A configured number announced later replaces the learned one, and
    // the network numbered 7 is remote again.
    assert_eq!(started.announce(8, 1).await, 8);
    assert_eq!(
        started.receive(&notification(9), BROADCAST).await,
        [
            unconfirmed(To::RemoteBroadcast(THIS_NETWORK), 1),
            unconfirmed(To::Remote(THIS_NETWORK, PEER_A.to_vec()), 2),
            remote_copy(),
        ]
    );
    started.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_registered_port_and_the_routes_share_one_network_number() {
    // Configured on the port: local from startup, and an announcement of
    // another number changes neither the port nor the routes.
    let started = Started::new(database(vec![numbered_recipients()]), Some(THIS_NETWORK)).await;
    assert_eq!(
        started.receive(&notification(9), Reception::UNICAST).await,
        [unconfirmed(To::Local(PEER_A.to_vec()), 2), remote_copy()]
    );
    assert_eq!(started.announce(8, 1).await, THIS_NETWORK);
    assert_eq!(started.port_number().await, (THIS_NETWORK, 3));
    assert_eq!(
        started.receive(&notification(9), BROADCAST).await,
        [remote_copy()]
    );
    started.stop().await;

    // Unknown on the port until learned: the port and the routes then hold
    // the learned number together.
    let started = Started::new(database(vec![numbered_recipients()]), Some(0)).await;
    assert_eq!(
        started.receive(&notification(9), BROADCAST).await,
        [
            unconfirmed(To::RemoteBroadcast(THIS_NETWORK), 1),
            unconfirmed(To::Remote(THIS_NETWORK, PEER_A.to_vec()), 2),
            remote_copy(),
        ]
    );
    assert_eq!(started.announce(THIS_NETWORK, 0).await, THIS_NETWORK);
    assert_eq!(started.port_number().await, (THIS_NETWORK, 1));
    assert_eq!(
        started.receive(&notification(9), Reception::UNICAST).await,
        [unconfirmed(To::Local(PEER_A.to_vec()), 2), remote_copy()]
    );
    started.stop().await;
}

/// Each notification `distribute_counted_through` saw, by process
/// identifier: where it went and whether it was confirmed.
fn routes(broadcasts: Vec<Bytes>, unicasts: Vec<(Vec<u8>, Bytes)>) -> Vec<(u32, To, bool)> {
    let read = |npdu: Bytes| {
        let npdu = decode_npdu(npdu).unwrap();
        let (confirmed, request) = match decode_apdu(npdu.payload).unwrap() {
            Apdu::UnconfirmedRequest(request) => (false, request.service_request),
            Apdu::ConfirmedRequest(request) => (true, request.service_request),
            other => panic!("expected an event notification, got {other:?}"),
        };
        let process = decode_event_notification(&request)
            .unwrap()
            .process_identifier;
        (npdu.destination, process, confirmed)
    };
    let mut routes: Vec<_> = unicasts
        .into_iter()
        .map(|(mac, npdu)| {
            let (destination, process, confirmed) = read(npdu);
            let to = match destination {
                None => To::Local(mac),
                Some(to) => To::Remote(to.network, to.mac_address.to_vec()),
            };
            (process, to, confirmed)
        })
        .chain(broadcasts.into_iter().map(|npdu| {
            let (destination, process, confirmed) = read(npdu);
            let to = match destination {
                None => To::LocalBroadcast,
                Some(to) if to.mac_address.is_empty() => To::RemoteBroadcast(to.network),
                Some(to) => To::Remote(to.network, to.mac_address.to_vec()),
            };
            (process, to, confirmed)
        }))
        .collect();
    routes.sort_by_key(|(process, ..)| *process);
    routes
}

#[tokio::test(start_paused = true)]
async fn notification_class_and_forwarded_copies_to_this_network_go_without_a_dnet() {
    let started = Started::new(database(Vec::new()), None).await;
    assert_eq!(started.announce(THIS_NETWORK, 0).await, THIS_NETWORK);

    // A Device bound through a router to a node on this network.
    let bound = ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap();
    let mut bindings = DeviceBindingTable::new();
    bindings
        .insert_configured(
            DeviceBinding::routed(bound, THIS_NETWORK, PEER_C, ROUTER).unwrap(),
            |_| false,
        )
        .unwrap();
    let this_device =
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, LOCAL_DEVICE).unwrap());
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    for (process, recipient, confirmed) in [
        (1, address_recipient(THIS_NETWORK, &PEER_A), false),
        (2, address_recipient(THIS_NETWORK, &[]), false),
        (
            3,
            address_recipient(THIS_NETWORK, LITERAL_BROADCAST_MAC),
            false,
        ),
        (4, address_recipient(THIS_NETWORK, &PEER_B), true),
        (5, BACnetRecipient::Device(bound), false),
        (6, address_recipient(REMOTE_NETWORK, &REMOTE_MAC), false),
        (7, this_device, false),
    ] {
        class
            .add_destination(destination(recipient, process, confirmed))
            .unwrap();
    }
    // Takes the notification the class hands this device as process 7.
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    nf.set_process_identifier_filter(Some(7));
    for (process, recipient) in [
        (8, address_recipient(THIS_NETWORK, &PEER_A)),
        (9, address_recipient(THIS_NETWORK, &[])),
    ] {
        nf.add_destination(destination(recipient, process, false))
            .unwrap();
    }
    let mut db = ObjectDatabase::new();
    db.add(Box::new(class)).unwrap();
    db.add(Box::new(nf)).unwrap();

    let (broadcasts, unicasts, counters) = distribute_counted_through(
        started.server.test_network(),
        &started.sent,
        db,
        Arc::new(RwLock::new(bindings)),
        DccState::Enable,
    )
    .await;
    assert_eq!(
        routes(broadcasts, unicasts),
        [
            (1, To::Local(PEER_A.to_vec()), false),
            (2, To::LocalBroadcast, false),
            (3, To::LocalBroadcast, false),
            (4, To::Local(PEER_B.to_vec()), true),
            (5, To::Local(PEER_C.to_vec()), false),
            (6, To::Remote(REMOTE_NETWORK, REMOTE_MAC.to_vec()), false),
            (8, To::Local(PEER_A.to_vec()), false),
            (9, To::LocalBroadcast, false),
        ]
    );
    assert_eq!(counters, EventNotificationCounters::default());
    started.stop().await;
}

mod localize;
