//! A router replaced on a running server (#2831, decision of 2026-11-26):
//! end to end through the server's own tasks, its Number worker handing
//! the router announcements to its router learning, and a scripted link
//! with two routers to network 1000.
//!
//! The first notification goes by local broadcast while no router is known,
//! and the server's router query learns ROUTER_A. Then ROUTER_A goes away and
//! ROUTER_B carries the network's traffic: each notification's first
//! attempt through ROUTER_A goes unanswered, its broadcast retry is carried
//! by ROUTER_B and acknowledged, and two checks a minute apart move the
//! router to ROUTER_B. A forged answer flood and a forged announcement flood
//! from a third station change nothing.

use super::*;
use crate::server::clocked_test_database;
use crate::server::event_notifications_tests::local_broadcast_destination;
use crate::server::test_transport::{BIP_LOCAL_MAC, SendLog, SentFrame, TestTransport};
use bacnet_encoding::apdu::encode_apdu;
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_npdu::NetworkMessageType;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::{EngineeringUnits, Reliability};
use std::sync::Mutex as StdMutex;
use tokio::sync::mpsc;

const DNET: u16 = 1000;
const RECIPIENT: [u8; 6] = [0x0A; 6];
const ROUTER_A: [u8; 6] = [10, 0, 0, 1, 0xBA, 0xC0];
const ROUTER_B: [u8; 6] = [10, 0, 0, 2, 0xBA, 0xC1];
const FORGER: [u8; 6] = [10, 0, 0, 66, 0xBA, 0xC0];
/// The server's notification APDU timeout, the default: each step of a
/// router check waits this long too.
const TIMEOUT: Duration = Duration::from_secs(3);

/// What the server sent on the link.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sent {
    /// A confirmed EventNotification for [`RECIPIENT`]: through a router,
    /// or with the link's broadcast address.
    Notification {
        via: Option<Vec<u8>>,
        invoke_id: u8,
    },
    /// The router query for [`DNET`].
    RouterQuery,
    /// A Who-Is through a router to [`RECIPIENT`].
    Probe {
        via: Vec<u8>,
    },
    Other,
}

fn classify(frame: &SentFrame) -> Sent {
    let npdu = frame.decode_npdu();
    if npdu.message_type == Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK) {
        return if frame.broadcast && npdu.payload.as_ref() == DNET.to_be_bytes() {
            Sent::RouterQuery
        } else {
            Sent::Other
        };
    }
    let Some(destination) = npdu.destination.filter(|d| d.network == DNET) else {
        return Sent::Other;
    };
    let via = (!frame.broadcast).then(|| frame.mac.to_vec());
    match frame.apdu() {
        Apdu::ConfirmedRequest(request)
            if request.service_choice == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION =>
        {
            Sent::Notification {
                via,
                invoke_id: request.invoke_id,
            }
        }
        Apdu::UnconfirmedRequest(request)
            if request.service_choice == UnconfirmedServiceChoice::WHO_IS
                && destination.mac_address.as_slice() == RECIPIENT =>
        {
            via.map_or(Sent::Other, |via| Sent::Probe { via })
        }
        _ => Sent::Other,
    }
}

fn npdu_bytes(
    source: Option<(u16, &[u8])>,
    apdu: Option<Apdu>,
    message: Option<(NetworkMessageType, Vec<u8>)>,
) -> Bytes {
    let mut payload = BytesMut::new();
    if let Some(apdu) = &apdu {
        encode_apdu(&mut payload, apdu).unwrap();
    }
    let (message_type, payload) = match message {
        Some((kind, body)) => (Some(kind), Bytes::from(body)),
        None => (None, payload.freeze()),
    };
    let mut buf = BytesMut::new();
    encode_npdu(
        &mut buf,
        &Npdu {
            source: source.map(|(network, mac)| NpduAddress {
                network,
                mac_address: MacAddr::from_slice(mac),
            }),
            message_type,
            payload,
            ..Npdu::default()
        },
    )
    .unwrap();
    buf.freeze()
}

fn received(from: &[u8], npdu: Bytes, broadcast: bool) -> ReceivedNpdu {
    ReceivedNpdu {
        direct_response: None,
        npdu,
        source_mac: MacAddr::from_slice(from),
        link_layer_group: broadcast,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// The recipient's acknowledgement of `invoke_id`, as a router carries it.
fn ack(invoke_id: u8) -> Bytes {
    npdu_bytes(
        Some((DNET, &RECIPIENT)),
        Some(Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        })),
        None,
    )
}

fn announcement() -> Bytes {
    npdu_bytes(
        None,
        None,
        Some((
            NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
            DNET.to_be_bytes().to_vec(),
        )),
    )
}

fn i_am() -> Bytes {
    npdu_bytes(
        Some((DNET, &RECIPIENT)),
        Some(Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: Bytes::new(),
        })),
        None,
    )
}

/// The routers to [`DNET`]: the one that carries its traffic now. It
/// answers the router query, forwards the notifications sent to it or by
/// broadcast, and carries the recipient's acknowledgement and I-Am back.
struct Wire {
    alive: StdMutex<[u8; 6]>,
    acked: StdMutex<Vec<u8>>,
}

impl Wire {
    fn replies(&self, frame: &SentFrame) -> Vec<(Vec<u8>, Bytes, bool)> {
        let alive = *self.alive.lock().unwrap();
        match classify(frame) {
            Sent::RouterQuery => vec![(alive.to_vec(), announcement(), true)],
            Sent::Notification { via, invoke_id }
                if via.as_deref().is_none_or(|via| via == alive) =>
            {
                self.acked.lock().unwrap().push(invoke_id);
                vec![(alive.to_vec(), ack(invoke_id), false)]
            }
            Sent::Probe { via } if via == alive => vec![(alive.to_vec(), i_am(), true)],
            _ => Vec::new(),
        }
    }
}

struct Site {
    server: BACnetServer<TestTransport>,
    tx: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    wire: Arc<Wire>,
    reliability: bool,
}

impl Site {
    async fn start() -> Self {
        let mut db = clocked_test_database();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let mut nc = NotificationClass::new(0, "NC-0").unwrap();
        nc.priority = [255, 255, 255];
        nc.add_destination(bacnet_types::constructed::BACnetDestination {
            recipient: BACnetRecipient::Address(BACnetAddress {
                network_number: DNET,
                mac_address: MacAddr::from_slice(&RECIPIENT),
            }),
            issue_confirmed_notifications: true,
            ..local_broadcast_destination()
        })
        .unwrap();
        db.add(Box::new(nc)).unwrap();
        let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        ai.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        ai.write_property(
            PropertyIdentifier::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xE0],
            },
            None,
        )
        .unwrap();
        db.add(Box::new(ai)).unwrap();
        let wire = Arc::new(Wire {
            alive: StdMutex::new(ROUTER_A),
            acked: StdMutex::default(),
        });
        let (tx, rx) = mpsc::channel(1024);
        let (hook_wire, hook_tx) = (Arc::clone(&wire), tx.clone());
        let transport = TestTransport::builder()
            .local_mac(&BIP_LOCAL_MAC)
            .number_controls()
            .inbound(rx)
            .on_send(move |frame| {
                for (from, npdu, broadcast) in hook_wire.replies(&frame) {
                    hook_tx.try_send(received(&from, npdu, broadcast)).unwrap();
                }
                async { Ok(()) }
            })
            .build();
        let sent = transport.sent();
        let server = BACnetServer::start(ServerConfig::default(), db, transport)
            .await
            .unwrap();
        Self {
            server,
            tx,
            sent,
            wire,
            reliability: false,
        }
    }

    /// One transition of AI-1, to FAULT or back, and its notification.
    async fn transition(&mut self) {
        self.reliability = !self.reliability;
        let reliability = if self.reliability {
            Reliability::OVER_RANGE
        } else {
            Reliability::NO_FAULT_DETECTED
        };
        self.server
            .write_local(
                &ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(reliability.to_raw()),
                None,
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent
            .frames()
            .iter()
            .map(classify)
            .filter(|sent| *sent != Sent::Other)
            .collect()
    }

    fn count(&self, wanted: impl Fn(&Sent) -> bool) -> usize {
        self.sent().iter().filter(|sent| wanted(sent)).count()
    }

    fn router(&self) -> Option<MacAddr> {
        self.server.learned_routers.router(DNET)
    }

    async fn hear(&self, from: &[u8], npdu: Bytes, broadcast: bool) {
        self.tx.send(received(from, npdu, broadcast)).await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn a_replaced_router_is_taken_after_two_checks_with_broadcast_delivery_meanwhile() {
    let mut site = Site::start().await;
    // No router known: the notification goes by broadcast, and the query
    // learns ROUTER_A, the one router answering it.
    site.transition().await;
    assert_eq!(
        site.count(|sent| matches!(sent, Sent::Notification { via: None, .. })),
        1
    );
    assert_eq!(site.count(|sent| *sent == Sent::RouterQuery), 1);
    assert_eq!(site.wire.acked.lock().unwrap().len(), 1);
    assert_eq!(
        site.router(),
        None,
        "decided at the end of the query's wait"
    );
    tokio::time::sleep(TIMEOUT + Duration::from_millis(100)).await;
    assert_eq!(site.router().as_deref(), Some(&ROUTER_A[..]));
    site.transition().await;
    assert!(matches!(
        site.sent().last(),
        Some(Sent::Notification { via: Some(via), .. }) if via[..] == ROUTER_A
    ));

    // ROUTER_B takes over. Each notification is still delivered: its
    // broadcast retry reaches the recipient through ROUTER_B.
    *site.wire.alive.lock().unwrap() = ROUTER_B;
    let before = site.wire.acked.lock().unwrap().len();
    site.transition().await;
    tokio::time::sleep(TIMEOUT + Duration::from_millis(10)).await;
    assert_eq!(
        site.wire.acked.lock().unwrap().len(),
        before + 1,
        "delivered by the broadcast retry"
    );
    assert_eq!(site.server.notification_transactions.active_count(), 0);
    // The check, then the second one a minute after the first strike.
    tokio::time::sleep(2 * TIMEOUT).await;
    assert_eq!(
        site.router().as_deref(),
        Some(&ROUTER_A[..]),
        "moved on the first strike"
    );
    assert_eq!(
        (
            site.count(|sent| matches!(sent, Sent::Probe { .. })),
            site.count(|sent| *sent == Sent::RouterQuery)
        ),
        (1, 2)
    );
    tokio::time::sleep(Duration::from_secs(60) + 2 * TIMEOUT).await;
    assert_eq!(site.router().as_deref(), Some(&ROUTER_B[..]));
    // About a minute and a quarter after the first silent attempt began.
    site.transition().await;
    assert!(matches!(
        site.sent().last(),
        Some(Sent::Notification { via: Some(via), .. }) if via[..] == ROUTER_B
    ));
    site.server.stop().await.unwrap();
}

/// A station on the link floods answers for the recipient under every
/// invoke ID, and announcements for its network, an hour of one each a
/// second: no notification completes from it, it never becomes the
/// router, and the server sends nothing in reply but its own pacing allows.
#[tokio::test(start_paused = true)]
async fn forged_answer_and_announcement_floods_change_nothing() {
    let mut site = Site::start().await;
    site.transition().await;
    tokio::time::sleep(TIMEOUT + Duration::from_millis(100)).await;
    assert_eq!(site.router().as_deref(), Some(&ROUTER_A[..]));
    let frames_before = site.sent.len();
    for second in 0..3_600u32 {
        let invoke_id = (second % 256) as u8;
        site.hear(&FORGER, ack(invoke_id), false).await;
        site.hear(&FORGER, announcement(), true).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert_eq!(site.router().as_deref(), Some(&ROUTER_A[..]));
    assert_eq!(site.sent.len(), frames_before, "{:?}", site.sent());
    assert!(
        site.server.off_path_answer_drops() == 0,
        "no request was open"
    );
    // With a notification open through ROUTER_A, the forger's answers are
    // refused and counted, and ROUTER_A's completes it.
    site.wire.alive.lock().unwrap().copy_from_slice(&[0; 6]);
    site.transition().await;
    let Some(Sent::Notification { invoke_id, .. }) = site.sent().last().cloned() else {
        panic!("{:?}", site.sent());
    };
    for _ in 0..100 {
        site.hear(&FORGER, ack(invoke_id), false).await;
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(site.server.off_path_answer_drops(), 100);
    site.hear(&ROUTER_A, ack(invoke_id), false).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(site.server.notification_transactions.active_count(), 0);
    assert_eq!(site.router().as_deref(), Some(&ROUTER_A[..]));
    site.server.stop().await.unwrap();
}
