use super::*;

fn peer() -> TapPeer {
    TapPeer::Mstp(7)
}

#[test]
fn without_a_subscriber_nothing_is_built_or_counted() {
    let tap = FrameTap::new();
    let mut built = false;
    tap.record(
        TapDirection::Outbound,
        DataLink::Mstp,
        || {
            built = true;
            peer()
        },
        &[1, 2, 3],
    );
    assert!(!built, "the peer is worked out only for a subscriber");
    assert_eq!(tap.counters(), FrameTapCounters::default());
}

#[test]
fn each_receiver_gets_each_frame_in_order() {
    let tap = FrameTap::new();
    let mut first = tap.subscribe();
    let mut second = tap.clone().subscribe();
    tap.record(TapDirection::Outbound, DataLink::Mstp, peer, &[1]);
    tap.record(TapDirection::Inbound, DataLink::Mstp, peer, &[2]);
    for receiver in [&mut first, &mut second] {
        let sent = receiver.try_recv().unwrap();
        assert_eq!(
            (sent.direction, sent.link, sent.peer, &sent.bytes[..]),
            (TapDirection::Outbound, DataLink::Mstp, peer(), &[1][..])
        );
        let received = receiver.try_recv().unwrap();
        assert_eq!(received.direction, TapDirection::Inbound);
        assert_eq!(&received.bytes[..], [2]);
        assert!(receiver.try_recv().is_none());
    }
    let counters = tap.counters();
    assert_eq!((counters.subscribers, counters.delivered), (2, 4));
}

/// A full queue drops the new frame for that receiver only, and counts it
/// there and tap-wide; the sender never waits.
#[test]
fn a_full_receiver_loses_new_frames_and_counts_them() {
    let tap = FrameTap::new();
    let mut slow = tap.subscribe_with_capacity(2);
    let mut fast = tap.subscribe_with_capacity(16);
    for octet in 0..5u8 {
        tap.record(TapDirection::Inbound, DataLink::Bip, peer, &[octet]);
    }
    assert_eq!(slow.dropped(), 3);
    assert_eq!(fast.dropped(), 0);
    let counters = tap.counters();
    assert_eq!((counters.delivered, counters.dropped), (7, 3));
    assert_eq!(&slow.try_recv().unwrap().bytes[..], [0]);
    assert_eq!(&slow.try_recv().unwrap().bytes[..], [1]);
    assert!(slow.try_recv().is_none());
    for octet in 0..5u8 {
        assert_eq!(&fast.try_recv().unwrap().bytes[..], [octet]);
    }
}

#[test]
fn a_dropped_receiver_unsubscribes() {
    let tap = FrameTap::new();
    let receiver = tap.subscribe();
    let mut kept = tap.subscribe();
    drop(receiver);
    assert_eq!(tap.counters().subscribers, 1);
    tap.record(TapDirection::Inbound, DataLink::Bip, peer, &[9]);
    assert_eq!(tap.counters().delivered, 1);
    assert_eq!(&kept.try_recv().unwrap().bytes[..], [9]);
    drop(kept);
    assert!(!tap.is_active());
    assert_eq!(tap.lock_sinks().len(), 1, "cleared on the next subscribe");
    let _again = tap.subscribe();
    assert_eq!(tap.lock_sinks().len(), 1);
}

#[tokio::test]
async fn the_queue_closes_when_the_tap_is_gone() {
    let tap = FrameTap::new();
    let mut receiver = tap.subscribe();
    tap.record(TapDirection::Outbound, DataLink::Sc, peer, &[4]);
    drop(tap);
    assert_eq!(&receiver.recv().await.unwrap().bytes[..], [4]);
    assert!(receiver.recv().await.is_none());
}
