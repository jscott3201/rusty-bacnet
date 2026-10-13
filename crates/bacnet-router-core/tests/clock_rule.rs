//! The clock rule (#1684): a `now` earlier than the latest one passed in is
//! taken as that latest one, from `Router::new` on, by `receive` and by
//! `on_deadline`. Each test makes a call stamped earlier than the router's
//! latest time, at a moment where the stamp and the latest time lead to
//! different outcomes, and pins the outcome of the latest time.

mod fixture;

use bacnet_router_core::message::MessageType;
use bacnet_router_core::router::{Event, Kind, Options};
use bacnet_router_core::{PortId, Router};
use fixture::{
    apdu_to, at, deliver, deliver_broadcast, i_am, message, router, two_ports, Outcome, Recorder,
    Store, CONFIRMED, NODE_A, R7,
};

/// A route learned by a frame stamped before the router's start is
/// confirmed at the start: `new`'s time is the first latest time.
#[test]
fn a_frame_stamped_before_the_start_is_taken_at_the_start() {
    let mut router = Router::new(
        Box::new(Store::new()),
        &two_ports(),
        Options::HOSTED,
        at(100),
    )
    .unwrap();
    let mut host = Recorder::default();
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(0));
    assert_eq!(router.table().lookup(9).unwrap().last_seen(), Some(at(100)));
}

/// `receive` stamped early learns at the latest time, so the route lives
/// its full age from then: at 360 s it is 260 s old and stays, where at
/// its stamp's 0 s it would be 360 s old and go.
#[test]
fn a_frame_stamped_early_learns_at_the_latest_time() {
    let mut router = router();
    let mut host = Recorder::default();
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(100));
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(0));
    let route = router.table().lookup(9).unwrap();
    assert_eq!(route.last_seen(), Some(at(100)));
    host.events.clear();
    router.on_deadline(at(360), &mut host);
    assert!(router.table().lookup(9).is_some(), "260 s old: kept");
    assert!(host.events.is_empty());
}

/// `receive` stamped early reads a busy mark at the latest time: past the
/// mark's deadline, so the request is routed, not refused with reason 2.
#[test]
fn a_frame_stamped_early_reads_marks_at_the_latest_time() {
    let mut router = router();
    let mut host = Recorder::default();
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(0));
    // Busy until 130 s.
    let busy = message(MessageType::ROUTER_BUSY_TO_NETWORK, &[0, 9]);
    deliver_broadcast(&mut router, &mut host, 1, R7, &busy, at(100));
    let to_9 = apdu_to(9, &[5], CONFIRMED);
    let (_, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_9, at(140));
    assert_eq!(sent[0].kind, Kind::Forward);
    let (outcome, sent) = deliver(&mut router, &mut host, 0, NODE_A, &to_9, at(120));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].port, sent[0].kind),
        (1, Kind::Forward),
        "routed at 140 s, past the busy mark"
    );
}

/// `on_deadline` moves the clock too: a frame stamped earlier than the
/// sweep's time, after it, learns at the sweep's time. The route to 9,
/// learned at 0 s, is purged at 400 s and learned again then.
#[test]
fn a_frame_after_a_deadline_call_is_taken_at_the_deadline_calls_time() {
    let mut router = router();
    let mut host = Recorder::default();
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(0));
    router.on_deadline(at(400), &mut host);
    assert!(router.table().lookup(9).is_none(), "purged at 400 s");
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(10));
    assert_eq!(router.table().lookup(9).unwrap().last_seen(), Some(at(400)));
}

/// `on_deadline` stamped before the sweep is due runs the sweep when the
/// latest time has reached it: a frame at 400 s moved the clock, so a call
/// stamped 10 s purges the route last confirmed at 0 s and sets the next
/// sweep a period after 400 s.
#[test]
fn a_deadline_call_stamped_early_sweeps_at_the_latest_time() {
    let mut router = router();
    let mut host = Recorder::default();
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[9]), at(0));
    deliver_broadcast(&mut router, &mut host, 1, R7, &i_am(&[10]), at(400));
    assert_eq!(router.next_deadline(), Some(at(60)));
    host.events.clear();
    router.on_deadline(at(10), &mut host);
    assert_eq!(host.events, [Event::Purged { network: 9 }]);
    assert!(router.table().lookup(9).is_none());
    assert_eq!(router.table().lookup(10).unwrap().port(), PortId(1));
    assert_eq!(router.next_deadline(), Some(at(460)));
}
