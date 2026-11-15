use super::*;
use std::future::Future;
use std::task::{Context, Waker};

#[tokio::test(start_paused = true)]
async fn owner_close_releases_receiver_and_wakes_pending_read() {
    let tap = bacnet_transport::tap::FrameTap::new();
    let owner = FrameSubscriptions::default();
    let iterator = owner.subscribe(tap.subscribe(), "local".into(), Clock::fixed());
    let (pending, _hook) = iterator.subscription.stream.next();
    tokio::pin!(pending);
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(tap.counters().subscribers, 1);
    owner.close();
    assert_eq!(tap.counters().subscribers, 0);
    assert!(pending.await.is_none());
    let fresh = owner.subscribe(tap.subscribe(), "new generation".into(), Clock::fixed());
    assert!(iterator.subscription.stream.next().0.await.is_none());
    assert_eq!(tap.counters().subscribers, 1);
    drop(fresh);
    assert_eq!(tap.counters().subscribers, 0);
}

#[tokio::test(start_paused = true)]
async fn cancelling_native_wait_keeps_subscription_until_iterator_drop() {
    let tap = bacnet_transport::tap::FrameTap::new();
    let owner = FrameSubscriptions::default();
    let iterator = owner.subscribe(tap.subscribe(), "local".into(), Clock::fixed());
    {
        let (pending, _hook) = iterator.subscription.stream.next();
        tokio::pin!(pending);
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(tap.counters().subscribers, 1);
    drop(iterator);
    assert_eq!(tap.counters().subscribers, 0);
}

/// `Frame.link` is the `transport=` token, and a link no Python owner runs
/// reads as the documented fallback.
#[test]
fn every_link_reads_as_its_transport_token() {
    let tokens = [
        (DataLink::Bip, "bip"),
        (DataLink::Bip6, "ipv6"),
        (DataLink::Sc, "sc"),
        (DataLink::Mstp, "mstp"),
        (DataLink::Ethernet, "other"),
        (DataLink::Loopback, "other"),
    ];
    for (link, token) in tokens {
        assert_eq!(link_token(link), token, "{link}");
    }
}

/// Each link's `local_address` and `peer` share one form: socket addresses
/// on B/IP and B/IPv6, decimal stations on MS/TP, colon hex for MACs. SC's
/// peer is the hub connection, named as `ScEvent.hub` names it.
#[test]
fn each_link_writes_local_and_peer_addresses_alike() {
    use bacnet_transport::sc::ScActiveHub;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

    let local = |link, mac: &[u8], socket| local_address(link, mac, socket).map(|a| a.to_string());
    let v4 = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 7), 47808));
    assert_eq!(
        local(DataLink::Bip, &[], Some(v4)).unwrap(),
        "192.0.2.7:47808"
    );
    assert_eq!(peer_text(TapPeer::Udp(v4)), "192.0.2.7:47808");
    assert_eq!(
        local(DataLink::Bip, &[192, 0, 2, 7, 0xBA, 0xC0], None),
        None
    );

    let v6 = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 47808, 0, 0));
    let scoped = SocketAddr::V6(SocketAddrV6::new("fe80::1".parse().unwrap(), 47809, 0, 3));
    // The socket address, never the VMAC in the MAC.
    assert_eq!(
        local(DataLink::Bip6, &[0, 4, 0xD2], Some(v6)).unwrap(),
        "[::1]:47808"
    );
    assert_eq!(peer_text(TapPeer::Udp(v6)), "[::1]:47808");
    assert_eq!(
        local(DataLink::Bip6, &[], Some(scoped)).unwrap(),
        "[fe80::1%3]:47809"
    );
    assert_eq!(peer_text(TapPeer::Udp(scoped)), "[fe80::1%3]:47809");
    assert_eq!(local(DataLink::Bip6, &[0, 4, 0xD2], None), None);

    assert_eq!(local(DataLink::Mstp, &[3], None).unwrap(), "3");
    assert_eq!(peer_text(TapPeer::Mstp(3)), "3");
    assert_eq!(peer_text(TapPeer::Mstp(255)), "255");
    assert_eq!(local(DataLink::Mstp, &[], None), None);
    assert_eq!(local(DataLink::Mstp, &[0, 3], None), None);

    let vmac = [0x02, 0, 0, 0, 0, 0x2A];
    assert_eq!(
        local(DataLink::Sc, &vmac, None).unwrap(),
        "02:00:00:00:00:2a"
    );
    assert_eq!(peer_text(TapPeer::ScHub(ScActiveHub::Primary)), "primary");
    assert_eq!(peer_text(TapPeer::ScHub(ScActiveHub::Failover)), "failover");
    assert_eq!(local(DataLink::Sc, &[], None), None);

    let mac = [0x00, 0x1B, 0x63, 0x84, 0x45, 0xE6];
    assert_eq!(
        local(DataLink::Ethernet, &mac, None).unwrap(),
        "00:1b:63:84:45:e6"
    );
    assert_eq!(peer_text(TapPeer::Ethernet(mac)), "00:1b:63:84:45:e6");
}

/// The repr names the frame without its octets.
#[test]
fn a_frame_repr_is_short() {
    let tap = bacnet_transport::tap::FrameTap::new();
    let mut receiver = tap.subscribe();
    let octets = [0x81; 600];
    tap_record(&tap, TapDirection::Inbound, &octets);
    let frame = PyFrame {
        frame: receiver.try_recv().unwrap(),
        local_address: "127.0.0.1:47808".into(),
        missed_before: 0,
        monotonic: 0.0,
    };
    assert_eq!(
        frame.__repr__(),
        "Frame(direction='inbound', link='bip', peer='192.0.2.9:47808', length=600)"
    );
}

/// What a B/IP transport would hand its tap, from 192.0.2.9:47808.
fn tap_record(tap: &bacnet_transport::tap::FrameTap, direction: TapDirection, octets: &[u8]) {
    tap.record_for_test(
        direction,
        DataLink::Bip,
        TapPeer::Udp(SocketAddr::from(([192, 0, 2, 9], 47808))),
        octets,
    );
}

/// Take the next fresh frame from `iterator` as `__anext__` does, with its
/// delivery hook, which the caller settles, puts back or drops.
async fn fresh(iterator: &PyFrameIterator) -> (PyFrame, Arc<dyn crate::py_async::PutBack>) {
    let (next, hook) = iterator.subscription.stream.next();
    let next = tokio::time::timeout(std::time::Duration::from_secs(1), next)
        .await
        .expect("a frame is queued");
    let Some(crate::event_stream::Next::Fresh(frame)) = next else {
        panic!("expected a fresh frame");
    };
    (iterator.converter().frame(frame), hook)
}

/// A full one-frame queue: frame 0 queued, then 1 and 2 lost behind it.
/// Returns the tap, the owner (whose drop would end the subscription) and
/// the iterator, frame 0 already read.
async fn after_a_gap_of_two() -> (
    bacnet_transport::tap::FrameTap,
    FrameSubscriptions,
    PyFrameIterator,
) {
    let tap = bacnet_transport::tap::FrameTap::new();
    let owner = FrameSubscriptions::default();
    let iterator = owner.subscribe(
        tap.subscribe_with_capacity(1),
        "local".into(),
        Clock::fixed(),
    );
    for octet in 0..3 {
        tap_record(&tap, TapDirection::Inbound, &[octet]);
    }
    let (first, hook) = fresh(&iterator).await;
    assert_eq!((&first.frame.bytes[..], first.missed_before), (&[0][..], 0));
    hook.settled();
    assert_eq!(iterator.missed(), 2, "the trailing gap");
    (tap, owner, iterator)
}

/// A read whose future was cancelled puts its frame back; the next read
/// returns that same frame, gap and all, ahead of a newer one.
#[tokio::test(start_paused = true)]
async fn a_put_back_frame_comes_back_with_its_gap() {
    use crate::py_async::Via;

    Python::initialize();
    let (tap, _owner, iterator) = after_a_gap_of_two().await;
    tap_record(&tap, TapDirection::Inbound, &[3]);
    let (frame, hook) = fresh(&iterator).await;
    assert_eq!((&frame.frame.bytes[..], frame.missed_before), (&[3][..], 2));
    Python::attach(|py| {
        let object = Py::new(py, frame).unwrap().into_any();
        hook.put_back(py, object, Via::OnLoop);
    });
    hook.settled();
    tap_record(&tap, TapDirection::Inbound, &[4]);
    let (next, hook) = iterator.subscription.stream.next();
    let again = tokio::time::timeout(std::time::Duration::from_secs(1), next)
        .await
        .expect("the put-back frame is ready");
    let Some(crate::event_stream::Next::Again(object)) = again else {
        panic!("expected the put-back frame first");
    };
    Python::attach(|py| {
        let bound = object.bind(py).cast::<PyFrame>().unwrap().clone();
        let frame = bound.get();
        assert_eq!((&frame.frame.bytes[..], frame.missed_before), (&[3][..], 2));
    });
    hook.settled();
    let (newer, hook) = fresh(&iterator).await;
    assert_eq!((&newer.frame.bytes[..], newer.missed_before), (&[4][..], 0));
    hook.settled();
    assert_eq!(iterator.missed(), 2);
}

/// A frame dropped on its way to Python, as when its loop closed, counts
/// in `FrameIterator.missed`, and the next frame reports it, with the gap
/// it carried, so the gaps received add up to the total.
#[tokio::test(start_paused = true)]
async fn a_frame_lost_on_its_way_carries_into_the_next_gap() {
    let (tap, _owner, iterator) = after_a_gap_of_two().await;
    tap_record(&tap, TapDirection::Inbound, &[3]);
    let (lost, hook) = fresh(&iterator).await;
    assert_eq!(lost.missed_before, 2);
    // Neither delivered nor put back: the delivery drops armed.
    drop((lost, hook));
    assert_eq!(iterator.missed(), 3, "two overflowed, one lost on its way");
    tap_record(&tap, TapDirection::Inbound, &[4]);
    let (next, hook) = fresh(&iterator).await;
    assert_eq!((&next.frame.bytes[..], next.missed_before), (&[4][..], 3));
    hook.settled();
    // Frames received reported 0 and 3; nothing was lost after the last.
    assert_eq!(iterator.missed(), 3);
}

/// `Frame.monotonic` converts the capture instant by its exact offset from
/// the clock pair, earlier instants included, and never reads backwards.
#[test]
fn capture_instants_convert_by_their_offset_from_the_pair() {
    let anchor = Instant::now();
    let clock = Clock {
        instant: anchor,
        python_ns: 5_000_000_000,
    };
    let later = anchor + std::time::Duration::from_micros(1_500);
    assert_eq!(clock.seconds(anchor), 5.0);
    assert_eq!(clock.seconds(later), 5.0015);
    if let Some(earlier) = anchor.checked_sub(std::time::Duration::from_millis(250)) {
        assert_eq!(clock.seconds(earlier), 4.75);
    }
    assert_eq!(posix_seconds(SystemTime::UNIX_EPOCH), 0.0);
    let before = SystemTime::UNIX_EPOCH - std::time::Duration::from_millis(500);
    assert_eq!(posix_seconds(before), -0.5);
}
