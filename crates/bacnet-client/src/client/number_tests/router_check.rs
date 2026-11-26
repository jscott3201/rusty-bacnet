//! What a window decides (maintainer decisions on #2826): any two numbers
//! decide nothing, whatever their flags; and a number the client reaches
//! through a router is taken only once no router on this link answers that
//! it routes to it (Clause 6.6.3.2).
use super::asking::{Site, open_site};
use super::local_routes::{
    PEER, REMOTE_NETWORK, ROUTER, THIS_NETWORK, i_am, relayed_from, send_to_client,
};
use super::*;
use bacnet_npdu::NetworkMessageType;
use bacnet_transport::test_transport::SentFrame;
use bacnet_types::error::Error;
use tokio::time::Duration;

/// Device 10's address on the remote network, behind `ROUTER`.
const FAR: [u8; 1] = [4];
/// A station that is not a router.
const NODE: [u8; 1] = [5];
/// A station that claims numbers it should not.
const STRANGER: [u8; 1] = [66];

/// Past a window: the longest a node may hold its answer and the margin.
const PAST_THE_WINDOW: Duration = Duration::from_secs(13);
/// Past a window and the router check after it (the default APDU timeout).
const PAST_THE_CHECK: Duration = Duration::from_secs(20);

/// Whether `frame` asks the routers on the link about `network`.
fn router_query_for(frame: &SentFrame, network: u16) -> bool {
    let npdu = frame.decode_npdu();
    npdu.message_type == Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
        && npdu.payload.as_ref() == network.to_be_bytes()
}

/// The router queries for `network` the client has sent.
fn router_queries(site: &Site, network: u16) -> usize {
    let frames = site.sent.frames();
    frames
        .iter()
        .filter(|f| router_query_for(f, network))
        .count()
}

/// `ROUTER` says, on this link, that it routes to `network`.
async fn router_routes_to(site: &Site, network: u16) {
    let [high, low] = network.to_be_bytes();
    // NPDU version 1, a network message, I-Am-Router-To-Network (0x01) and
    // the one network.
    let npdu = [1, 0x80, 0x01, high, low];
    broadcast_from(&site.inbound, &npdu, &ROUTER).await;
}

/// Answers that name two numbers decide nothing, whatever their flags:
/// stricter than Clause 6.4.20, which would let a configured one win. The
/// client keeps what it held and counts the window.
#[tokio::test(start_paused = true)]
async fn a_configured_and_a_learned_answer_that_differ_decide_nothing() {
    let mut site = open_site(true).await;
    site.client.what_is_network_number(None).await.unwrap();
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    broadcast_from(&site.inbound, &number(THIS_NETWORK + 1, 0), &NODE).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), None);
    assert_eq!(site.client.network_number_disagreements(), 1);
    site.client.stop().await.unwrap();
}

/// One station that answers twice with two numbers disagrees with itself.
#[tokio::test(start_paused = true)]
async fn one_station_answering_two_numbers_decides_nothing() {
    let mut site = open_site(true).await;
    site.client.what_is_network_number(None).await.unwrap();
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &STRANGER).await;
    broadcast_from(&site.inbound, &number(THIS_NETWORK + 1, 0), &STRANGER).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), None);
    assert_eq!(site.client.network_number_disagreements(), 1);
    site.client.stop().await.unwrap();
}

/// Answers that agree on the number, one of them flagged configured, are
/// taken as configured: a later window of learned answers alone, naming
/// another number, can't replace it (Clause 6.4.20).
#[tokio::test(start_paused = true)]
async fn one_number_with_mixed_flags_is_taken_as_configured() {
    let mut site = open_site(true).await;
    site.client.what_is_network_number(None).await.unwrap();
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 0), &NODE).await;
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), Some(THIS_NETWORK));
    site.client.what_is_network_number(None).await.unwrap();
    broadcast_from(&site.inbound, &number(THIS_NETWORK + 1, 0), &NODE).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), Some(THIS_NETWORK), "kept");
    assert_eq!(site.client.network_number_disagreements(), 0);
    site.client.stop().await.unwrap();
}

/// A client that holds Device 10 as routed through `ROUTER` at `FAR` on
/// `REMOTE_NETWORK`, its first question gone unanswered and taken from
/// the log.
async fn holding_a_routed_device() -> Site {
    let site = open_site(true).await;
    let routed = relayed_from(REMOTE_NETWORK, &FAR);
    send_to_client(&site.inbound, &i_am(10), &ROUTER, routed).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    site.sent.take();
    site
}

/// A stranger claims the remote network as this one, alone, on a link
/// whose router doesn't answer What-Is-Network-Number (one older than the
/// message): its claim draws the question and its repeat answers it. The
/// client reaches that network through `ROUTER`, so before taking it the
/// client asks who routes to it, and `ROUTER` does: a link can't be a
/// network a router on it routes to. The number is refused and counted,
/// Device 10 stays routed, and requests to it go through its router.
#[tokio::test(start_paused = true)]
async fn a_number_a_router_routes_to_is_refused() {
    let mut site = holding_a_routed_device().await;
    for _ in 0..2 {
        broadcast_from(&site.inbound, &number(REMOTE_NETWORK, 1), &STRANGER).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(router_queries(&site, REMOTE_NETWORK), 1, "the check");
    router_routes_to(&site, REMOTE_NETWORK).await;
    tokio::time::sleep(PAST_THE_CHECK).await;
    assert_eq!(site.client.network_number(), None);
    assert_eq!(site.client.network_number_disagreements(), 1);
    let device = site
        .client
        .get_device(10)
        .await
        .unwrap()
        .expect("Device 10");
    assert_eq!(device.source_network, Some(REMOTE_NETWORK), "{device:?}");
    let to = site.client.resolve_device(10).await.unwrap();
    assert_eq!(
        to,
        crate::client::Station::routed(&ROUTER, REMOTE_NETWORK, &FAR)
    );
    site.client.stop().await.unwrap();
}

/// #2572 with the check: a router relays a local device's I-Am back with
/// this network's number, so the client holds a routed row on it. The
/// router answers the client's question with that number; before taking
/// it, the client asks who routes to it, no router does (a router doesn't
/// list the network its port is on), and the number is taken and the
/// routed row folds into the local one: one check later than without it.
#[tokio::test(start_paused = true)]
async fn an_honest_late_answer_is_checked_then_folds() {
    let mut site = open_site(true).await;
    send_to_client(&site.inbound, &i_am(9), &PEER, None).await;
    let echo = relayed_from(THIS_NETWORK, &PEER);
    send_to_client(&site.inbound, &i_am(9), &ROUTER, echo).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    broadcast_from(&site.inbound, &number(THIS_NETWORK, 1), &ROUTER).await;
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(site.client.network_number(), None, "checking first");
    assert_eq!(router_queries(&site, THIS_NETWORK), 1);
    assert!(matches!(
        site.client.get_device(9).await,
        Err(Error::AmbiguousDevice { instance: 9, .. })
    ));
    tokio::time::sleep(PAST_THE_CHECK).await;
    assert_eq!(site.client.network_number(), Some(THIS_NETWORK));
    let device = site
        .client
        .get_device(9)
        .await
        .unwrap()
        .expect("one device");
    assert!(device.is_local());
    site.client.stop().await.unwrap();
}

/// A check that may have lost an answer (here the Number owner's queue
/// overflows while the check waits) refuses the number: the router's
/// answer may have been among what was lost.
#[tokio::test(start_paused = true)]
async fn a_check_that_may_have_lost_an_answer_refuses() {
    let mut site = holding_a_routed_device().await;
    for _ in 0..2 {
        broadcast_from(&site.inbound, &number(REMOTE_NETWORK, 1), &STRANGER).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(router_queries(&site, REMOTE_NETWORK), 1, "checking");
    for _ in 0..300 {
        broadcast_from(&site.inbound, &number(REMOTE_NETWORK, 1), &STRANGER).await;
    }
    tokio::time::sleep(PAST_THE_CHECK).await;
    assert_eq!(site.client.network_number(), None);
    assert_eq!(site.client.network_number_disagreements(), 1, "counted");
    let device = site
        .client
        .get_device(10)
        .await
        .unwrap()
        .expect("Device 10");
    assert_eq!(device.source_network, Some(REMOTE_NETWORK), "{device:?}");
    site.client.stop().await.unwrap();
}

/// A client stopped while its check waits takes nothing from it, and the
/// stop doesn't wait for the check.
#[tokio::test(start_paused = true)]
async fn a_stop_during_the_check_takes_nothing() {
    let mut site = holding_a_routed_device().await;
    for _ in 0..2 {
        broadcast_from(&site.inbound, &number(REMOTE_NETWORK, 1), &STRANGER).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(PAST_THE_WINDOW).await;
    assert_eq!(router_queries(&site, REMOTE_NETWORK), 1, "checking");
    let stopped = tokio::time::Instant::now();
    site.client.stop().await.unwrap();
    assert!(stopped.elapsed() < Duration::from_secs(1), "no wait for it");
    tokio::time::sleep(PAST_THE_CHECK).await;
    assert_eq!(site.client.network_number(), None);
}
