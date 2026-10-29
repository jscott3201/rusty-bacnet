//! A port's NPDU limit after start (#1696): the router follows the changes
//! a transport publishes, and counts a send its transport refuses as too
//! long, on ports the test drives. The BACnet/SC failovers are in
//! `tests/sc_limits.rs`.

use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_npdu::NetworkMessageType;
use bacnet_npdu::NpduLimits;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use std::sync::atomic::Ordering;
use tokio::sync::watch;
use tokio::time::{Duration, timeout};

use super::driven_port::{Driven, Handle, Sent};
use super::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};

/// The node on network 1000 that sends, and the one on 2000 it sends to.
const NODE: &[u8] = &[7];
const PEER: &[u8] = &[9];

/// A frame for `PEER` on network 2000 with an APDU of `apdu` octets: 7
/// octets more on the way in, and 6 once rewritten onto network 2000,
/// which drops the DNET, DADR and hop count and adds `NODE` as SADR.
fn to_peer(apdu: usize) -> Vec<u8> {
    let mut payload = vec![0x10, 0x08];
    payload.resize(apdu, 0);
    let npdu = Npdu {
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(PEER),
        }),
        hop_count: 255,
        payload: Bytes::from(payload),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    buf.to_vec()
}

/// A router on two driven ports, networks 1000 and 2000.
async fn start(ports: [Driven; 2]) -> BACnetRouter {
    let [first, second] = ports;
    let ports = vec![
        RouterPort {
            transport: first,
            network_number: 1000,
        },
        RouterPort {
            transport: second,
            network_number: 2000,
        },
    ];
    let StartedRouter { router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    router
}

/// The next send on `port` for which `pick` is true, skipping the rest.
async fn next_send(port: &mut Handle, pick: impl Fn(&Sent) -> bool) -> Sent {
    let wanted = async {
        loop {
            let sent = port.sent.recv().await.expect("the port is open");
            if pick(&sent) {
                return sent;
            }
        }
    };
    timeout(Duration::from_secs(5), wanted)
        .await
        .expect("the send comes")
}

/// Yield until `done`, failing after 10,000 scheduler rounds.
async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..10_000 {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("{what}: not reached");
}

/// Whether `sent` is a reject with reason 4 for network 2000, to `NODE`.
fn is_reason_4(sent: &Sent) -> bool {
    let npdu = decode_npdu(Bytes::from(sent.npdu.clone())).unwrap();
    sent.to.as_deref() == Some(NODE)
        && npdu.message_type == Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
        && npdu.payload.as_ref() == [4, 0x07, 0xD0]
}

/// A limit the transport publishes routes every frame after it: lowered to
/// 480 octets, a 600-octet relay draws reason 4 and the count; raised back,
/// the same relay goes.
#[tokio::test(start_paused = true)]
async fn a_published_limit_routes_every_later_frame() {
    let (changes_tx, changes) = watch::channel(NpduLimits::STANDARD);
    let (first, mut node) = Driven::new(1, 1497);
    let (second, mut peer) = Driven::new(2, 1497);
    let router = start([first, second.publishing(changes)]).await;
    let limit = |router: &BACnetRouter| router.shared.engine.lock().ports()[1].max_npdu;

    changes_tx.send_replace(NpduLimits::both(480));
    until("the router has 480", || {
        limit(&router) == NpduLimits::both(480)
    })
    .await;
    node.receive(to_peer(594), NODE).await;
    next_send(&mut node, is_reason_4).await;
    assert_eq!(router.counters().egress_too_long, 1);

    changes_tx.send_replace(NpduLimits::STANDARD);
    until("the router has 1497", || {
        limit(&router) == NpduLimits::both(1497)
    })
    .await;
    node.receive(to_peer(594), NODE).await;
    let relayed = next_send(&mut peer, |sent| sent.to.as_deref() == Some(PEER)).await;
    assert_eq!((relayed.npdu.len(), relayed.taken), (600, true));
    assert_eq!(router.counters().egress_too_long, 1);
}

/// A send already past the engine when the port's limit falls, which the
/// transport then refuses as too long, counts as `egress_too_long` and
/// `refused`, and returns reason 4; an unrelated failure does not.
#[tokio::test(start_paused = true)]
async fn a_send_the_port_refuses_as_too_long_returns_reason_4() {
    let (first, mut node) = Driven::new(1, 1497);
    let (second, mut peer) = Driven::new(2, 1497);
    let router = start([first, second]).await;
    assert!(node.started.load(Ordering::SeqCst) && peer.started.load(Ordering::SeqCst));

    peer.down.store(true, Ordering::SeqCst);
    node.receive(to_peer(10), NODE).await;
    let refused = next_send(&mut peer, |sent| sent.to.as_deref() == Some(PEER)).await;
    assert!(!refused.taken);

    // The port now carries less, and the router hasn't heard.
    peer.down.store(false, Ordering::SeqCst);
    peer.limit.store(480, Ordering::SeqCst);
    node.receive(to_peer(594), NODE).await;
    let refused = next_send(&mut peer, |sent| sent.to.as_deref() == Some(PEER)).await;
    assert_eq!((refused.npdu.len(), refused.taken), (600, false));
    until("the refusal is counted", || {
        router.counters().egress_too_long == 1
    })
    .await;
    let counters = router.counters();
    assert_eq!((counters.refused, counters.rejects), (1, 1));
    let rejected = next_send(&mut node, is_reason_4).await;
    assert!(rejected.taken);
}

/// #2087: an application port keeps the receive capacity it declared when
/// it was made after a router port's limit falls. The router refuses a relay
/// too long for the smaller path with reason 4, and the port's own send too
/// long for it fails as too long, while its egress limit follows the path.
#[tokio::test(start_paused = true)]
async fn an_application_port_keeps_its_declaration_when_a_path_shrinks() {
    use bacnet_transport::port::TransportPort;
    use bacnet_types::error::Error;

    let (changes_tx, changes) = watch::channel(NpduLimits::STANDARD);
    let (first, mut node) = Driven::new(1, 1497);
    let (second, _peer) = Driven::new(2, 1497);
    let ports = vec![
        RouterPort {
            transport: first,
            network_number: 1000,
        },
        RouterPort {
            transport: second.publishing(changes),
            network_number: 2000,
        },
    ];
    let started = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let (router, mut port, _controls) = started.into_application_port();
    let _apdus = port.start().await.unwrap();
    assert_eq!(port.local_receive_apdu_capacity(), 1476);
    assert_eq!(port.egress_apdu_limit(), 1476);
    let limit = |router: &BACnetRouter| router.shared.engine.lock().ports()[1].max_npdu;

    changes_tx.send_replace(NpduLimits::both(480));
    until("the router has 480", || {
        limit(&router) == NpduLimits::both(480)
    })
    .await;
    assert_eq!(port.local_receive_apdu_capacity(), 1476);
    assert!(port.egress_apdu_limit() < 480);

    // A relay onto the smaller path: reason 4 back to its source.
    node.receive(to_peer(594), NODE).await;
    next_send(&mut node, is_reason_4).await;

    // The port's own send onto it: too long, refused before it goes.
    let mut npdu = vec![1, 0x20, 0x07, 0xD0, 1, PEER[0], 255, 0x10, 0x08];
    npdu.resize(600, 0);
    let error = port.send_unicast(&npdu, &[0xEE]).await.unwrap_err();
    assert!(matches!(error, Error::SendTooLong { .. }), "{error:?}");
    assert_eq!(router.counters().egress_too_long, 2);
}
