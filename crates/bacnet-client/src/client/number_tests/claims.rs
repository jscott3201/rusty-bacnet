//! A Network-Number-Is the client did not ask for (#2826).
//!
//! Any station on the link can broadcast one, flagged configured, and the
//! client used to take it as its own network's number at once: every row it
//! held as routed on that network folded into a local row, requests to those
//! devices went to link addresses where they are not, and an answer from a
//! station holding such an address completed them. These tests hold the
//! client to the rule that closes that: an announcement that would change
//! the number is a cue to ask, and the number comes from the answers to the
//! client's own question when they agree.
use super::asking::{Site, assert_ask, open_site};
use super::local_routes::{
    REMOTE_NETWORK, ROUTER, THIS_NETWORK, i_am, relayed_from, send_to_client,
};
use super::*;
use crate::client::Station;
use bacnet_encoding::apdu::{Apdu, SimpleAck};
use bacnet_npdu::NetworkMessageType;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use tokio::time::Duration;

/// Device 10's address on the remote network, behind `ROUTER`.
const FAR: [u8; 1] = [4];

/// Past the window the client gives answers to its question: the default
/// APDU timeout, with room to spare.
const PAST_THE_WINDOW: Duration = Duration::from_secs(10);

/// A client that holds Device 10 as routed through `ROUTER` at `FAR` on
/// `REMOTE_NETWORK`, its question about the number gone unanswered.
async fn holding_a_routed_device() -> Site {
    let site = open_site(true).await;
    let routed = relayed_from(REMOTE_NETWORK, &FAR);
    send_to_client(&site.inbound, &i_am(10), &ROUTER, routed).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), None, "nobody answered");
    let frames = site.sent.take();
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_ask(&frames[0]);
    site
}

/// How many What-Is-Network-Number questions the client has sent since the
/// log was last taken.
fn questions(site: &Site) -> usize {
    site.sent
        .frames()
        .iter()
        .filter(|frame| {
            frame.decode_npdu().message_type == Some(NetworkMessageType::WHAT_IS_NETWORK_NUMBER)
        })
        .count()
}

/// Device 10 is still the routed row it was.
async fn still_routed(site: &Site) {
    let device = site
        .client
        .get_device(10)
        .await
        .unwrap()
        .expect("Device 10");
    assert_eq!(device.source_network, Some(REMOTE_NETWORK), "{device:?}");
    assert_eq!(device.mac_address.as_slice(), ROUTER, "{device:?}");
}

/// A station's unasked Network-Number-Is naming the remote network folds
/// no routed row, and the client does not take the number.
#[tokio::test(start_paused = true)]
async fn an_unasked_number_folds_no_routed_row() {
    let mut site = holding_a_routed_device().await;
    inject(&site.inbound, &number(REMOTE_NETWORK, 1), true).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), None);
    still_routed(&site).await;
    site.client.stop().await.unwrap();
}

/// After the same announcement a request to Device 10 still goes through
/// its router with the DNET. A station sending from `FAR`'s address on
/// this link cannot answer it; the router's relayed answer completes it.
#[tokio::test(start_paused = true)]
async fn requests_to_a_routed_device_still_go_through_its_router() {
    let mut site = holding_a_routed_device().await;
    inject(&site.inbound, &number(REMOTE_NETWORK, 1), true).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    site.sent.take();
    let to = site.client.resolve_device(10).await.unwrap();
    let write = site.client.write_property(
        &to,
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
        vec![0x44, 0x42, 0xA0, 0x00, 0x00],
        None,
    );
    let router = async {
        site.sent.wait_for_len(1).await;
        let frame = site.sent.frame(0);
        assert_eq!(frame.mac.as_slice(), ROUTER, "through the router");
        let npdu = frame.decode_npdu();
        let to = npdu.destination.expect("with the DNET");
        assert_eq!(
            (to.network, to.mac_address.as_slice()),
            (REMOTE_NETWORK, &FAR[..])
        );
        let Apdu::ConfirmedRequest(request) = frame.apdu() else {
            panic!("a confirmed request");
        };
        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
        });
        send_to_client(&site.inbound, &ack, &FAR, None).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        send_to_client(
            &site.inbound,
            &ack,
            &ROUTER,
            relayed_from(REMOTE_NETWORK, &FAR),
        )
        .await;
    };
    let started = tokio::time::Instant::now();
    let (result, ()) = bounded(async { tokio::join!(write, router) }).await;
    result.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the router's answer"
    );
    assert_eq!(site.client.off_path_answer_drops(), 0);
    site.client.stop().await.unwrap();
}

/// The client's question draws the router's answer with this network's
/// number and a stranger's with another, both flagged configured: the
/// window decides nothing, the client keeps what it held, and counts it.
/// A known number stays known; an unknown one stays unknown.
#[tokio::test(start_paused = true)]
async fn answers_that_disagree_decide_nothing_and_are_counted() {
    for known in [false, true] {
        let mut site = holding_a_routed_device().await;
        if known {
            taught(&site).await;
        }
        // The stranger's announcement is a cue; its answer, and the
        // router's, come in the window the client's question opens.
        inject(&site.inbound, &number(REMOTE_NETWORK, 1), true).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(questions(&site), 1 + usize::from(known), "asked about it");
        broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
        inject(&site.inbound, &number(REMOTE_NETWORK, 1), true).await;
        tokio::time::sleep(PAST_THE_WINDOW).await;
        let held = known.then_some(THIS_NETWORK);
        assert_eq!(site.client.network_number(), held, "known {known}");
        assert_eq!(site.client.network_number_disagreements(), 1);
        still_routed(&site).await;
        site.client.stop().await.unwrap();
    }
}

/// A site renumbered: the router announces its new number, flagged
/// configured. The client asks; the router answers with the new number,
/// and another node still holding the old one, as learned, answers with
/// that. A configured answer outranks a learned one (Clause 6.4.20), so
/// the client takes the new number.
#[tokio::test(start_paused = true)]
async fn a_configured_answer_outranks_a_learned_one() {
    const NEW: u16 = THIS_NETWORK + 1;
    let mut site = open_site(true).await;
    taught(&site).await;
    deliver_announcement(&site, NEW, 1, &ROUTER).await;
    assert_eq!(questions(&site), 2, "asked about the new number");
    deliver_announcement(&site, NEW, 1, &ROUTER).await;
    deliver_announcement(&site, THIS_NETWORK, 0, &[5]).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), Some(NEW));
    assert_eq!(site.client.network_number_disagreements(), 0);
    site.client.stop().await.unwrap();
}

/// Broadcast a Network-Number-Is naming `network` with `flag` from
/// `source`, and give the client a turn.
async fn deliver_announcement(site: &Site, network: u16, flag: u8, source: &[u8]) {
    broadcast_from(&site.inbound, &number(network, flag), source).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
}

/// A stranger repeating its announcement, and the router repeating the
/// real number, draw one question at once, then at most one per hold: a
/// minute, then two. Each question's window holds both numbers, so none
/// decides.
#[tokio::test(start_paused = true)]
async fn repeated_announcements_ask_once_per_hold() {
    let mut site = holding_a_routed_device().await;
    // Each a second for three and a half minutes: questions at 0, 60 and
    // 180 seconds.
    for _ in 0..210 {
        inject(&site.inbound, &number(REMOTE_NETWORK, 1), true).await;
        broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert_eq!(questions(&site), 3, "{:?}", site.sent.frames());
    assert_eq!(site.client.network_number(), None);
    assert_eq!(site.client.network_number_disagreements(), 3);
    still_routed(&site).await;
    site.client.stop().await.unwrap();
}

/// What is left (#2826): on a link where no station that knows the number
/// answers the question (no router, or one older than the message), a
/// station that answers it alone still decides the number. Here the
/// stranger's announcement draws the question and its next one answers it.
#[tokio::test(start_paused = true)]
async fn a_station_answering_the_question_alone_still_decides() {
    let mut site = holding_a_routed_device().await;
    for _ in 0..2 {
        inject(&site.inbound, &number(REMOTE_NETWORK, 1), true).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), Some(REMOTE_NETWORK));
    let device = site
        .client
        .get_device(10)
        .await
        .unwrap()
        .expect("Device 10");
    assert!(device.is_local(), "folded: {device:?}");
    site.client.stop().await.unwrap();
}

/// Teach the client `THIS_NETWORK`: it asks, `ROUTER` answers, flagged
/// configured, and the window closes.
async fn taught(site: &Site) {
    site.client.what_is_network_number(None).await.unwrap();
    let answer = number(THIS_NETWORK, 1);
    broadcast_from(&site.inbound, &answer, &ROUTER).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), Some(THIS_NETWORK));
}

/// An unasked number never replaces a learned one, a remote network's
/// included: the client keeps its number and Device 10 stays routed.
#[tokio::test(start_paused = true)]
async fn an_unasked_number_does_not_replace_a_learned_one() {
    for forged in [THIS_NETWORK + 1, REMOTE_NETWORK] {
        let mut site = holding_a_routed_device().await;
        taught(&site).await;
        inject(&site.inbound, &number(forged, 1), true).await;
        tokio::time::sleep(PAST_THE_WINDOW).await;
        assert_eq!(site.client.network_number(), Some(THIS_NETWORK), "{forged}");
        still_routed(&site).await;
        let to = site.client.resolve_device(10).await.unwrap();
        assert_eq!(to, Station::routed(&ROUTER, REMOTE_NETWORK, &FAR));
        site.client.stop().await.unwrap();
    }
}

/// A router that announces this network's number at its start, unasked:
/// the client asks once, at once, and takes the router's answer when the
/// window closes.
#[tokio::test(start_paused = true)]
async fn a_routers_announcement_is_asked_about_and_taken() {
    let mut site = open_site(true).await;
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(site.client.network_number(), None, "not taken unasked");
    assert_eq!(questions(&site), 1, "{:?}", site.sent.frames());
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), Some(THIS_NETWORK));
    assert_eq!(questions(&site), 1, "asked once");
    site.client.stop().await.unwrap();
}
