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
        assert!(sent.monotonic_timestamp <= received.monotonic_timestamp);
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
    let held = slow.try_recv().unwrap();
    assert_eq!((&held.bytes[..], held.missed_before), (&[1][..], 0));
    assert!(slow.try_recv().is_none());
    for octet in 0..5u8 {
        assert_eq!(&fast.try_recv().unwrap().bytes[..], [octet]);
    }
}

/// The queue loses the newest frames, so a gap opens after the frames it
/// holds: the next frame queued reports it, once, and only to the receiver
/// that lost them. `dropped` covers the gaps reported and the trailing one.
#[test]
fn the_next_queued_frame_reports_the_gap_in_front_of_it() {
    let tap = FrameTap::new();
    let mut slow = tap.subscribe_with_capacity(1);
    let mut fast = tap.subscribe_with_capacity(16);
    let record = |octet: u8| tap.record(TapDirection::Inbound, DataLink::Bip, peer, &[octet]);
    for octet in 0..3 {
        record(octet);
    }
    // Frames 1 and 2 are lost after frame 0, which was queued with no gap.
    let first = slow.try_recv().unwrap();
    assert_eq!((&first.bytes[..], first.missed_before), (&[0][..], 0));
    assert_eq!(
        slow.dropped(),
        2,
        "the trailing gap counts before it is reported"
    );
    record(3);
    record(4);
    let next = slow.try_recv().unwrap();
    assert_eq!((&next.bytes[..], next.missed_before), (&[3][..], 2));
    assert!(slow.try_recv().is_none());
    record(5);
    let after = slow.try_recv().unwrap();
    assert_eq!((&after.bytes[..], after.missed_before), (&[5][..], 1));
    assert_eq!(slow.dropped(), 3);
    for octet in 0..6u8 {
        let frame = fast.try_recv().unwrap();
        assert_eq!((&frame.bytes[..], frame.missed_before), (&[octet][..], 0));
    }
    assert_eq!(fast.dropped(), 0);
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

#[test]
fn close_includes_an_overflow_in_progress_and_detaches_the_receiver() {
    let tap = FrameTap::new();
    let slow = tap.subscribe_with_capacity(1);
    let mut witness = tap.subscribe();
    tap.record(TapDirection::Inbound, DataLink::Sc, peer, &[1]);
    let frame = witness.try_recv().unwrap();

    // Stage a publication between its full-queue rejection and the counter
    // increment. Hold the same lock across these steps as FrameTap::publish.
    let sinks = tap.lock_sinks();
    let slow_sink = &sinks[0];
    assert!(matches!(
        slow_sink.tx.try_send(frame),
        Err(mpsc::error::TrySendError::Full(_))
    ));
    let closing = std::thread::spawn(move || slow.close());
    // The receiver has sealed its queue. It must wait for the in-flight count.
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(slow_sink.tx.closed());
    slow_sink.dropped.fetch_add(1, Ordering::Relaxed);
    tap.shared.dropped.fetch_add(1, Ordering::Relaxed);
    drop(sinks);

    assert_eq!(closing.join().unwrap(), 1);
    assert_eq!(tap.counters().subscribers, 1);
    tap.record(TapDirection::Inbound, DataLink::Sc, peer, &[2]);
    assert_eq!(&witness.try_recv().unwrap().bytes[..], [2]);
    assert_eq!(tap.counters().dropped, 1);
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
