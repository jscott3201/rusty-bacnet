//! The tracker's side of a request that comes during a segmented answer
//! (#2770), with a handle standing in for the sending task. The wire tests
//! are in `segmentation_tests/request_during_answer.rs`.

use super::*;
use crate::server::segmented_send::SegmentedSendControlEvent;
use bacnet_encoding::apdu::AbortPdu;
use bacnet_types::MacAddr;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bytes::Bytes;
use std::sync::atomic::Ordering;
use tokio::sync::{mpsc, watch};

type Control = watch::Receiver<Option<SegmentedSendControlEvent>>;

fn request(invoke_id: u8, body: &'static [u8]) -> ConfirmedRequest {
    ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: Some(4),
        max_apdu_length: 480,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::from_static(body),
    }
}

fn new(admission: ConfirmedRequestAdmission) -> PendingConfirmedRequest {
    match admission {
        ConfirmedRequestAdmission::New(pending) => pending,
        _ => panic!("expected new"),
    }
}

fn begin(
    tracker: &Arc<ConfirmedRequestTracker>,
    request: ConfirmedRequest,
) -> ConfirmedRequestAdmission {
    tracker.begin(b"peer", None, TransportProvenance::unverified(), request)
}

fn duplicate(admission: ConfirmedRequestAdmission) -> bool {
    matches!(admission, ConfirmedRequestAdmission::Duplicate)
}

fn aborts(admission: ConfirmedRequestAdmission) -> bool {
    matches!(admission, ConfirmedRequestAdmission::AbortsAnswer)
}

/// A send task's handle for `request`'s entry, not yet noted.
fn handle(request: Option<u64>) -> (Arc<SegmentedSendHandle>, Control) {
    let (tx, _rx) = mpsc::channel(1);
    let (control_tx, control_rx) = watch::channel(None);
    let handle = SegmentedSendHandle::new(tx, control_tx, 3, request);
    (Arc::new(handle), control_rx)
}

/// `pending` answered in segments, as the send task notes it.
fn answer(pending: &PendingConfirmedRequest) -> (Arc<SegmentedSendHandle>, Control) {
    let (handle, control) = handle(pending.id());
    pending.answering(&handle);
    (handle, control)
}

fn closed(handle: &SegmentedSendHandle) -> bool {
    handle.closed.load(Ordering::Acquire)
}

#[test]
fn a_different_request_ends_the_answer_and_frees_its_invoke_id() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let answered = request(4, b"answered");
    let other = request(4, b"other");
    let pending = new(begin(&tracker, answered.clone()));
    let (handle, control) = answer(&pending);
    assert!(
        duplicate(begin(&tracker, answered.clone())),
        "the same again"
    );
    assert!(!closed(&handle));
    assert!(aborts(begin(&tracker, other.clone())));
    assert!(closed(&handle));
    assert!(matches!(
        *control.borrow(),
        Some(SegmentedSendControlEvent::Supersede)
    ));
    assert!(handle.finish(), "the task owes the Abort");
    // The entry is over: either request is new, and the late guard of the
    // answered one ends neither.
    let other_again = new(begin(&tracker, other.clone()));
    let answered_again = new(begin(&tracker, answered.clone()));
    drop(pending);
    for body in [other, answered] {
        assert!(duplicate(begin(&tracker, body)));
    }
    drop((other_again, answered_again));
    assert!(tracker.state.lock().unwrap().entries.is_empty());
}

/// The service choice is part of the request.
#[test]
fn the_same_data_under_another_service_ends_the_answer() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let answered = request(4, b"same data");
    let pending = new(begin(&tracker, answered.clone()));
    let (handle, _control) = answer(&pending);
    let mut other = answered;
    other.service_choice = ConfirmedServiceChoice::WRITE_PROPERTY;
    assert!(aborts(begin(&tracker, other)));
    assert!(closed(&handle));
}

/// Only the answer's own requester, in its trust context and under its
/// invoke ID, ends it; a routed requester is the same through any router.
#[test]
fn only_a_request_under_the_answers_key_ends_it() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let unverified = TransportProvenance::unverified;
    let routed = |network| NpduAddress {
        network,
        mac_address: MacAddr::from_slice(b"peer"),
    };
    let other = request(4, b"other");
    let pending = new(tracker.begin(
        b"router",
        Some(&routed(5)),
        unverified(),
        request(4, b"answered"),
    ));
    let (handle, _control) = answer(&pending);
    for (what, admission) in [
        (
            "the address on another network",
            tracker.begin(b"router", Some(&routed(6)), unverified(), other.clone()),
        ),
        (
            "the address on the server's own network",
            tracker.begin(b"peer", None, unverified(), other.clone()),
        ),
        (
            "the router itself",
            tracker.begin(b"router", None, unverified(), other.clone()),
        ),
        (
            "another invoke ID",
            tracker.begin(b"router", Some(&routed(5)), unverified(), request(5, b"x")),
        ),
    ] {
        assert!(
            matches!(admission, ConfirmedRequestAdmission::New(_)),
            "{what}"
        );
        assert!(!closed(&handle), "{what}");
    }
    assert!(!tracker.supersede_answer(b"peer", None, &unverified(), 4));
    assert!(!tracker.supersede_answer(b"router", Some(&routed(5)), &unverified(), 5));
    assert!(!closed(&handle));
    let through_another_router = tracker.begin(b"router-b", Some(&routed(5)), unverified(), other);
    assert!(aborts(through_another_router));
    assert!(closed(&handle));
}

/// An answer whose task sends no more (its last segment sent, or given up),
/// or one already closed (cancelled by a newer answer, aborted by the
/// requester), is not in progress: a different request is admitted as new,
/// and the entry stays to its guard.
#[test]
fn an_answer_no_longer_sending_is_left_alone() {
    let requester_abort = SegmentedSendControlEvent::Abort(AbortPdu {
        sent_by_server: false,
        invoke_id: 4,
        abort_reason: AbortReason::OTHER,
    });
    for stop in ["finished", "cancelled", "aborted"] {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let answered = request(4, b"answered");
        let pending = new(begin(&tracker, answered.clone()));
        let (handle, _control) = answer(&pending);
        match stop {
            "finished" => assert!(!handle.finish()),
            "cancelled" => handle.send_control(SegmentedSendControlEvent::Cancel),
            _ => handle.send_control(requester_abort.clone()),
        }
        let other = new(begin(&tracker, request(4, b"other")));
        assert!(
            duplicate(begin(&tracker, answered)),
            "{stop}: its entry stays"
        );
        assert!(!handle.finish(), "{stop}: no Abort owed");
        drop((other, pending));
    }
}

/// While a request still executes, before any answer, a different one
/// under its invoke ID is admitted as new, as before (#2751).
#[test]
fn a_request_still_executing_has_no_answer_to_end() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let pending = new(begin(&tracker, request(4, b"executing")));
    let other = new(begin(&tracker, request(4, b"other")));
    drop((other, pending));
}

/// Two pending requests under one key, each answered: the one still
/// sending is ended, newest first. A newer answer cancels the older, so
/// once that lands the newest is the one; until it lands, the older one
/// may be the one sending.
#[test]
fn the_answer_still_sending_is_the_one_ended() {
    for newest_cancelled in [false, true] {
        let tracker = Arc::new(ConfirmedRequestTracker::default());
        let first = new(begin(&tracker, request(4, b"first")));
        let second = new(begin(&tracker, request(4, b"second")));
        let (older, _older) = answer(&second);
        let (newest, _newest) = answer(&first);
        if newest_cancelled {
            newest.send_control(SegmentedSendControlEvent::Cancel);
        }
        assert!(aborts(begin(&tracker, request(4, b"third"))));
        // Only the one ended owes the Abort, and the other's entry stays.
        assert_eq!(
            (newest.finish(), older.finish()),
            (!newest_cancelled, newest_cancelled)
        );
        let kept: &'static [u8] = if newest_cancelled {
            b"first"
        } else {
            b"second"
        };
        assert!(duplicate(begin(&tracker, request(4, kept))));
        drop((first, second));
    }
}

/// A note whose task has ended (its handle gone) ends nothing: the
/// request is new.
#[test]
fn a_note_whose_task_is_gone_ends_nothing() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let pending = new(begin(&tracker, request(4, b"answered")));
    drop(answer(&pending));
    let other = new(begin(&tracker, request(4, b"other")));
    assert!(duplicate(begin(&tracker, request(4, b"answered"))));
    drop((other, pending));
}

/// The Abort is owed once, by the task when it stops, only when the answer
/// was ended while it still sent.
#[test]
fn the_abort_is_owed_once_and_only_by_an_answer_ended_while_sending() {
    let (sending, _control) = handle(None);
    assert!(sending.supersede());
    assert!(!sending.supersede(), "ended once");
    assert!(sending.finish());
    assert!(!sending.finish(), "owed once");
    let (stopped, control) = handle(None);
    assert!(!stopped.finish());
    assert!(!stopped.supersede());
    assert!(!closed(&stopped) && control.borrow().is_none());
}

/// While its answer goes out in segments, a segment zero under a request
/// that came in segments draws no note: it opens a transfer. Any other
/// segment, or any segment while the request still executes, draws it.
#[test]
fn during_its_answer_a_segment_zero_draws_no_note() {
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let route = bacnet_network::response_route::ResponseRoute::unverified();
    let unverified = TransportProvenance::unverified();
    let admitted = tracker.next_admission();
    let pending = new(begin(&tracker, request(3, b"in segments")));
    tracker.note_segmented(
        admitted,
        b"peer",
        None,
        unverified.clone(),
        route.clone(),
        3,
        9,
        2,
    );
    let repeat = |opening| tracker.repeated_segment(b"peer", None, &unverified, &route, 3, opening);
    assert_eq!(repeat(true), Some((9, 2)), "while it executes");
    let (_handle, _control) = answer(&pending);
    assert_eq!(repeat(true), None, "segment zero opens a transfer");
    assert_eq!(repeat(false), Some((9, 2)), "another segment is a repeat");
    drop(pending);
}
