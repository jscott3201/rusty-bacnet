//! What the move onto the `bacnet-router-core` engine changed (#1679), each
//! against a real router through [`ingress_harness`](super::ingress_harness):
//!
//! - router time is Tokio time from the router's start, and the sweep runs
//!   at the engine's deadlines (R8);
//! - the control authorizer runs under the engine's lock, and the counts
//!   take no lock, so they return while the table's guard lives;
//! - an I-Could-Be-Router for a new network stops at the route cap;
//! - `start` refuses what the engine can't run: no port, more than 255,
//!   or a router MAC longer than 18 octets;
//! - a global broadcast whose rewrite is longer than the engine's scratch
//!   isn't passed on; with R1, any other NPDU that long has already drawn
//!   reason 4, since no port's limit reaches the scratch.
//!
//! The bounded hold-down pool is `bacnet-router-core`'s own, tested there.

use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::control_policy::{ControlAuthContext, ControlAuthorizer};
use super::dispatch::Shared;
use super::ingress_harness::*;
use super::*;
use crate::router_table::{Instant as RouterInstant, Mac, PortId, ROUTE_CAP};

/// A network message of type `kind` with `data`, as peer `source` on a port
/// sends it.
fn control(kind: u8, data: &[u8], source: u8) -> ReceivedNpdu {
    let mut npdu = vec![1, 0x80, kind];
    npdu.extend_from_slice(data);
    ReceivedNpdu::unverified(
        Bytes::from(npdu),
        MacAddr::from_slice(&[source]),
        true,
        Vec::new(),
        None,
    )
}

/// Let the router's tasks run what is due.
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

/// R8: the router's clock is Tokio's, so paused time drives its table and
/// its sweep. A route last confirmed at the start outlives the sweeps until
/// it is more than 300 s old: the sweep at 360 s purges it, at its
/// deadline to the millisecond. Time moves a minute at a time, as a sweep
/// runs at the time its task wakes.
#[tokio::test(start_paused = true)]
async fn router_time_is_tokio_time_and_the_sweep_runs_at_its_deadline() {
    let (ports, mut peers) = fixture(2);
    let (mut router, _apdus) = launch(ports, RouterOptions::new()).await;
    drain_announcements(&mut peers).await;
    assert_eq!(router.now(), RouterInstant::ZERO);
    let now = router.now();
    router
        .table()
        .add_learned(3000, PortId(1), Mac::from_array([9]), now);
    tokio::time::advance(Duration::from_secs(10)).await;
    assert_eq!(router.now(), RouterInstant::from_secs(10));
    tokio::time::advance(Duration::from_secs(50)).await;
    for _ in 0..4 {
        settle().await;
        tokio::time::advance(Duration::from_secs(60)).await;
    }
    settle().await;
    assert_eq!(router.now(), RouterInstant::from_secs(300));
    assert!(
        router.table().lookup(3000).is_some(),
        "the 300 s sweep keeps it"
    );
    tokio::time::advance(Duration::from_millis(59_999)).await;
    settle().await;
    assert!(router.table().lookup(3000).is_some(), "300 s is not stale");
    tokio::time::advance(Duration::from_millis(1)).await;
    settle().await;
    assert!(
        router.table().lookup(3000).is_none(),
        "the 360 s sweep purges it"
    );
    router.stop().await;
}

/// The authorizer runs with the engine's lock held, as its contract says:
/// it must not block or call into the router.
#[tokio::test]
async fn the_authorizer_runs_under_the_engine_lock() {
    let cell: Arc<OnceLock<Arc<Shared>>> = Arc::default();
    let locked: Arc<StdMutex<Vec<bool>>> = Arc::default();
    let authorizer: ControlAuthorizer = {
        let (cell, locked) = (Arc::clone(&cell), Arc::clone(&locked));
        Arc::new(move |_: &ControlAuthContext| {
            let shared = cell.get().expect("the router is up");
            locked.lock().unwrap().push(shared.engine.is_locked());
            true
        })
    };
    let (ports, mut peers) = fixture(2);
    let (mut router, _apdus) =
        launch(ports, RouterOptions::new().control_authorizer(authorizer)).await;
    drain_announcements(&mut peers).await;
    assert!(cell.set(Arc::clone(&router.shared)).is_ok());
    peers[1]
        .tx
        .send(control(0x01, &[0x0b, 0xb8], 7))
        .await
        .unwrap();
    // The I-Am-Router goes on to port 0 once the gate admits it.
    let SendRequest::Broadcast { npdu, .. } = wire(&mut peers[0]).await else {
        panic!("expected the I-Am-Router relayed");
    };
    assert_eq!(npdu.as_ref(), [1, 0x80, 0x01, 0x0b, 0xb8]);
    assert_eq!(*locked.lock().unwrap(), [true]);
    assert!(!router.shared.engine.is_locked());
    router.stop().await;
}

/// The counts take no lock: on a thread that holds the table's guard,
/// `counters` and the four drop counts return, with what the engine
/// counted before the guard was taken. Had they taken the engine's lock,
/// that thread would wait for itself forever, so it reports back through a
/// channel with a timeout.
#[tokio::test]
async fn the_counts_return_while_the_table_guard_lives() {
    let (ports, mut peers) = fixture(2);
    let (router, _apdus) = launch(ports, RouterOptions::new()).await;
    drain_announcements(&mut peers).await;
    // A global broadcast that also names a DADR is dropped and counted; the
    // reject the barrier draws comes after it on the same port.
    peers[0]
        .tx
        .send(incoming(Some(address(0xFFFF, &[5])), 1))
        .await
        .unwrap();
    barrier(&mut peers[0]).await;

    let router = Arc::new(router);
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = Arc::clone(&router);
    let thread = std::thread::spawn(move || {
        let table = reader.table();
        let counts = (
            reader.counters(),
            reader.address_length_drops(),
            reader.global_broadcast_dadr_drops(),
            reader.broadcast_pdu_type_drops(),
            reader.group_dadr_drops(),
        );
        drop(table);
        let _ = tx.send(counts);
    });
    let (counters, address_length, global_dadr, pdu_type, group_dadr) = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the counts returned while the guard lived");
    thread.join().unwrap();
    assert_eq!((address_length, pdu_type, group_dadr), (0, 0, 0));
    assert_eq!(global_dadr, 1);
    assert_eq!(counters.global_broadcast_dadr_drops, 1);
    assert_eq!(counters.rejects, 1, "the barrier's reject");
    let Ok(mut router) = Arc::try_unwrap(router) else {
        panic!("the reader let the router go");
    };
    router.stop().await;
}

/// An I-Could-Be-Router for a network the table lacks is stored only below
/// the route cap; at the cap it is turned away and counted, where the
/// router before the engine stored it anyway.
#[tokio::test]
async fn i_could_be_router_stops_at_the_route_cap() {
    let (ports, mut peers) = fixture(2);
    let (mut router, _apdus) = launch(ports, RouterOptions::new()).await;
    drain_announcements(&mut peers).await;
    let now = router.now();
    for network in 1000..1254 {
        assert!(router
            .table()
            .add_learned(network, PortId(0), Mac::from_array([9]), now));
    }
    assert_eq!(router.table().len(), ROUTE_CAP);
    peers[1]
        .tx
        .send(control(0x02, &[0x13, 0x88, 5], 7))
        .await
        .unwrap();
    barrier(&mut peers[1]).await;
    {
        let table = router.table();
        assert!(table.lookup(5000).is_none());
        assert_eq!(table.len(), ROUTE_CAP);
        assert_eq!(table.claim_counters().learned_cap_ignored, 1);
    }
    // One route fewer, and the same offer is stored.
    router.table().remove(1253);
    peers[1]
        .tx
        .send(control(0x02, &[0x13, 0x88, 5], 7))
        .await
        .unwrap();
    barrier(&mut peers[1]).await;
    let route = router.table().lookup(5000).expect("stored below the cap");
    assert_eq!(
        (route.port(), *route.next_hop()),
        (PortId(1), Mac::from_array([7]))
    );
    assert_eq!(router.table().claim_counters().learned_cap_ignored, 1);
    assert_quiet(&mut peers);
    router.stop().await;
}

async fn refused(ports: Vec<RouterPort<LoopbackTransport>>) -> Option<Error> {
    BACnetRouter::start(ports, RouterOptions::new()).await.err()
}

fn loopback_ports(macs: &[Vec<u8>]) -> Vec<RouterPort<LoopbackTransport>> {
    macs.iter()
        .enumerate()
        .map(|(index, mac)| RouterPort {
            transport: LoopbackTransport::pair(mac.clone(), vec![0xEE]).0,
            network_number: 100 + index as u16,
        })
        .collect()
}

/// A router needs 1 to 255 ports, a routing table's Port ID naming at most
/// 255, and its own MACs within the 18-octet address bound. The router
/// before the engine started without ports or with more than 255.
#[tokio::test]
async fn start_refuses_what_the_engine_cannot_run() {
    let none = refused(Vec::new()).await.expect("no port");
    assert!(none.to_string().contains("1 to 255 ports"), "{none}");
    let many = refused(loopback_ports(&vec![vec![1]; 256]))
        .await
        .expect("256 ports");
    assert!(many.to_string().contains("not 256"), "{many}");
    let long = refused(loopback_ports(&[vec![1], vec![2; 19]]))
        .await
        .expect("a 19-octet MAC");
    assert!(long.to_string().contains("network 101"), "{long}");
    let StartedRouter { mut router, .. } = BACnetRouter::start(
        loopback_ports(&[vec![1], vec![2; 18]]),
        RouterOptions::new(),
    )
    .await
    .expect("18 octets is within the bound");
    router.stop().await;
}

/// The engine's 65,556-octet scratch bounds what it writes, and no port's
/// limit reaches it: a port carries at most 65,535 octets, so with R1 an
/// NPDU routed toward one network draws reason 4 before its rewrite is
/// written. Only a fan-out is written first. A global broadcast whose
/// rewrite fits the scratch, but no port, goes nowhere, counted as too
/// long; one octet more is dropped, logged and counted in `write_failures`,
/// where the router before the engine sent NPDUs of any length on to the
/// port. It is delivered locally either way.
#[tokio::test]
async fn a_rewrite_past_the_scratch_is_dropped_and_counted() {
    use super::envelope_harness::Harness;
    use bacnet_encoding::npdu::{decode_npdu, NpduAddress};
    use bacnet_transport::port::NpduLimits;

    let widest = [NpduLimits::both(u16::MAX); 2];
    let npdu = |network: u16, mac: &[u8], apdu_len: usize| Npdu {
        destination: Some(NpduAddress {
            network,
            mac_address: MacAddr::from_slice(mac),
        }),
        hop_count: 255,
        payload: Bytes::from(vec![0x10; apdu_len]),
        ..Npdu::default()
    };
    // Version and control, SNET, SLEN and an 18-octet SADR: 23 octets
    // before the APDU on an attached network.
    for (apdu_len, routed) in [(65_512, true), (65_513, false)] {
        let mut h = Harness::with_limits(&[1000, 2000], &widest);
        h.dispatch(h.ctx(0, &[0xA5; 18], npdu(2000, &[5], apdu_len)))
            .await;
        let sent = h.drain(1);
        if routed {
            let [SendRequest::Unicast { npdu, mac, .. }] = &sent[..] else {
                panic!("expected the rewrite on port 1, got {sent:?}");
            };
            assert_eq!(npdu.len(), 65_535);
            assert_eq!(mac.as_slice(), [5]);
        } else {
            assert!(sent.is_empty(), "{apdu_len}");
            let [SendRequest::Unicast { npdu, .. }] = &h.drain(0)[..] else {
                panic!("expected a reject on port 0");
            };
            let reject = decode_npdu(npdu.clone()).unwrap();
            assert_eq!(reject.payload.as_ref(), [4, 0x07, 0xD0]);
        }
        assert_eq!(h.counters().egress_too_long, u64::from(!routed));
        assert_eq!(h.take_write_failures(), 0, "{apdu_len}");
        h.assert_quiet();
    }
    // A global broadcast keeps its DNET, DLEN and hop count too: 27 octets.
    for (apdu_len, written) in [(65_529, true), (65_530, false)] {
        let mut h = Harness::with_limits(&[1000, 2000], &widest);
        h.dispatch(h.ctx(0, &[0xA5; 18], npdu(0xFFFF, &[], apdu_len)))
            .await;
        assert!(h.apdus.try_recv().is_ok(), "delivered locally");
        assert_eq!(h.counters().egress_too_long, u64::from(written));
        assert_eq!(h.take_write_failures(), u64::from(!written), "{apdu_len}");
        h.assert_quiet();
    }
}
