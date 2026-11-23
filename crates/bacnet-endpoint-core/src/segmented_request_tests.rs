//! The receiver's transitions, limits and timers, without I/O (#2646).

use super::*;
use bacnet_types::enums::ConfirmedServiceChoice;

const TSEG: Duration = Duration::from_secs(1);

fn segment(invoke_id: u8, sequence: u8, window: u8, more: bool, data: &[u8]) -> ConfirmedRequest {
    ConfirmedRequest {
        segmented: true,
        more_follows: more,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id,
        sequence_number: Some(sequence),
        proposed_window_size: Some(window),
        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
        service_request: Bytes::copy_from_slice(data),
    }
}

struct Peer {
    mac: Vec<u8>,
    route: ResponseRoute,
}

impl Peer {
    fn new(mac: u8) -> Self {
        Self {
            mac: vec![mac],
            route: ResponseRoute::unverified(),
        }
    }

    fn send(
        &self,
        table: &mut SegmentedRequests,
        request: &ConfirmedRequest,
        now: Instant,
    ) -> Receipt {
        table.receive(
            Segment {
                source_mac: &self.mac,
                source_network: None,
                route: &self.route,
                request,
            },
            now,
        )
    }
}

#[track_caller]
fn acked(receipt: Receipt, negative: bool, sequence: u8, window: u8) {
    let Receipt::Ack(ack) = receipt else {
        panic!("expected a Segment-ACK, got {receipt:?}");
    };
    assert!(ack.sent_by_server, "the answering side sets the server bit");
    assert_eq!(
        (
            ack.negative_ack,
            ack.sequence_number,
            ack.actual_window_size
        ),
        (negative, sequence, window)
    );
}

#[track_caller]
fn aborted(receipt: Receipt, reason: AbortReason) {
    let Receipt::Abort(got) = receipt else {
        panic!("expected an Abort, got {receipt:?}");
    };
    assert_eq!(got, reason);
}

#[track_caller]
fn completed(receipt: Receipt, last: u8, data: &[u8]) -> ConfirmedRequest {
    let Receipt::Complete(done) = receipt else {
        panic!("expected the whole request, got {receipt:?}");
    };
    assert!(done.ack.sent_by_server && !done.ack.negative_ack);
    assert_eq!(done.ack.sequence_number, last);
    let request = done.request;
    assert!(!request.segmented && !request.more_follows);
    assert_eq!(request.sequence_number, None);
    assert_eq!(request.proposed_window_size, None);
    assert_eq!(&request.service_request[..], data);
    request
}

#[test]
fn window_one_acknowledges_every_segment_and_completes_once() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    acked(
        peer.send(&mut table, &segment(5, 0, 1, true, b"ab"), now),
        false,
        0,
        1,
    );
    acked(
        peer.send(&mut table, &segment(5, 1, 1, true, b"cd"), now),
        false,
        1,
        1,
    );
    let request = completed(
        peer.send(&mut table, &segment(5, 2, 1, false, b"e"), now),
        2,
        b"abcde",
    );
    assert_eq!(request.invoke_id, 5);
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE
    );
    assert!(table.is_empty());
    assert_eq!(table.saved_bytes(), 0);
}

#[test]
fn segment_zero_is_acknowledged_at_once_then_each_full_window() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    // Segment zero alone, as a conforming requester sends it, with the
    // window it proposed.
    acked(
        peer.send(&mut table, &segment(6, 0, 3, true, b"0"), now),
        false,
        0,
        3,
    );
    for sequence in 1..=2 {
        assert!(matches!(
            peer.send(&mut table, &segment(6, sequence, 3, true, b"x"), now),
            Receipt::Saved
        ));
    }
    acked(
        peer.send(&mut table, &segment(6, 3, 3, true, b"3"), now),
        false,
        3,
        3,
    );
    assert!(matches!(
        peer.send(&mut table, &segment(6, 4, 3, true, b"4"), now),
        Receipt::Saved
    ));
    // The last segment is acknowledged wherever the window stands.
    completed(
        peer.send(&mut table, &segment(6, 5, 3, false, b"5"), now),
        5,
        b"0xx345",
    );
    assert_eq!(table.saved_bytes(), 0);
}

#[test]
fn a_one_segment_request_completes_on_segment_zero() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    completed(
        Peer::new(1).send(&mut table, &segment(7, 0, 4, false, b"all"), now),
        0,
        b"all",
    );
    assert!(table.is_empty());
}

#[test]
fn duplicates_are_discarded_up_to_a_window_then_answered_with_a_negative_ack() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    acked(
        peer.send(&mut table, &segment(8, 0, 3, true, b"0"), now),
        false,
        0,
        3,
    );
    peer.send(&mut table, &segment(8, 1, 3, true, b"1"), now);
    peer.send(&mut table, &segment(8, 2, 3, true, b"2"), now);
    for _ in 0..3 {
        assert!(matches!(
            peer.send(&mut table, &segment(8, 1, 3, true, b"1"), now),
            Receipt::Discarded
        ));
    }
    acked(
        peer.send(&mut table, &segment(8, 2, 3, true, b"2"), now),
        true,
        2,
        3,
    );
    // Nothing a duplicate carried was kept.
    assert_eq!(table.saved_bytes(), 3);
    // The window counts again from the negative acknowledgment.
    for sequence in 3..=4 {
        assert!(matches!(
            peer.send(&mut table, &segment(8, sequence, 3, true, b"x"), now),
            Receipt::Saved
        ));
    }
    acked(
        peer.send(&mut table, &segment(8, 5, 3, true, b"5"), now),
        false,
        5,
        3,
    );
    completed(
        peer.send(&mut table, &segment(8, 6, 3, false, b"6"), now),
        6,
        b"012xx56",
    );
}

#[test]
fn a_gap_draws_a_negative_ack_for_the_last_segment_saved() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    acked(
        peer.send(&mut table, &segment(9, 0, 3, true, b"0"), now),
        false,
        0,
        3,
    );
    acked(
        peer.send(&mut table, &segment(9, 2, 3, true, b"2"), now),
        true,
        0,
        3,
    );
    // The requester goes again from segment 1, and the window counts from
    // the negative acknowledgment.
    peer.send(&mut table, &segment(9, 1, 3, true, b"1"), now);
    peer.send(&mut table, &segment(9, 2, 3, true, b"2"), now);
    acked(
        peer.send(&mut table, &segment(9, 3, 3, true, b"3"), now),
        false,
        3,
        3,
    );
    completed(
        peer.send(&mut table, &segment(9, 4, 3, false, b"4"), now),
        4,
        b"01234",
    );
}

#[test]
fn segment_zero_again_while_its_transfer_runs_is_a_duplicate_or_a_gap() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    acked(
        peer.send(&mut table, &segment(10, 0, 2, true, b"0"), now),
        false,
        0,
        2,
    );
    // Its acknowledgment was lost: segment zero comes again, and the
    // negative acknowledgment of segment zero moves the requester on.
    acked(
        peer.send(&mut table, &segment(10, 0, 2, true, b"0"), now),
        true,
        0,
        2,
    );
    peer.send(&mut table, &segment(10, 1, 2, true, b"1"), now);
    assert!(matches!(
        peer.send(&mut table, &segment(10, 0, 2, true, b"new"), now),
        Receipt::Discarded
    ));
    assert_eq!(table.len(), 1);
    completed(
        peer.send(&mut table, &segment(10, 2, 2, false, b"2"), now),
        2,
        b"012",
    );
}

#[test]
fn a_segment_with_no_transfer_or_a_bad_window_is_aborted() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    aborted(
        peer.send(&mut table, &segment(11, 1, 1, true, b"1"), now),
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
    for window in [0, 128] {
        aborted(
            peer.send(&mut table, &segment(11, 0, window, true, b"0"), now),
            AbortReason::WINDOW_SIZE_OUT_OF_RANGE,
        );
    }
    assert!(table.is_empty());
}

fn small(limits: impl FnOnce(&mut Limits)) -> SegmentedRequests {
    let mut fixed = Limits::FIXED;
    limits(&mut fixed);
    SegmentedRequests::with_limits(TSEG, fixed)
}

#[test]
fn transfers_at_once_and_from_one_peer_are_capped() {
    let now = Instant::now();
    let mut table = small(|limits| {
        limits.transfers = 3;
        limits.per_peer = 2;
    });
    let (one, two, three) = (Peer::new(1), Peer::new(2), Peer::new(3));
    acked(
        one.send(&mut table, &segment(1, 0, 1, true, b"a"), now),
        false,
        0,
        1,
    );
    acked(
        one.send(&mut table, &segment(2, 0, 1, true, b"a"), now),
        false,
        0,
        1,
    );
    aborted(
        one.send(&mut table, &segment(3, 0, 1, true, b"a"), now),
        AbortReason::OUT_OF_RESOURCES,
    );
    acked(
        two.send(&mut table, &segment(1, 0, 1, true, b"a"), now),
        false,
        0,
        1,
    );
    aborted(
        three.send(&mut table, &segment(1, 0, 1, true, b"a"), now),
        AbortReason::BUFFER_OVERFLOW,
    );
    // A transfer in progress still takes its segments at the cap.
    acked(
        one.send(&mut table, &segment(1, 1, 1, true, b"b"), now),
        false,
        1,
        1,
    );
    completed(
        one.send(&mut table, &segment(1, 2, 1, false, b"c"), now),
        2,
        b"abc",
    );
    acked(
        three.send(&mut table, &segment(1, 0, 1, true, b"a"), now),
        false,
        0,
        1,
    );
}

#[test]
fn the_byte_budget_ends_the_transfer_that_would_pass_it() {
    let now = Instant::now();
    let mut table = small(|limits| limits.saved_bytes = 10);
    let (one, two) = (Peer::new(1), Peer::new(2));
    acked(
        one.send(&mut table, &segment(1, 0, 1, true, b"1234"), now),
        false,
        0,
        1,
    );
    acked(
        two.send(&mut table, &segment(1, 0, 1, true, b"1234"), now),
        false,
        0,
        1,
    );
    assert_eq!(table.saved_bytes(), 8);
    aborted(
        two.send(&mut table, &segment(1, 1, 1, true, b"123"), now),
        AbortReason::BUFFER_OVERFLOW,
    );
    assert_eq!((table.len(), table.saved_bytes()), (1, 4));
    // A first segment past it is refused and keeps nothing.
    let mut full = small(|limits| limits.saved_bytes = 2);
    aborted(
        one.send(&mut full, &segment(1, 0, 1, true, b"123"), now),
        AbortReason::BUFFER_OVERFLOW,
    );
    assert_eq!((full.len(), full.saved_bytes()), (0, 0));
}

#[test]
fn every_sequence_number_once_reassembles_and_one_more_is_refused() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    acked(
        peer.send(&mut table, &segment(1, 0, 127, true, &[0]), now),
        false,
        0,
        127,
    );
    let mut expected = vec![0u8];
    for count in 1..MAX_SEGMENTS {
        let sequence = count as u8;
        expected.push(sequence);
        peer.send(
            &mut table,
            &segment(1, sequence, 127, true, &[sequence]),
            now,
        );
    }
    // The 257th segment would land on segment zero's slot.
    aborted(
        peer.send(&mut table, &segment(1, 0, 127, true, &[0]), now),
        AbortReason::BUFFER_OVERFLOW,
    );
    assert!(table.is_empty());

    for (count, byte) in expected.iter().enumerate().take(MAX_SEGMENTS - 1) {
        let sequence = count as u8;
        peer.send(&mut table, &segment(2, sequence, 127, true, &[*byte]), now);
    }
    let last = (MAX_SEGMENTS - 1) as u8;
    completed(
        peer.send(&mut table, &segment(2, last, 127, false, &[last]), now),
        last,
        &expected,
    );
}

#[test]
fn silence_past_four_segment_timeouts_ends_a_transfer_without_a_word() {
    let start = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    acked(
        peer.send(&mut table, &segment(1, 0, 1, true, b"a"), start),
        false,
        0,
        1,
    );
    let deadline = start + TSEG * 4 + Duration::from_nanos(1);
    assert_eq!(table.next_deadline(), Some(deadline));
    assert!(table.expire(start + TSEG * 4).is_empty());
    assert_eq!(
        table.len(),
        1,
        "exactly four segment timeouts is not past them"
    );
    assert!(table.expire(deadline).is_empty(), "no Abort for silence");
    assert!(table.is_empty());
    assert_eq!(table.saved_bytes(), 0);
    assert_eq!(table.next_deadline(), None);
    // What comes after it finds no transfer.
    aborted(
        peer.send(&mut table, &segment(1, 1, 1, true, b"b"), deadline),
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
}

#[test]
fn a_transfer_that_only_repeats_itself_ends_with_an_abort() {
    let start = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    acked(
        peer.send(&mut table, &segment(1, 0, 2, true, b"a"), start),
        false,
        0,
        2,
    );
    let mut now = start;
    while now < start + PROGRESS_TIMEOUT {
        now += Duration::from_millis(500);
        // A gap each time: the segment timer restarts, nothing is saved.
        peer.send(&mut table, &segment(1, 2, 2, true, b"c"), now);
        let expired = table.expire(now);
        if now < start + PROGRESS_TIMEOUT {
            assert!(expired.is_empty());
        } else {
            assert_eq!(expired.len(), 1);
            assert_eq!(expired[0].invoke_id, 1);
            assert_eq!(&expired[0].source_mac[..], &[1]);
        }
    }
    assert!(table.is_empty());
    assert_eq!(table.saved_bytes(), 0);
}

#[test]
fn a_requester_abort_ends_its_transfer_only() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    peer.send(&mut table, &segment(1, 0, 1, true, b"a"), now);
    peer.send(&mut table, &segment(2, 0, 1, true, b"b"), now);
    table.abort(&[1], None, 1, TransportProvenance::unverified());
    table.abort(&[9], None, 2, TransportProvenance::unverified());
    assert_eq!((table.len(), table.saved_bytes()), (1, 1));
    aborted(
        peer.send(&mut table, &segment(1, 1, 1, true, b"b"), now),
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
    completed(
        peer.send(&mut table, &segment(2, 1, 1, false, b"c"), now),
        1,
        b"bc",
    );
    table.clear();
    assert_eq!(table.saved_bytes(), 0);
}

#[test]
fn a_routed_peer_keeps_one_transfer_whichever_router_brings_it() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let behind = NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[0x42]),
    };
    let route = ResponseRoute::unverified();
    let request = |sequence, more, data: &'static [u8]| segment(3, sequence, 1, more, data);
    let first = request(0, true, b"a");
    let receipt = table.receive(
        Segment {
            source_mac: &[1],
            source_network: Some(&behind),
            route: &route,
            request: &first,
        },
        now,
    );
    acked(receipt, false, 0, 1);
    let last = request(1, false, b"b");
    let receipt = table.receive(
        Segment {
            source_mac: &[2],
            source_network: Some(&behind),
            route: &route,
            request: &last,
        },
        now,
    );
    let Receipt::Complete(done) = receipt else {
        panic!("expected the whole request");
    };
    // Its answer goes back the way the first segment came.
    assert_eq!(&done.opening.source_mac[..], &[1]);
    assert_eq!(done.opening.source_network, Some(behind));
}

#[test]
fn a_stated_segment_count_caps_a_request_but_never_past_the_sequence_space() {
    let now = Instant::now();
    let peer = Peer::new(1);
    let mut table = SegmentedRequests::new(TSEG).with_max_segments(2);
    acked(
        peer.send(&mut table, &segment(1, 0, 1, true, b"a"), now),
        false,
        0,
        1,
    );
    acked(
        peer.send(&mut table, &segment(1, 1, 1, true, b"b"), now),
        false,
        1,
        1,
    );
    aborted(
        peer.send(&mut table, &segment(1, 2, 1, false, b"c"), now),
        AbortReason::BUFFER_OVERFLOW,
    );
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));
    let wide = SegmentedRequests::new(TSEG).with_max_segments(1000);
    assert_eq!(wide.limits.segments, MAX_SEGMENTS);
    let none = SegmentedRequests::new(TSEG).with_max_segments(0);
    assert_eq!(none.limits.segments, 1);
}

#[path = "segmented_request_budget_tests.rs"]
mod budget;

#[path = "segmented_request_timer_tests.rs"]
mod timers;
