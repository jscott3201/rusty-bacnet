//! The two clocks' exact boundaries, which one wins, and the next deadline
//! (#2742). Moved from the full server's own receiver
//! (`request_progress_expiry.rs`), whose tests these were.

use super::*;

/// Open a transfer with segment zero of a window of three at `at`.
fn open(table: &mut SegmentedRequests, peer: &Peer, at: Instant) {
    acked(
        peer.send(table, &segment(0, 0, 3, true, b"first"), at),
        false,
        0,
        3,
    );
}

/// A gap at `at`: the requester is heard from, and nothing is saved.
fn gap(table: &mut SegmentedRequests, peer: &Peer, at: Instant) {
    acked(
        peer.send(table, &segment(0, 5, 3, true, b"gap"), at),
        true,
        0,
        3,
    );
}

#[test]
fn progress_ends_a_transfer_at_sixteen_seconds_and_not_before() {
    let start = Instant::now();
    let nanosecond = Duration::from_nanos(1);
    for (elapsed, survives) in [
        (PROGRESS_TIMEOUT - nanosecond, true),
        (PROGRESS_TIMEOUT, false),
        (PROGRESS_TIMEOUT + nanosecond, false),
    ] {
        let now = start + elapsed;
        let mut table = SegmentedRequests::new(TSEG);
        let peer = Peer::new(1);
        open(&mut table, &peer, start);
        // Heard from even at the boundary, so only progress can be due.
        gap(&mut table, &peer, now);
        let expired = table.expire(now);
        assert_eq!(table.len() == 1, survives, "elapsed {elapsed:?}");
        assert_eq!(expired.len(), usize::from(!survives), "elapsed {elapsed:?}");
    }
}

#[test]
fn silence_ends_a_transfer_only_once_past_four_segment_timeouts() {
    let start = Instant::now();
    let nanosecond = Duration::from_nanos(1);
    for (idle, survives) in [
        (TSEG * 4 - nanosecond, true),
        (TSEG * 4, true),
        (TSEG * 4 + nanosecond, false),
    ] {
        let mut table = SegmentedRequests::new(TSEG);
        open(&mut table, &Peer::new(1), start);
        assert!(
            table.expire(start + idle).is_empty(),
            "silence sends nothing"
        );
        assert_eq!(table.len() == 1, survives, "idle {idle:?}");
    }
}

#[test]
fn expiry_ends_only_what_is_due_and_gives_back_its_bytes() {
    let start = Instant::now();
    let now = start + PROGRESS_TIMEOUT;
    let mut table = SegmentedRequests::new(TSEG);
    let (stalled, fresh, idle) = (Peer::new(1), Peer::new(2), Peer::new(3));
    // Two segments saved at the start, heard from since, never progressing.
    open(&mut table, &stalled, start);
    assert!(matches!(
        stalled.send(&mut table, &segment(0, 1, 3, true, &[2; 8]), start),
        Receipt::Saved
    ));
    acked(
        stalled.send(&mut table, &segment(0, 5, 3, true, b"gap"), now),
        true,
        1,
        3,
    );
    // Saved a second short of the progress limit, and heard from since.
    open(&mut table, &fresh, now - Duration::from_secs(15));
    gap(&mut table, &fresh, now);
    // Progress is fresh, but silent a nanosecond past four segment timeouts.
    open(&mut table, &idle, now - TSEG * 4 - Duration::from_nanos(1));
    assert_eq!(table.saved_bytes(), 5 + 8 + 5 + 5);

    let expired = table.expire(now);
    assert_eq!(expired.len(), 1, "only the stalled one draws an Abort");
    assert_eq!(&expired[0].source_mac[..], &[1]);
    assert_eq!((table.len(), table.saved_bytes()), (1, 5));
    assert!(
        table.expire(now).is_empty(),
        "ending them again is harmless"
    );
    assert_eq!((table.len(), table.saved_bytes()), (1, 5));
    completed(
        fresh.send(&mut table, &segment(0, 1, 3, false, b""), now),
        1,
        b"first",
    );
}

#[test]
fn silence_wins_when_both_are_due_and_the_next_deadline_is_the_earliest() {
    let start = Instant::now();
    // Four segment timeouts of 4 s: both clocks run out at 16 s. At exactly
    // 16 s silence isn't past due yet, so progress ends it with an Abort;
    // a nanosecond later both are, and silence ends it without one.
    let four_seconds = Duration::from_secs(4);
    for (elapsed, aborts) in [
        (PROGRESS_TIMEOUT, 1),
        (PROGRESS_TIMEOUT + Duration::from_nanos(1), 0),
    ] {
        let mut table = SegmentedRequests::new(four_seconds);
        open(&mut table, &Peer::new(1), start);
        assert_eq!(table.next_deadline(), Some(start + PROGRESS_TIMEOUT));
        assert_eq!(table.expire(start + elapsed).len(), aborts);
        assert!(table.is_empty());
        assert_eq!(table.next_deadline(), None);
    }
    let mut table = SegmentedRequests::new(TSEG);
    open(&mut table, &Peer::new(1), start);
    assert_eq!(
        table.next_deadline(),
        Some(start + TSEG * 4 + Duration::from_nanos(1))
    );
}
