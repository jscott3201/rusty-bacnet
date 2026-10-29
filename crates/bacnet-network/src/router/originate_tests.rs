//! The router's own requests go through its engine and its port queues,
//! routed by the table with an explicit source; replies reach its local
//! receiver; a full queue and a stopped router refuse (#1699).

use bacnet_npdu::{Address, Mac, NetworkPriority};
use bacnet_router_core::PortId;

use super::priority_harness::*;
use super::send_queue::CAPACITY;
use super::{OriginateError, OriginateFailure, Request, SourceSelection};

const READ_PROPERTY: [u8; 4] = [0x00, 0x05, 0x01, 0x0C];

fn to_3000(source: SourceSelection) -> Request<'static> {
    let mut request = Request::new(
        Address {
            network: 3000,
            mac: &[5],
        },
        &READ_PROPERTY,
    );
    request.expecting_reply = true;
    request.source = source;
    request
}

#[tokio::test]
async fn a_request_is_routed_by_the_table_and_its_reply_comes_home() {
    let (mut router, mut apdus, mut peers) = start(2).await;
    let now = router.now();
    assert!(
        router
            .table()
            .add_learned(3000, PortId(1), Mac::from_array([7]), now)
    );

    // From the egress port's address: no SNET, to the next router.
    router
        .originate(&to_3000(SourceSelection::EgressPort))
        .unwrap();
    let sent = sending(&mut peers[1]).await;
    assert_eq!(sent.to, Some(vec![7]));
    assert_eq!(
        sent.npdu,
        [&[1, 0x24, 0x0B, 0xB8, 1, 5, 255][..], &READ_PROPERTY].concat()
    );
    assert!(sent.attributes.is_empty());
    sent.complete.send(Ok(())).unwrap();

    // From port 0's address: SNET 1000 and the router's MAC there.
    let mut urgent = to_3000(SourceSelection::Port(PortId(0)));
    urgent.priority = NetworkPriority::Urgent;
    router.originate(&urgent).unwrap();
    let sent = sending(&mut peers[1]).await;
    assert_eq!(
        sent.npdu,
        [
            &[1, 0x2D, 0x0B, 0xB8, 1, 5, 0x03, 0xE8, 1, 1, 255][..],
            &READ_PROPERTY
        ]
        .concat()
    );
    sent.complete.send(Ok(())).unwrap();
    assert!(peers[0].sent.try_recv().is_err(), "nothing on port 0");
    assert_eq!(router.counters().originated, 2);

    // The next router hands the reply back on network 2000, SNET 3000.
    let reply = [1, 0x08, 0x0B, 0xB8, 1, 5, 0x30, 0x05, 0x0C];
    receive(&peers[1], reply.to_vec(), 0).await;
    let delivered = apdus.recv().await.unwrap();
    assert_eq!(delivered.apdu.as_ref(), [0x30, 0x05, 0x0C]);
    assert_eq!(delivered.ingress_network, Some(2000));
    let source = delivered.source_network.expect("its SNET");
    assert_eq!(
        (source.network, source.mac_address.as_slice()),
        (3000, &[5][..])
    );
    router.stop().await.unwrap();
}

#[tokio::test]
async fn no_route_searches_and_refuses() {
    let (mut router, _apdus, mut peers) = start(2).await;
    assert_eq!(
        router.originate(&to_3000(SourceSelection::EgressPort)),
        Err(OriginateFailure::Refused(OriginateError::NoRoute))
    );
    for peer in &mut peers {
        let search = sending(peer).await;
        assert_eq!(search.to, None);
        assert_eq!(search.npdu, [1, 0x80, 0x00, 0x0B, 0xB8]);
        search.complete.send(Ok(())).unwrap();
    }
    assert_eq!(router.counters().originated, 0);
    router.stop().await.unwrap();
}

#[tokio::test]
async fn a_full_queue_refuses_and_a_stopped_router_refuses_everything() {
    let (mut router, _apdus, mut peers) = start(2).await;
    let now = router.now();
    assert!(
        router
            .table()
            .add_learned(3000, PortId(1), Mac::from_array([7]), now)
    );
    let request = to_3000(SourceSelection::EgressPort);
    router.originate(&request).unwrap();
    let _in_progress = sending(&mut peers[1]).await;
    for _ in 0..CAPACITY {
        router.originate(&request).unwrap();
    }
    assert_eq!(router.originate(&request), Err(OriginateFailure::QueueFull));
    assert_eq!(router.counters().refused, 1);

    router.stop().await.unwrap();
    assert_eq!(router.originate(&request), Err(OriginateFailure::Stopped));
}

/// A request on network 2000 or 1000, each a port's own.
fn to_port(network: u16) -> Request<'static> {
    Request::new(Address { network, mac: &[5] }, &READ_PROPERTY)
}

/// Only the request's own refusal fails it: a Router-Busy refused by a
/// full queue in the same drain doesn't.
#[tokio::test(start_paused = true)]
async fn a_busy_refused_in_the_same_drain_doesnt_fail_a_queued_request() {
    let options = super::RouterOptions::new().congestion(super::Congestion::DEFAULT);
    let (mut router, _apdus, mut peers) = start_with(2, options).await;
    // Port 0's queue full, one send in progress.
    router.originate(&to_port(1000)).unwrap();
    let _port_0 = sending(&mut peers[0]).await;
    for _ in 0..CAPACITY {
        router.originate(&to_port(1000)).unwrap();
    }
    // Port 1's queue full, then one slot freed: waiting to clog.
    router.originate(&to_port(2000)).unwrap();
    let in_progress = sending(&mut peers[1]).await;
    for _ in 0..CAPACITY {
        router.originate(&to_port(2000)).unwrap();
    }
    in_progress.complete.send(Ok(())).unwrap();
    let _port_1 = sending(&mut peers[1]).await;

    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    let refused = router.counters().refused;
    // Admitted, filling port 1 past its clog time: its Router-Busy for
    // port 0 is refused by port 0's full queue in this same drain.
    assert_eq!(router.originate(&to_port(2000)), Ok(()));
    let counts = router.counters();
    assert!(counts.busy_announced >= 1);
    assert!(counts.refused > refused, "the Busy was refused");
    // Now port 1 is full: the next request is refused itself.
    assert_eq!(
        router.originate(&to_port(2000)),
        Err(OriginateFailure::QueueFull)
    );
    router.stop().await.unwrap();
}

/// A global Who-Is from the router's own application leaves by every
/// port as that link's broadcast, still global with a full hop count, from
/// the address the request selects (#1809).
#[tokio::test]
async fn a_global_broadcast_leaves_by_every_port() {
    const WHO_IS: [u8; 2] = [0x10, 0x08];
    let (mut router, _apdus, mut peers) = start(3).await;
    let global = |source| {
        let mut request = Request::new(
            Address {
                network: 0xFFFF,
                mac: &[],
            },
            &WHO_IS,
        );
        request.source = source;
        request
    };
    let bare = [&[1, 0x20, 0xFF, 0xFF, 0, 255][..], &WHO_IS].concat();

    router
        .originate(&global(SourceSelection::EgressPort))
        .unwrap();
    for peer in &mut peers {
        let sent = sending(peer).await;
        assert_eq!(sent.to, None, "the link's broadcast");
        assert_eq!(sent.npdu, bare);
        assert!(sent.attributes.is_empty());
        sent.complete.send(Ok(())).unwrap();
    }

    // From port 1's address: SNET 2000 and SADR [2] on the other ports,
    // none on port 1 itself.
    router
        .originate(&global(SourceSelection::Port(PortId(1))))
        .unwrap();
    let sourced = [
        &[1, 0x28, 0xFF, 0xFF, 0, 0x07, 0xD0, 1, 2, 255][..],
        &WHO_IS,
    ]
    .concat();
    for (index, peer) in peers.iter_mut().enumerate() {
        let sent = sending(peer).await;
        assert_eq!(sent.to, None, "port {index}");
        let expected = if index == 1 { &bare } else { &sourced };
        assert_eq!(&sent.npdu, expected, "port {index}");
        sent.complete.send(Ok(())).unwrap();
    }
    assert_eq!(router.counters().originated, 2);
    router.stop().await.unwrap();
}
