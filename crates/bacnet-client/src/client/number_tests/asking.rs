//! The client asks for its network's number (#2572): once as it starts, with
//! a local broadcast What-Is-Network-Number, and again each time its link
//! comes back while nothing has taught it the number. Learning the number,
//! from the answer or from any later announcement, folds the discovery
//! table's routed rows on this network into local rows in the same locked
//! call, so a device a router relayed back is one device at once.
use super::local_routes::{
    PEER, REMOTE_NETWORK, ROUTER, THIS_NETWORK, i_am, relayed_from, send_to_client,
};
use super::*;
use crate::client::{BACnetClient, ClientConfig, DeviceEventKind};
use bacnet_encoding::apdu::{Apdu, SimpleAck};
use bacnet_npdu::NetworkMessageType;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::{mpsc, watch};
use tokio::time::Duration;

/// A started client over a link that reports its returns on `back`, and
/// takes the local Number controls when `numbers`.
struct Site {
    client: BACnetClient<TestTransport>,
    inbound: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    back: watch::Sender<u64>,
}

async fn open_site(numbers: bool) -> Site {
    let (inbound, receiver) = mpsc::channel(64);
    let (back, returns) = watch::channel(0);
    let mut transport = TestTransport::builder()
        .local_mac(&[1])
        .inbound(receiver)
        .link_returns(returns);
    if numbers {
        transport = transport.number_controls();
    }
    let transport = transport.build();
    let sent = transport.sent();
    let client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    Site {
        client,
        inbound,
        sent,
        back,
    }
}

impl Site {
    /// Have a node broadcast that this network is `network`, with `flag`,
    /// and wait until the client holds it.
    async fn announce(&self, network: u16, flag: u8) {
        inject(&self.inbound, &number(network, flag), true).await;
        bounded(async {
            while self.client.network_number() != Some(network) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await;
    }

    /// The link is back.
    fn link_returns(&self) {
        self.back.send_modify(|count| *count += 1);
    }

    /// How many What-Is-Network-Number frames the client has sent.
    fn asks(&self) -> usize {
        let frames = self.sent.frames();
        frames.iter().filter(|frame| frame.npdu == QUERY).count()
    }
}

/// `frame` is the client's question: a local broadcast of a bare
/// What-Is-Network-Number, with no DNET and no SNET (Clause 6.4.19).
fn assert_ask(frame: &SentFrame) {
    assert!(frame.broadcast, "a broadcast");
    assert!(frame.mac.is_empty(), "to this link's broadcast");
    let npdu = frame.decode_npdu();
    assert_eq!(
        npdu.destination, None,
        "never sent with a remote destination"
    );
    assert_eq!(npdu.source, None);
    assert_eq!(
        npdu.message_type,
        Some(NetworkMessageType::WHAT_IS_NETWORK_NUMBER)
    );
    assert!(npdu.payload.is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_client_asks_once_as_it_starts_and_not_on_a_link_without_number_controls() {
    let mut site = open_site(true).await;
    assert_eq!(site.sent.len(), 1, "sent before start returned");
    assert_ask(&site.sent.frame(0));
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(site.sent.len(), 1, "unanswered, it is not asked again");
    site.client.stop().await.unwrap();

    // A link that takes no Number controls, such as a router's application
    // port, is asked nothing, at start or when it comes back.
    let mut site = open_site(false).await;
    site.link_returns();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(site.sent.is_empty(), "{:?}", site.sent.frames());
    assert_eq!(site.client.network_number(), None);
    site.client.stop().await.unwrap();
}

/// The answer to the question teaches the number, and so does an
/// announcement nobody asked for, such as a router's at its own start: one
/// that says the number is configured takes the place of a learned one
/// (Clause 6.4.20). Neither is answered or asked again.
#[tokio::test(start_paused = true)]
async fn the_answer_teaches_the_number_and_a_later_announcement_still_does() {
    let mut site = open_site(true).await;
    site.announce(THIS_NETWORK, 0).await;
    assert_eq!(site.client.network_number(), Some(THIS_NETWORK));
    site.announce(THIS_NETWORK + 1, 1).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(site.sent.len(), 1, "{:?}", site.sent.frames());
    site.client.stop().await.unwrap();
}

/// Each return of the link while the number is unknown asks once more; once
/// the number is known, a return asks nothing: the client keeps the answer
/// rather than repeat the question (Clause 6.4.19).
#[tokio::test(start_paused = true)]
async fn a_link_return_asks_again_only_while_the_number_is_unknown() {
    let mut site = open_site(true).await;
    for asks in [2, 3] {
        site.link_returns();
        bounded(site.sent.wait_for_len(asks)).await;
        assert_ask(&site.sent.frame(asks - 1));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(site.sent.len(), asks, "one ask per return");
    }
    site.announce(THIS_NETWORK, 0).await;
    site.link_returns();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(site.asks(), 3, "a known number is not asked for");
    assert_eq!(site.sent.len(), 3, "{:?}", site.sent.frames());
    site.client.stop().await.unwrap();
}

/// Learning the number folds every routed row on this network in one
/// locked call: one whose station has a local row is taken out, one with
/// no local row becomes it, keeping what its I-Am said. Each is reported
/// `Lost`, the row it became `Discovered`, then one collision notice per
/// instance whose claimants changed. A row on another network stays.
#[tokio::test(start_paused = true)]
async fn learning_the_number_folds_the_routed_rows_on_this_network() {
    let mut site = open_site(true).await;
    let relayed = |network, station: &[u8]| relayed_from(network, station);
    let inbound = &site.inbound;
    // Device 9 at PEER, also heard relayed back; device 10 only relayed;
    // device 11 on another network; device 12 relayed from both networks.
    send_to_client(inbound, &i_am(9), &PEER, None).await;
    send_to_client(inbound, &i_am(9), &ROUTER, relayed(THIS_NETWORK, &PEER)).await;
    send_to_client(inbound, &i_am(10), &ROUTER, relayed(THIS_NETWORK, &[4])).await;
    send_to_client(inbound, &i_am(11), &ROUTER, relayed(REMOTE_NETWORK, &[4])).await;
    send_to_client(inbound, &i_am(12), &ROUTER, relayed(THIS_NETWORK, &[6])).await;
    send_to_client(inbound, &i_am(12), &ROUTER, relayed(REMOTE_NETWORK, &[6])).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(site.client.discovered_devices().await.len(), 6);

    let mut events = site.client.device_events();
    let mut collisions = site.client.device_collision_events();
    site.announce(THIS_NETWORK, 0).await;
    let devices = site.client.discovered_devices().await;
    assert_eq!(devices.len(), 5, "{devices:?}");

    let mut next = || events.try_recv().expect("a device notice");
    let mut expect = |kind, instance, network: Option<u16>, mac: &[u8]| {
        let event = next();
        assert_eq!(event.kind, kind, "{event:?}");
        let device = event.device;
        assert_eq!(device.object_identifier.instance_number(), instance);
        assert_eq!(device.source_network, network, "{device:?}");
        let station = device
            .source_address
            .as_ref()
            .unwrap_or(&device.mac_address);
        assert_eq!(station.as_slice(), mac);
    };
    expect(DeviceEventKind::Lost, 9, Some(THIS_NETWORK), &PEER);
    expect(DeviceEventKind::Lost, 10, Some(THIS_NETWORK), &[4]);
    expect(DeviceEventKind::Discovered, 10, None, &[4]);
    expect(DeviceEventKind::Lost, 12, Some(THIS_NETWORK), &[6]);
    expect(DeviceEventKind::Discovered, 12, None, &[6]);
    assert!(events.try_recv().is_err(), "nothing else changed");

    // Device 9's contest ended; device 12's goes on, with its local row.
    let ended = collisions.try_recv().unwrap();
    assert_eq!((ended.instance, ended.claimants.len()), (9, 1));
    let goes_on = collisions.try_recv().unwrap();
    assert_eq!(goes_on.instance, 12);
    let networks: Vec<_> = goes_on
        .claimants
        .iter()
        .map(|claimant| claimant.source_network)
        .collect();
    assert_eq!(networks, [None, Some(REMOTE_NETWORK)]);
    assert!(collisions.try_recv().is_err());

    let device = site.client.get_device(10).await.unwrap().unwrap();
    assert!(device.is_local());
    assert_eq!(device.mac_address.as_slice(), [4]);
    assert_eq!((device.max_apdu_length, device.vendor_id), (480, 260));
    assert!(device.last_announced.is_some(), "its I-Am's age kept");
    let device = site.client.get_device(11).await.unwrap().unwrap();
    assert_eq!(device.source_network, Some(REMOTE_NETWORK));
    assert!(site.client.get_device(9).await.unwrap().unwrap().is_local());
    assert!(matches!(
        site.client.get_device(12).await,
        Err(Error::AmbiguousDevice { instance: 12, .. })
    ));
    site.client.stop().await.unwrap();
}

/// The case #2572 is about, end to end: a router relays a device's I-Am
/// back onto its own network before the client knows the network's number,
/// and the device is ambiguous. The answer to the question the client
/// asked at start resolves it, with no second relay: the device is one
/// local row, and a request to it by instance goes to its MAC.
#[tokio::test(start_paused = true)]
async fn a_relayed_device_is_one_device_once_the_answer_comes() {
    let mut site = open_site(true).await;
    assert_ask(&site.sent.frame(0));
    send_to_client(&site.inbound, &i_am(9), &PEER, None).await;
    let echo = relayed_from(THIS_NETWORK, &PEER);
    send_to_client(&site.inbound, &i_am(9), &ROUTER, echo).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(matches!(
        site.client.get_device(9).await,
        Err(Error::AmbiguousDevice { instance: 9, .. })
    ));

    site.announce(THIS_NETWORK, 0).await;
    let device = site
        .client
        .get_device(9)
        .await
        .unwrap()
        .expect("one device");
    assert!(device.is_local());
    assert_eq!(device.mac_address.as_slice(), PEER);

    let write = site.client.write_property_to_device(
        9,
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
        vec![0x44, 0x42, 0xA0, 0x00, 0x00],
        None,
    );
    let peer = async {
        site.sent.wait_for_len(2).await;
        let frame = site.sent.frame(1);
        assert_eq!(frame.mac.as_slice(), PEER, "to the device's own MAC");
        assert_eq!(frame.decode_npdu().destination, None, "with no DNET");
        let Apdu::ConfirmedRequest(request) = frame.apdu() else {
            panic!("a confirmed request");
        };
        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
        });
        send_to_client(&site.inbound, &ack, &PEER, None).await;
    };
    let (result, ()) = bounded(async { tokio::join!(write, peer) }).await;
    result.unwrap();
    site.client.stop().await.unwrap();
}
