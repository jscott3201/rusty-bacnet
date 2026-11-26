//! Adding and removing points on a live watch (#2603): nothing for a point
//! after its removal returns, a held point added again, an add past the
//! hub's capacity, a point removed and added again, a control outliving
//! its watch, an empty watch waiting for an add, and closing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bacnet_types::primitives::PropertyValue;
use futures_util::FutureExt;

use super::hub::WatchOptions;
use super::stream_tests::{lab_device, latest, next, number, ready, until};
use super::test_lab::{Lab, SUBSCRIBE_COV, ai, settle};
use super::*;

/// Set the lab device's AI `instance` to `value`.
pub(super) async fn set(lab: &Lab, instance: u32, value: f32) {
    lab.server
        .set_present_value_local(&ai(instance), PropertyValue::Real(value))
        .await
        .unwrap();
}

/// Confirmed requests of `service` the lab's client sent so far.
pub(super) fn sent(lab: &Lab, service: u8) -> usize {
    lab.services().iter().filter(|s| **s == service).count()
}

/// Wait, on the real clock, for the client's pooled hubs to go, their
/// tasks ended, so the lab can take its client back.
pub(super) async fn hubs_gone(lab: &Lab) {
    for _ in 0..500 {
        if super::stream_tests::pooled(&lab.client) == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("a pooled hub outlived its watches");
}

/// Whether `update` is of `point`, subscribed.
fn subscribed(update: &Update, point: &Point) -> bool {
    update.point == *point && update.state == State::Subscribed
}

/// Acceptance: once `remove` returns, nothing comes for the point: not the
/// change already waiting in the mailbox, nor any later one; the other
/// point goes on, and the removed one's subscription is cancelled.
#[tokio::test(start_paused = true)]
async fn nothing_comes_for_a_point_after_its_removal_returns() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let (one, two) = (device.point("ai:1").unwrap(), device.point("ai:2").unwrap());
    let mut watch = lab.client.watch([one.clone(), two.clone()]).unwrap();
    until(&mut watch, |u| subscribed(u, &one)).await;
    until(&mut watch, |u| subscribed(u, &two)).await;
    latest(&mut watch).await;
    let control = watch.control();
    // A change of ai:1 waits in the mailbox.
    set(&lab, 1, 55.0).await;
    settle().await;
    assert_eq!(control.remove([&one]).unwrap(), 1);
    assert_eq!(control.remove([&one]).unwrap(), 0, "held no more");
    assert_eq!(watch.points(), std::slice::from_ref(&two));
    assert_eq!(control.points(), std::slice::from_ref(&two));
    set(&lab, 1, 66.0).await;
    set(&lab, 2, 77.0).await;
    let update = next(&mut watch).await;
    assert_eq!(update.point, two, "{update:?}");
    assert_eq!(number(&update), Some(77.0));
    settle().await;
    while let Some(update) = ready(&mut watch) {
        assert_ne!(update.point, one, "{update:?}");
    }
    let table = lab.table().await;
    assert_eq!(table.len(), 1, "ai:1's was cancelled: {table:?}");
    assert_eq!(
        table[0].monitored_property_reference.object_identifier,
        ai(2)
    );
    drop((watch, control, device));
    settle().await;
    lab.stop().await;
}

/// Concurrency: a removal from one task while another awaits `next`, the
/// device changing both points all the while. No `next` begun after the
/// removal returned yields the removed point. Real threads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_removal_beside_a_waiting_next_lets_nothing_through_after() {
    let lab = Arc::new(Lab::start(WatchOptions::default()).await);
    let device = lab_device(&lab).await;
    let (one, two) = (device.point("ai:1").unwrap(), device.point("ai:2").unwrap());
    let mut watch = lab.client.watch([one.clone(), two.clone()]).unwrap();
    let wait = Duration::from_secs(10);
    tokio::time::timeout(wait, until(&mut watch, |u| subscribed(u, &one)))
        .await
        .unwrap();
    let control = watch.control();
    let removed = Arc::new(AtomicBool::new(false));
    let reader = {
        let (removed, one, two) = (Arc::clone(&removed), one.clone(), two.clone());
        tokio::spawn(async move {
            let mut late = Vec::new();
            loop {
                // Read before the call begins: a call begun after the
                // removal returned.
                let after = removed.load(Ordering::SeqCst);
                let update = watch.next().await.expect("the watch goes on");
                if after && update.point == one {
                    late.push(update);
                } else if update.point == two && number(&update) == Some(-1.0) {
                    return late;
                }
            }
        })
    };
    let writer = {
        let lab = Arc::clone(&lab);
        tokio::spawn(async move {
            for step in 0..40u8 {
                set(&lab, 1, f32::from(step)).await;
                set(&lab, 2, f32::from(step)).await;
                tokio::task::yield_now().await;
            }
        })
    };
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(control.remove([&one]).unwrap(), 1);
    removed.store(true, Ordering::SeqCst);
    writer.await.unwrap();
    set(&lab, 2, -1.0).await;
    let late = tokio::time::timeout(wait, reader)
        .await
        .expect("the reader saw the last change")
        .unwrap();
    assert!(late.is_empty(), "after the removal: {late:?}");
    drop((control, device));
    let lab = Arc::try_unwrap(lab).ok().expect("the lab's last holder");
    hubs_gone(&lab).await;
    lab.stop().await;
}

/// Adding a point the watch holds does nothing and counts none; one named
/// twice in an add is added once.
#[tokio::test(start_paused = true)]
async fn adding_a_held_point_does_nothing() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let (one, two) = (device.point("ai:1").unwrap(), device.point("ai:2").unwrap());
    let mut watch = lab.client.watch([one.clone()]).unwrap();
    until(&mut watch, |u| subscribed(u, &one)).await;
    latest(&mut watch).await;
    let control = watch.control();
    let sent = lab.services().len();
    assert_eq!(control.add([one.clone(), one.clone()]).unwrap(), 0);
    settle().await;
    assert_eq!(lab.services().len(), sent, "nothing sent");
    assert!(ready(&mut watch).is_none(), "no update for the held point");
    assert_eq!(
        control
            .add([two.clone(), two.clone(), one.clone()])
            .unwrap(),
        1
    );
    assert_eq!(watch.points(), [one, two.clone()]);
    until(&mut watch, |u| subscribed(u, &two)).await;
    assert_eq!(lab.table().await.len(), 2);
    drop((watch, control, device));
    settle().await;
    lab.stop().await;
}

/// An add that would take the hub past what it watches at once is
/// refused whole, naming the most, and sends nothing.
#[tokio::test(start_paused = true)]
async fn an_add_past_the_hubs_capacity_takes_nothing() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let one = device.point("ai:1").unwrap();
    let mut watch = lab.client.watch([one.clone()]).unwrap();
    next(&mut watch).await;
    latest(&mut watch).await;
    let control = watch.control();
    let more: Vec<Point> = (2..=MAX_WATCHED as u32 + 1)
        .map(|n| device.point(format!("ai:{n}").as_str()).unwrap())
        .collect();
    let sent = lab.services().len();
    assert_eq!(
        control.add(more),
        Err(WatchError::Full { max: MAX_WATCHED })
    );
    settle().await;
    assert_eq!(lab.services().len(), sent, "nothing sent");
    assert_eq!(watch.points(), [one]);
    drop((watch, control, device));
    settle().await;
    lab.stop().await;
}

/// A point removed and added again is a new point of the watch: its old
/// subscription is cancelled, the new one takes its own subscriber process
/// identifier, and its first update comes from a fresh first read.
#[tokio::test(start_paused = true)]
async fn a_point_removed_and_added_again_starts_afresh() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let one = device.point("ai:1").unwrap();
    let mut watch = lab.client.watch([one.clone()]).unwrap();
    until(&mut watch, |u| subscribed(u, &one)).await;
    let old = lab.table().await[0].recipient.process_identifier;
    let control = watch.control();
    assert_eq!(control.remove([&one]).unwrap(), 1);
    settle().await;
    assert!(lab.table().await.is_empty(), "cancelled");
    set(&lab, 1, 31.0).await;
    assert_eq!(control.add([one.clone()]).unwrap(), 1);
    let first = next(&mut watch).await;
    assert_eq!(first.point, one);
    assert_eq!(number(&first), Some(31.0), "{first:?}");
    assert!(
        matches!(first.via, Some(Via::First | Via::Notification)),
        "{first:?}"
    );
    until(&mut watch, |u| subscribed(u, &one)).await;
    let table = lab.table().await;
    assert_eq!(table.len(), 1);
    assert_ne!(table[0].recipient.process_identifier, old);
    drop((watch, control, device));
    settle().await;
    lab.stop().await;
}

/// Every change of a point after its add returns comes as an update to a
/// task already awaiting the watch.
#[tokio::test(start_paused = true)]
async fn a_change_after_an_add_reaches_a_waiting_next() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let (one, three) = (device.point("ai:1").unwrap(), device.point("ai:3").unwrap());
    let mut watch = lab.client.watch([one.clone()]).unwrap();
    until(&mut watch, |u| subscribed(u, &one)).await;
    latest(&mut watch).await;
    let control = watch.control();
    let wanted = three.clone();
    let waiting = tokio::spawn(async move {
        until(&mut watch, |u| u.point == wanted && number(u) == Some(42.0)).await;
        watch
    });
    settle().await;
    assert_eq!(control.add([three]).unwrap(), 1);
    set(&lab, 3, 42.0).await;
    let watch = tokio::time::timeout(Duration::from_secs(60), waiting)
        .await
        .expect("the change came")
        .unwrap();
    drop((watch, control, device));
    settle().await;
    lab.stop().await;
}

/// A control outliving its watch is refused, typed; closing again does
/// nothing.
#[tokio::test(start_paused = true)]
async fn a_control_outliving_its_watch_is_refused() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let (one, two) = (device.point("ai:1").unwrap(), device.point("ai:2").unwrap());
    let watch = lab.client.watch([one.clone()]).unwrap();
    let control = watch.control();
    let clone = control.clone();
    drop(watch);
    assert_eq!(control.add([two]), Err(WatchError::Closed));
    assert_eq!(clone.remove([&one]), Err(WatchError::Closed));
    assert!(control.points().is_empty());
    assert!(clone.close(Duration::from_secs(1)).await, "closed already");
    drop((control, clone, device));
    settle().await;
    lab.stop().await;
}

/// A watch of no points waits while a control lives, takes points added
/// and given back, and ends once no control is left; ended, it stays so.
#[tokio::test(start_paused = true)]
async fn an_empty_watch_waits_while_a_control_lives() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let one = device.point("ai:1").unwrap();
    let mut watch = lab.client.watch([]).unwrap();
    let control = watch.control();
    assert!(watch.next().now_or_never().is_none(), "it waits");
    assert_eq!(control.add([one.clone()]).unwrap(), 1);
    assert_eq!(next(&mut watch).await.point, one);
    assert_eq!(control.remove([&one]).unwrap(), 1);
    settle().await;
    assert!(watch.next().now_or_never().is_none(), "it waits again");
    drop(control);
    assert!(watch.next().await.is_none(), "no control: it ends");
    assert_eq!(watch.control().add([one]), Err(WatchError::Closed));
    assert!(watch.next().await.is_none(), "and stays ended");
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

/// Closing ends the stream at once and waits for the cancels of the points
/// no other watch holds; one another watch holds goes on.
#[tokio::test(start_paused = true)]
async fn close_ends_the_stream_and_waits_for_the_cancels() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let (one, two) = (device.point("ai:1").unwrap(), device.point("ai:2").unwrap());
    let mut watch = lab.client.watch([one.clone(), two.clone()]).unwrap();
    let mut other = lab.client.watch([two.clone()]).unwrap();
    until(&mut watch, |u| subscribed(u, &one)).await;
    until(&mut other, |u| subscribed(u, &two)).await;
    let control = watch.control();
    let before = sent(&lab, SUBSCRIBE_COV);
    assert!(control.close(Duration::from_secs(10)).await);
    assert_eq!(
        sent(&lab, SUBSCRIBE_COV) - before,
        1,
        "ai:1's cancel, answered"
    );
    let table = lab.table().await;
    assert_eq!(table.len(), 1, "ai:2 goes on: {table:?}");
    assert_eq!(
        table[0].monitored_property_reference.object_identifier,
        ai(2)
    );
    assert!(watch.next().await.is_none());
    assert_eq!(control.add([one]), Err(WatchError::Closed));
    drop((watch, other, control, device));
    settle().await;
    lab.stop().await;
}

/// An update held back (a buffer, a slot) is refused once its point was
/// removed, even when it was added again: its mark names the old place.
#[tokio::test(start_paused = true)]
async fn an_update_held_back_is_refused_after_its_removal() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let one = device.point("ai:1").unwrap();
    let mut watch = lab.client.watch([one.clone()]).unwrap();
    let control = watch.control();
    let held = next(&mut watch).await;
    assert!(control.holds(held.mark()));
    assert_eq!(control.remove([&one]).unwrap(), 1);
    assert!(!control.holds(held.mark()), "removed");
    assert_eq!(control.add([one.clone()]).unwrap(), 1);
    assert!(!control.holds(held.mark()), "added again: another point");
    let fresh = next(&mut watch).await;
    assert!(control.holds(fresh.mark()));
    drop(watch);
    assert!(!control.holds(fresh.mark()), "the watch ended");
    drop((control, device));
    settle().await;
    lab.stop().await;
}
