//! What a transfer keeps and is charged for, and how one peer's share is
//! counted (#2742). Moved from the full server's own receiver, whose tests
//! these were (`request_payload_owner.rs`, `request_peer_quota_state.rs`);
//! they read the table's private state where the old ones read the server's.

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

/// A segment of a WriteProperty, with a header unlike [`segment`]'s.
fn write(invoke_id: u8, sequence: u8, window: u8, more: bool, data: Bytes) -> ConfirmedRequest {
    ConfirmedRequest {
        segmented: true,
        more_follows: more,
        segmented_response_accepted: false,
        max_segments: Some(16),
        max_apdu_length: 480,
        invoke_id,
        sequence_number: Some(sequence),
        proposed_window_size: Some(window),
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: data,
    }
}

/// Bytes that count their buffer's drop, so a test sees whether the table
/// kept the buffer a segment came in.
struct Counted {
    bytes: Vec<u8>,
    drops: Arc<AtomicUsize>,
}

impl AsRef<[u8]> for Counted {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.drops.fetch_add(1, SeqCst);
    }
}

fn counted(data: &[u8], drops: &Arc<AtomicUsize>) -> Bytes {
    Bytes::from_owner(Counted {
        bytes: data.to_vec(),
        drops: Arc::clone(drops),
    })
}

fn only(table: &SegmentedRequests) -> &Transfer {
    let mut transfers = table.transfers.values();
    let transfer = transfers.next().expect("one transfer");
    assert!(transfers.next().is_none(), "only one transfer");
    transfer
}

#[test]
fn each_segment_is_charged_once_and_the_request_keeps_the_first_header() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    let drops = Arc::new(AtomicUsize::new(0));
    let first = write(23, 0, 3, true, counted(b"first", &drops));
    acked(peer.send(&mut table, &first, now), false, 0, 3);
    assert_eq!(table.saved_bytes(), 5);
    assert!(
        only(&table).header.service_request.is_empty(),
        "the header keeps no data of its own"
    );
    drop(first);
    assert_eq!(
        drops.load(SeqCst),
        1,
        "a copy is kept, never the buffer the segment came in"
    );
    let last = write(23, 1, 3, false, counted(b"last", &drops));
    let request = completed(peer.send(&mut table, &last, now), 1, b"firstlast");
    drop(last);
    assert_eq!(drops.load(SeqCst), 2);
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));
    assert_eq!(request.invoke_id, 23);
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::WRITE_PROPERTY
    );
    assert_eq!(request.max_apdu_length, 480);
    assert_eq!(request.max_segments, Some(16));
    assert!(!request.segmented_response_accepted);
}

#[test]
fn a_refused_segment_is_neither_charged_nor_kept() {
    let now = Instant::now();
    let peer = Peer::new(1);
    let drops = Arc::new(AtomicUsize::new(0));
    let mut table = SegmentedRequests::new(TSEG);
    // Longer than a segment can be: refused before anything is kept.
    let oversized = write(23, 0, 3, true, counted(&[1; 1477], &drops));
    aborted(
        peer.send(&mut table, &oversized, now),
        AbortReason::BUFFER_OVERFLOW,
    );
    drop(oversized);
    assert_eq!(drops.load(SeqCst), 1);
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));

    let first = |data: &'static [u8]| write(23, 0, 3, true, Bytes::from_static(data));
    acked(peer.send(&mut table, &first(b"first"), now), false, 0, 3);
    // Segment zero again, then a gap: each is asked for again and changes
    // nothing saved.
    for sequence in [0, 2] {
        let again = write(23, sequence, 3, true, Bytes::from_static(b"again"));
        acked(peer.send(&mut table, &again, now), true, 0, 3);
        assert_eq!((table.len(), table.saved_bytes()), (1, 5));
        assert_eq!(only(&table).accepted, 1);
    }
    // Too long a next segment ends the transfer and gives back its bytes.
    let oversized = write(23, 1, 3, true, Bytes::from(vec![2; 1477]));
    aborted(
        peer.send(&mut table, &oversized, now),
        AbortReason::BUFFER_OVERFLOW,
    );
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));

    // A full budget refuses one byte more, but takes a segment of none.
    let mut full = small(|limits| limits.saved_bytes = 5);
    acked(peer.send(&mut full, &first(b"first"), now), false, 0, 3);
    let other = Peer::new(2);
    acked(
        other.send(&mut full, &write(1, 0, 3, true, Bytes::new()), now),
        false,
        0,
        3,
    );
    aborted(
        other.send(
            &mut full,
            &write(1, 1, 3, true, Bytes::from_static(b"x")),
            now,
        ),
        AbortReason::BUFFER_OVERFLOW,
    );
    assert_eq!((full.len(), full.saved_bytes()), (1, 5));
    completed(
        peer.send(&mut full, &write(23, 1, 3, false, Bytes::new()), now),
        1,
        b"first",
    );
    assert_eq!((full.len(), full.saved_bytes()), (0, 0));
}

#[test]
fn the_byte_budget_is_inclusive_and_its_sum_fails_closed() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let one = |data: &'static [u8]| write(1, 0, 1, true, Bytes::from_static(data));
    // Stand in for other transfers' segments, one byte short of the budget.
    table.saved_bytes = MAX_SAVED_BYTES - 1;
    acked(Peer::new(1).send(&mut table, &one(b"x"), now), false, 0, 1);
    assert_eq!(table.saved_bytes(), MAX_SAVED_BYTES);
    aborted(
        Peer::new(2).send(&mut table, &one(b"y"), now),
        AbortReason::BUFFER_OVERFLOW,
    );
    acked(Peer::new(3).send(&mut table, &one(b""), now), false, 0, 1);
    assert_eq!((table.len(), table.saved_bytes()), (2, MAX_SAVED_BYTES));
    // A sum past the integer's range is refused, never wrapped to fit.
    table.saved_bytes = usize::MAX;
    aborted(
        Peer::new(4).send(&mut table, &one(b"z"), now),
        AbortReason::BUFFER_OVERFLOW,
    );
    assert_eq!((table.len(), table.saved_bytes()), (2, usize::MAX));
}

#[test]
fn a_request_missing_a_segment_when_it_ends_is_aborted_and_gives_back_its_bytes() {
    let now = Instant::now();
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    acked(
        peer.send(
            &mut table,
            &write(1, 0, 1, true, Bytes::from_static(b"first")),
            now,
        ),
        false,
        0,
        1,
    );
    // Count a segment never saved, which `receive` can't do: it saves
    // strictly in order.
    table.transfers.values_mut().next().expect("open").accepted += 1;
    aborted(
        peer.send(
            &mut table,
            &write(1, 1, 1, false, Bytes::from_static(b"last")),
            now,
        ),
        AbortReason::OTHER,
    );
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));
    // Ending every transfer keeps nothing either.
    peer.send(
        &mut table,
        &write(2, 0, 1, true, Bytes::from_static(b"kept")),
        now,
    );
    assert_eq!(table.saved_bytes(), 4);
    table.clear();
    assert_eq!((table.len(), table.saved_bytes()), (0, 0));
}

#[test]
fn expiry_gives_back_the_bytes_of_what_it_ends_only() {
    for progress in [false, true] {
        let start = Instant::now();
        let now = if progress {
            start + PROGRESS_TIMEOUT
        } else {
            start + TSEG * 4 + Duration::from_nanos(1)
        };
        let mut table = SegmentedRequests::new(TSEG);
        let (stale, fresh) = (Peer::new(1), Peer::new(2));
        let first = |data: &'static [u8]| write(1, 0, 3, true, Bytes::from_static(data));
        acked(stale.send(&mut table, &first(b"stale"), start), false, 0, 3);
        if progress {
            // Heard from a second ago, with nothing new saved since.
            let gap = write(1, 2, 3, true, Bytes::from_static(b"gap"));
            let recent = now - Duration::from_secs(1);
            acked(stale.send(&mut table, &gap, recent), true, 0, 3);
        }
        acked(fresh.send(&mut table, &first(b"fresh"), now), false, 0, 3);
        assert_eq!(table.saved_bytes(), 10);
        assert_eq!(table.expire(now).len(), usize::from(progress));
        assert_eq!((table.len(), table.saved_bytes()), (1, 5));
        assert!(table.expire(now).is_empty(), "nothing more is due");
        assert_eq!((table.len(), table.saved_bytes()), (1, 5));
        table.clear();
        assert_eq!(table.saved_bytes(), 0);
    }
}

#[test]
fn duplicates_and_gaps_leave_progress_and_saved_segments_alone() {
    let start = Instant::now();
    let later = start + Duration::from_secs(1);
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    let part = |sequence, more, data: &'static [u8]| {
        write(23, sequence, 3, more, Bytes::from_static(data))
    };
    acked(
        peer.send(&mut table, &part(0, true, b"first"), start),
        false,
        0,
        3,
    );
    assert!(matches!(
        peer.send(&mut table, &part(1, true, b"second"), start),
        Receipt::Saved
    ));
    // Three duplicates are a window's allowance; the fourth, and then a
    // gap, are asked for again from segment 1.
    for (sequence, negative) in [(1, false), (1, false), (1, false), (1, true), (5, true)] {
        let receipt = peer.send(&mut table, &part(sequence, true, b"again"), later);
        if negative {
            acked(receipt, true, 1, 3);
        } else {
            assert!(matches!(receipt, Receipt::Discarded), "{receipt:?}");
        }
        let transfer = only(&table);
        assert_eq!(
            (transfer.last_progress, transfer.last_activity),
            (start, later)
        );
        assert_eq!((transfer.accepted, transfer.expected_sequence), (2, 2));
        assert_eq!(table.saved_bytes(), 11);
    }
    completed(
        peer.send(&mut table, &part(2, false, b""), later),
        2,
        b"firstsecond",
    );
}

fn send_from(
    table: &mut SegmentedRequests,
    mac: &[u8],
    source_network: Option<&NpduAddress>,
    request: &ConfirmedRequest,
    now: Instant,
) -> Receipt {
    table.receive(
        Segment {
            source_mac: mac,
            source_network,
            route: &ResponseRoute::unverified(),
            request,
        },
        now,
    )
}

#[test]
fn a_peer_is_its_address_or_its_routed_address_and_fills_only_its_own_share() {
    let a: &[u8] = &[127, 0, 0, 1, 0xBA, 0xC0];
    let b: &[u8] = &[127, 0, 0, 2, 0xBA, 0xC0];
    let routed = |network, byte: u8| {
        Some(NpduAddress {
            network,
            mac_address: MacAddr::from_slice(&[byte, byte + 1, byte + 2]),
        })
    };
    let empty = |network| {
        Some(NpduAddress {
            network,
            mac_address: MacAddr::new(),
        })
    };
    // Named groups are the oracle, the invalid fallback partitions among
    // them; the flag says whether the key leaves the router's address out.
    let sources = [
        ("direct-a", a, None, false),
        ("direct-b", b, None, false),
        ("routed", a, routed(400, 10), true),
        ("routed", b, routed(400, 10), true),
        ("other-address", a, routed(400, 11), true),
        ("other-network", a, routed(401, 10), true),
        ("min-network", a, routed(1, 10), true),
        ("min-network", b, routed(1, 10), true),
        ("max-network", a, routed(0xFFFE, 10), true),
        ("max-network", b, routed(0xFFFE, 10), true),
        ("zero-net-a", a, routed(0, 10), false),
        ("zero-net-b", b, routed(0, 10), false),
        ("zero-net-other-address", a, routed(0, 11), false),
        ("global-net-a", a, routed(0xFFFF, 10), false),
        ("global-net-b", b, routed(0xFFFF, 10), false),
        ("empty-valid-a", a, empty(400), false),
        ("empty-valid-b", b, empty(400), false),
        ("empty-zero", a, empty(0), false),
        ("empty-global", a, empty(0xFFFF), false),
    ];
    let now = Instant::now();
    let unverified = TransportProvenance::unverified;
    for (group, mac, source, drops_router) in &sources {
        let mut table = SegmentedRequests::new(TSEG);
        for invoke_id in 0..16 {
            let expected_mac = if *drops_router {
                MacAddr::new()
            } else {
                MacAddr::from_slice(mac)
            };
            assert_eq!(
                transfer_key(mac, source.as_ref(), invoke_id, unverified()),
                (expected_mac, source.clone(), invoke_id, unverified()),
                "{group}"
            );
            let first = segment(invoke_id, 0, 1, true, b"a");
            acked(
                send_from(&mut table, mac, source.as_ref(), &first, now),
                false,
                0,
                1,
            );
        }
        for (query_group, query_mac, query_source, _) in &sources {
            let first = segment(200, 0, 1, true, b"q");
            let receipt = send_from(&mut table, query_mac, query_source.as_ref(), &first, now);
            if group == query_group {
                aborted(receipt, AbortReason::OUT_OF_RESOURCES);
            } else {
                acked(receipt, false, 0, 1);
                table.abort(query_mac, query_source.as_ref(), 200, unverified());
            }
            assert_eq!(table.len(), 16, "filled {group}, asked by {query_group}");
        }
    }
}

#[test]
fn refused_new_transfers_leave_a_peers_open_ones_as_they_were() {
    let start = Instant::now();
    let later = start + Duration::from_secs(1);
    let mut table = SegmentedRequests::new(TSEG);
    let peer = Peer::new(1);
    for invoke_id in 0..16 {
        let first = segment(invoke_id, 0, 1, true, b"retained");
        acked(peer.send(&mut table, &first, start), false, 0, 1);
    }
    for invoke_id in 16..=255 {
        let first = segment(invoke_id, 0, 1, true, b"refused");
        aborted(
            peer.send(&mut table, &first, later),
            AbortReason::OUT_OF_RESOURCES,
        );
    }
    assert_eq!((table.len(), table.saved_bytes()), (16, 16 * 8));
    for transfer in table.transfers.values() {
        assert_eq!(
            (transfer.last_activity, transfer.last_progress),
            (start, start)
        );
        assert_eq!(
            (
                transfer.accepted,
                transfer.expected_sequence,
                transfer.initial_sequence,
                transfer.last_sequence,
                transfer.duplicates,
                transfer.window,
                transfer.saved_bytes,
            ),
            (1, 1, 0, 0, 0, 1, 8)
        );
    }
    for invoke_id in 0..16 {
        let last = segment(invoke_id, 1, 1, false, b"");
        let request = completed(peer.send(&mut table, &last, later), 1, b"retained");
        assert_eq!(request.invoke_id, invoke_id);
    }
}

#[test]
fn expiry_frees_a_peers_share_at_each_boundary() {
    for progress in [false, true] {
        let start = Instant::now();
        let now = if progress {
            start + PROGRESS_TIMEOUT
        } else {
            start + TSEG * 4 + Duration::from_nanos(1)
        };
        let recent = now - Duration::from_secs(1);
        let just_before = now - Duration::from_nanos(1);
        let mut table = SegmentedRequests::new(TSEG);
        let peer = Peer::new(1);
        acked(
            peer.send(&mut table, &segment(0, 0, 1, true, b"stale"), start),
            false,
            0,
            1,
        );
        if progress {
            let gap = segment(0, 2, 1, true, b"gap");
            acked(peer.send(&mut table, &gap, recent), true, 0, 1);
        }
        for invoke_id in 1..16 {
            let first = segment(invoke_id, 0, 1, true, b"live");
            acked(peer.send(&mut table, &first, recent), false, 0, 1);
        }
        assert!(table.expire(just_before).is_empty());
        aborted(
            peer.send(&mut table, &segment(200, 0, 1, true, b"q"), just_before),
            AbortReason::OUT_OF_RESOURCES,
        );
        assert_eq!(table.expire(now).len(), usize::from(progress));
        assert_eq!(table.len(), 15);
        assert!(table.expire(now).is_empty());
        acked(
            peer.send(&mut table, &segment(200, 0, 1, true, b"q"), now),
            false,
            0,
            1,
        );
        aborted(
            peer.send(&mut table, &segment(201, 0, 1, true, b"q"), now),
            AbortReason::OUT_OF_RESOURCES,
        );
    }
}
