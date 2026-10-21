//! [`BACnetRouter::start`] over loopback ports with every option on, and
//! with none (#1220).

use std::sync::Arc;

use bacnet_npdu::NetworkMessageType;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use tokio::time::{timeout, Duration};

use super::*;
use crate::layer::QueueAdmissionSnapshot;
use crate::loopback_fixture::recv;
use crate::router::RouterPort;

/// I-Am-Router-To-Network for 3000, as peer B announces it.
const I_AM_3000: [u8; 5] = [0x01, 0x80, 0x01, 0x0B, 0xB8];
/// Reject-Message-To-Network with no DNET, reason 1 for 3000: addressed to
/// the router itself.
const REJECT_3000: [u8; 6] = [0x01, 0x80, 0x03, 0x01, 0x0B, 0xB8];
/// A local APDU for network 6000, which the router has no route to, and the
/// reason 1 reject it draws.
const TO_6000: [u8; 8] = [0x01, 0x20, 0x17, 0x70, 0x00, 0xFF, 0x10, 0x08];
const REJECT_6000: [u8; 6] = [0x01, 0x80, 0x03, 0x01, 0x17, 0x70];

/// Loopback peers A (0A, network 1000) and B (0B, network 2000), started,
/// and the router ports they face: the router is 01 and 02.
async fn ports() -> (
    Vec<RouterPort<LoopbackTransport>>,
    [(LoopbackTransport, mpsc::Receiver<ReceivedNpdu>); 2],
) {
    let (port_a, mut peer_a) = LoopbackTransport::pair(vec![0x01], vec![0x0A]);
    let (port_b, mut peer_b) = LoopbackTransport::pair(vec![0x02], vec![0x0B]);
    let from_router_a = peer_a.start().await.unwrap();
    let from_router_b = peer_b.start().await.unwrap();
    let ports = vec![
        RouterPort {
            transport: port_a,
            network_number: 1000,
        },
        RouterPort {
            transport: port_b,
            network_number: 2000,
        },
    ];
    (ports, [(peer_a, from_router_a), (peer_b, from_router_b)])
}

#[tokio::test]
async fn one_start_combines_admission_tracking_a_control_policy_and_the_control_receiver() {
    let (ports, [(mut peer_a, _), (mut peer_b, _)]) = ports().await;
    // Hardened, with an authorizer that admits rejects and nothing else.
    let reject = NetworkMessageType::REJECT_MESSAGE_TO_NETWORK;
    let options = RouterOptions::new()
        .track_admission()
        .control_policy(ControlPolicy::Hardened)
        .control_authorizer(Arc::new(move |ctx| ctx.message_type == reject))
        .network_control_receiver();
    let StartedRouter {
        mut router,
        mut apdus,
        network_control,
    } = BACnetRouter::start(ports, options).await.unwrap();
    let mut controls = network_control.expect("asked for");
    let counters = apdus.counters();

    // Peer B announces 3000, which the authorizer refuses, then sends the
    // router a reject (no DNET, reason 1 for 3000), which it admits. One
    // dispatch task handles both, in order.
    peer_b.send_broadcast(&I_AM_3000).await.unwrap();
    let reject_3000 = [0x01, 0x80, 0x03, 0x01, 0x0B, 0xB8];
    peer_b.send_unicast(&reject_3000, &[0x02]).await.unwrap();
    let control = recv(&mut controls).await;
    assert_eq!(control.npdu.payload[..], reject_3000[3..]);
    assert_eq!(control.source_mac.as_slice(), [0x0B]);
    let decisions = router.control_snapshot();
    assert_eq!(
        (decisions.i_am.allow_total, decisions.i_am.deny_total),
        (0, 1)
    );
    assert_eq!(decisions.reject.allow_total, 1);
    assert!(router.table().lookup(3000).is_none());

    // A local APDU from peer A lands in the tracked queue, which counts it.
    peer_a
        .send_unicast(&[0x01, 0x00, 0x10, 0x08], &[0x01])
        .await
        .unwrap();
    let apdu = timeout(Duration::from_secs(2), apdus.recv())
        .await
        .expect("local APDU")
        .expect("queue open");
    assert_eq!(apdu.apdu[..], [0x10, 0x08]);
    assert_eq!(apdu.ingress_network, Some(1000));
    let snapshot = counters.snapshot();
    assert_eq!((snapshot.current_depth, snapshot.high_water), (0, 1));

    router.stop().await;
    peer_a.stop().await.unwrap();
    peer_b.stop().await.unwrap();
}

#[tokio::test]
async fn plain_options_start_a_permissive_router_without_a_control_receiver() {
    let (ports, [(mut peer_a, mut from_router_a), (mut peer_b, _)]) = ports().await;
    let StartedRouter {
        mut router,
        apdus: _apdus,
        network_control,
    } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    assert!(network_control.is_none());

    // The permissive policy admits peer B's I-Am: the router learns 3000 and
    // passes the announcement on to port A.
    peer_b.send_broadcast(&I_AM_3000).await.unwrap();
    loop {
        let frame = recv(&mut from_router_a).await;
        if frame.npdu[..] == I_AM_3000 {
            break;
        }
    }
    assert_eq!(router.control_snapshot().i_am.allow_total, 1);
    assert_eq!(
        router.table().lookup(3000).unwrap().port(),
        crate::router_table::PortId(1)
    );

    router.stop().await;
    peer_a.stop().await.unwrap();
    peer_b.stop().await.unwrap();
}

/// Send `count` rejects addressed to the router from peer B, then [`TO_6000`],
/// and wait for its reject. Port B's dispatch takes them in order, so by then
/// each reject before it has been queued for the control receiver or dropped.
async fn rejects_to_the_router(
    peer_b: &LoopbackTransport,
    from_router_b: &mut mpsc::Receiver<ReceivedNpdu>,
    count: usize,
) {
    for _ in 0..count {
        peer_b.send_unicast(&REJECT_3000, &[0x02]).await.unwrap();
        // The loopback link drops what finds the router's 256-frame queue
        // full (#1713), so let the router take each frame as it goes.
        tokio::task::yield_now().await;
    }
    peer_b.send_unicast(&TO_6000, &[0x02]).await.unwrap();
    while recv(from_router_b).await.npdu[..] != REJECT_6000 {}
}

#[tokio::test]
async fn tracked_network_control_receiver_counts_its_high_water_mark_and_drops() {
    let (ports, [(mut peer_a, _from_router_a), (mut peer_b, mut from_router_b)]) = ports().await;
    let options = RouterOptions::new().network_control_receiver_with_admission();
    let StartedRouter {
        mut router,
        apdus: _apdus,
        network_control,
    } = BACnetRouter::start(ports, options).await.unwrap();
    let mut controls = network_control.expect("asked for");
    let counters = controls.counters();

    // Four more than the 256-control queue holds: the last four are dropped.
    rejects_to_the_router(&peer_b, &mut from_router_b, 260).await;
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 4,
            fairness_drops: 0,
            closed_drops: 0,
        }
    );
    assert_eq!(router.network_control_ingress_sequence(), 260);

    // Draining empties the queue and keeps the high-water mark and drops.
    for sequence in 1..=256 {
        let control = controls.try_recv().unwrap();
        assert_eq!(control.ingress_sequence, sequence);
        assert_eq!(control.npdu.payload[..], REJECT_3000[3..]);
    }
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            full_drops: 4,
            ..Default::default()
        }
    );

    // Once closed, each reject for the router is a closed drop.
    controls.close();
    rejects_to_the_router(&peer_b, &mut from_router_b, 2).await;
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            full_drops: 4,
            closed_drops: 2,
            ..Default::default()
        }
    );

    router.stop().await;
    peer_a.stop().await.unwrap();
    peer_b.stop().await.unwrap();
}
