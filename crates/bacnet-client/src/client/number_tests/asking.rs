//! When the client asks for its network's number (#2572): never at start,
//! and once on first need, before its first Who-Is or when it first stores
//! a routed row while the number is unknown. A client that only names
//! devices by address never asks. After that first question, each return
//! of the link while the number is still unknown asks once more; a question
//! that fails to send holds nothing back and doesn't count.
use super::local_routes::{
    PEER, REMOTE_NETWORK, ROUTER, THIS_NETWORK, i_am, relayed_from, send_to_client,
};
use super::*;
use crate::client::{BACnetClient, ClientConfig};
use crate::discovery::RoutedDeviceConfig;
use bacnet_encoding::apdu::{Apdu, ComplexAck, SimpleAck};
use bacnet_npdu::NetworkMessageType;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, SentFrame, TestTransport, TestTransportHandle};
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, PropertyIdentifier, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use tokio::sync::{mpsc, watch};
use tokio::time::Duration;

/// A started client over a link that reports its returns on `back`, and
/// takes the local Number controls when `numbers`.
pub(super) struct Site {
    pub(super) client: BACnetClient<TestTransport>,
    pub(super) inbound: mpsc::Sender<ReceivedNpdu>,
    pub(super) sent: SendLog,
    handle: TestTransportHandle,
    back: watch::Sender<u64>,
}

pub(super) async fn open_site(numbers: bool) -> Site {
    let (inbound, receiver) = mpsc::channel(64);
    let (back, returns) = watch::channel(0);
    let mut transport = TestTransport::builder()
        .local_mac(&[1])
        .inbound(receiver)
        .link_returns(returns);
    if numbers {
        transport = transport.number_controls();
    }
    let handle = transport.handle();
    let transport = transport.build();
    let sent = transport.sent();
    let client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    Site {
        client,
        inbound,
        sent,
        handle,
        back,
    }
}

impl Site {
    /// Have a node broadcast that this network is `network`, with `flag`,
    /// and wait until the client holds it.
    pub(super) async fn announce(&self, network: u16, flag: u8) {
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

    /// Give the client's tasks a turn, then the frames it has sent.
    async fn settled(&self) -> Vec<SentFrame> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        self.sent.frames()
    }

    /// Register Device `instance` as routed through `ROUTER` at `mac` on
    /// `network`.
    async fn add_routed(&self, instance: u32, network: u16, mac: &[u8]) {
        let config = RoutedDeviceConfig {
            instance,
            router_mac: ROUTER.to_vec(),
            remote_network: network,
            remote_mac: mac.to_vec(),
            max_apdu_length: 480,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        };
        self.client.add_routed_device(config).await.unwrap();
    }
}

/// `frame` is the client's question: a local broadcast of a bare
/// What-Is-Network-Number, with no DNET and no SNET (Clause 6.4.19).
pub(super) fn assert_ask(frame: &SentFrame) {
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

/// `frame` carries a Who-Is.
fn assert_who_is(frame: &SentFrame) {
    let Apdu::UnconfirmedRequest(request) = frame.apdu() else {
        panic!("an unconfirmed request: {frame:?}");
    };
    assert_eq!(request.service_choice, UnconfirmedServiceChoice::WHO_IS);
}

/// Nothing goes out at start, nor for requests that name a device by
/// address, nor when the link comes back before anything was asked: a
/// client that only reads and writes by address never asks.
#[tokio::test(start_paused = true)]
async fn nothing_is_asked_at_start_or_for_requests_by_address() {
    let mut site = open_site(true).await;
    assert!(site.settled().await.is_empty(), "no frame at start");
    site.link_returns();

    let station = BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&PEER),
    };
    let object = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    let write = site.client.write_property(
        &PEER,
        object,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        vec![0x44, 0x42, 0xA0, 0x00, 0x00],
        None,
    );
    let read = site.client.confirmed_request_to(
        &station,
        ConfirmedServiceChoice::READ_PROPERTY,
        &[0x0C, 0x00, 0x80, 0x00, 0x01, 0x19, 0x55],
    );
    let peer = async {
        for index in 0..2 {
            site.sent.wait_for_len(index + 1).await;
            let frame = site.sent.frame(index);
            assert_eq!(frame.mac.as_slice(), PEER, "frame {index} to the device");
            let Apdu::ConfirmedRequest(request) = frame.apdu() else {
                panic!("frame {index} is a confirmed request, not a question");
            };
            let answer = if request.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY {
                Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice,
                })
            } else {
                Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id: request.invoke_id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: request.service_choice,
                    service_ack: Bytes::from_static(&[0x0C]),
                })
            };
            send_to_client(&site.inbound, &answer, &PEER, None).await;
        }
    };
    let (written, read, ()) = bounded(async { tokio::join!(write, read, peer) }).await;
    written.unwrap();
    read.unwrap();
    assert_eq!(
        site.settled().await.len(),
        2,
        "two requests and nothing else"
    );
    site.client.stop().await.unwrap();
}

/// The first Who-Is, of any scope, goes after one question; no later one
/// does. A link that takes no Number controls is never asked.
#[tokio::test(start_paused = true)]
async fn the_first_who_is_goes_after_one_question_and_no_other_does() {
    let mut site = open_site(true).await;
    site.client.who_is(None).await.unwrap();
    let frames = site.sent.frames();
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_ask(&frames[0]);
    assert_who_is(&frames[1]);
    site.client.who_is(None).await.unwrap();
    site.client
        .who_is_network(REMOTE_NETWORK, None)
        .await
        .unwrap();
    site.client.who_is_directed(&PEER, None).await.unwrap();
    let frames = site.settled().await;
    assert_eq!(frames.len(), 5, "{frames:?}");
    frames[1..].iter().for_each(assert_who_is);
    site.client.stop().await.unwrap();

    let mut site = open_site(false).await;
    site.client.who_is(None).await.unwrap();
    let frames = site.settled().await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_who_is(&frames[0]);
    site.client.stop().await.unwrap();
}

/// A local I-Am asks nothing; the first routed row the client stores while
/// the number is unknown asks once, through the Number owner, and a later
/// routed row or Who-Is asks nothing more.
#[tokio::test(start_paused = true)]
async fn the_first_routed_i_am_asks_once() {
    let mut site = open_site(true).await;
    send_to_client(&site.inbound, &i_am(8), &PEER, None).await;
    assert!(
        site.settled().await.is_empty(),
        "a local device asks nothing"
    );
    for (instance, station) in [(10, [4]), (11, [5])] {
        let routed = relayed_from(REMOTE_NETWORK, &station);
        send_to_client(&site.inbound, &i_am(instance), &ROUTER, routed).await;
    }
    let frames = site.settled().await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_ask(&frames[0]);
    site.client.who_is(None).await.unwrap();
    let frames = site.settled().await;
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_who_is(&frames[1]);
    site.client.stop().await.unwrap();
}

/// `add_routed_device` asks too, once, while the number is unknown.
#[tokio::test(start_paused = true)]
async fn the_first_routed_registration_asks_once() {
    let mut site = open_site(true).await;
    site.add_routed(10, REMOTE_NETWORK, &[4]).await;
    let frames = site.sent.frames();
    assert_eq!(frames.len(), 1, "asked before the call returned");
    assert_ask(&frames[0]);
    site.add_routed(11, REMOTE_NETWORK, &[5]).await;
    site.client.who_is(None).await.unwrap();
    let frames = site.settled().await;
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_who_is(&frames[1]);
    site.client.stop().await.unwrap();
}

/// A question that fails to send holds the Who-Is back no more than one
/// that goes, and isn't counted: the next Who-Is asks again.
#[tokio::test(start_paused = true)]
async fn a_failed_question_holds_no_who_is_back_and_is_asked_again() {
    let mut site = open_site(true).await;
    site.handle.fail_next_send();
    site.client.who_is(None).await.unwrap();
    site.client.who_is(None).await.unwrap();
    site.client.who_is(None).await.unwrap();
    let frames = site.settled().await;
    assert_eq!(frames.len(), 5, "{frames:?}");
    assert_ask(&frames[0]);
    assert_who_is(&frames[1]);
    assert_ask(&frames[2]);
    assert_who_is(&frames[3]);
    assert_who_is(&frames[4]);
    site.client.stop().await.unwrap();
}

/// A return of the link asks again only once the client has asked, and
/// only while the number is still unknown: one question per return.
#[tokio::test(start_paused = true)]
async fn a_link_return_asks_again_after_a_first_question_while_unknown() {
    let mut site = open_site(true).await;
    site.link_returns();
    assert!(site.settled().await.is_empty(), "nothing asked yet");
    site.client.who_is(None).await.unwrap();
    for asks in [3, 4] {
        site.link_returns();
        let frames = site.settled().await;
        assert_eq!(frames.len(), asks, "{frames:?}");
        assert_ask(&frames[asks - 1]);
    }
    site.announce(THIS_NETWORK, 0).await;
    site.link_returns();
    let frames = site.settled().await;
    assert_eq!(frames.len(), 4, "a known number is not asked for");
    site.client.stop().await.unwrap();
}

/// The case #2572 is about, end to end: a router relays a device's I-Am
/// back onto its own network before the client knows the network's number,
/// and the device is ambiguous. The routed row asks; the answer resolves
/// it, with no second relay: the device is one local row, and a request to
/// it by instance goes to its MAC.
#[tokio::test(start_paused = true)]
async fn a_relayed_device_is_one_device_once_the_answer_comes() {
    let mut site = open_site(true).await;
    send_to_client(&site.inbound, &i_am(9), &PEER, None).await;
    let echo = relayed_from(THIS_NETWORK, &PEER);
    send_to_client(&site.inbound, &i_am(9), &ROUTER, echo).await;
    bounded(site.sent.wait_for_len(1)).await;
    assert_ask(&site.sent.frame(0));
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
