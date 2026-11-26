//! Races on a live watch's points (#2603): two controls adding and removing
//! one point from two threads, an add racing the hub's shutdown, and a
//! pooled hub taken again while a cancel of its is still out.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::control_tests::{hubs_gone, set};
use super::hub::{PROCESS_BASE, WatchOptions};
use super::stream_tests::{lab_device, number, pooled, until};
use super::test_lab::{Lab, settle};
use super::*;

/// The range of subscriber process identifiers `process` is in.
fn range_of(process: u32) -> u32 {
    (process - PROCESS_BASE) / 0x1_0000
}

/// Wait, on the real clock, until the lab device's subscription table holds
/// `count` subscriptions.
async fn table_holds(lab: &Lab, count: usize) {
    for _ in 0..1000 {
        if lab.table().await.len() == count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the table: {:?}", lab.table().await);
}

/// Two controls of one watch add and remove one point from two threads.
/// Each call's count is the change it made, so what the two counted adds
/// up to whether the watch holds the point at the end; the hub agrees: the
/// device holds a subscription only while some watch holds the point.
/// With `other`, a second watch holds the point throughout, and its
/// subscription goes on untouched.
async fn two_controls_race(other: bool) {
    let lab = Arc::new(Lab::start(WatchOptions::default()).await);
    let device = lab_device(&lab).await;
    let (one, two) = (device.point("ai:1").unwrap(), device.point("ai:2").unwrap());
    let wait = Duration::from_secs(10);
    let mut keeper = match other {
        true => {
            let mut keeper = lab.client.watch([two.clone()]).unwrap();
            tokio::time::timeout(wait, until(&mut keeper, |u| u.state == State::Subscribed))
                .await
                .unwrap();
            Some(keeper)
        }
        false => None,
    };
    let mut watch = lab.client.watch([one.clone()]).unwrap();
    tokio::time::timeout(wait, until(&mut watch, |u| u.state == State::Subscribed))
        .await
        .unwrap();
    let controls = [watch.control(), watch.control()];
    let tasks: Vec<_> = controls
        .into_iter()
        .map(|control| {
            let two = two.clone();
            tokio::spawn(async move {
                let mut net = 0i64;
                for _ in 0..20 {
                    net += i64::try_from(control.add([two.clone()]).unwrap()).unwrap();
                    tokio::task::yield_now().await;
                    net -= i64::try_from(control.remove([&two]).unwrap()).unwrap();
                }
                net += i64::try_from(control.add([two.clone()]).unwrap()).unwrap();
                net
            })
        })
        .collect();
    let mut net = 0;
    for task in tasks {
        net += tokio::time::timeout(wait, task).await.unwrap().unwrap();
    }
    let held = watch.points().contains(&two);
    assert_eq!(net, i64::from(held), "the counts: {net}, held: {held}");
    assert!(held, "each task added it last");
    // The device holds one subscription for each point watched.
    table_holds(&lab, 2).await;
    // The point goes on for this watch, and for the other.
    set(&lab, 2, 42.0).await;
    tokio::time::timeout(
        wait,
        until(&mut watch, |u| u.point == two && number(u) == Some(42.0)),
    )
    .await
    .unwrap();
    if let Some(keeper) = keeper.as_mut() {
        tokio::time::timeout(wait, until(keeper, |u| number(u) == Some(42.0)))
            .await
            .unwrap();
    }
    // Removed from this watch, it goes from the device unless the other
    // holds it.
    assert_eq!(watch.control().remove([&two]).unwrap(), 1);
    table_holds(&lab, if other { 2 } else { 1 }).await;
    drop((watch, keeper, device));
    hubs_gone(&lab).await;
    let lab = Arc::try_unwrap(lab).ok().expect("the lab's last holder");
    lab.stop().await;
}

/// Concurrency: two threads' controls on one point no other watch holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_controls_adding_and_removing_one_point_agree() {
    two_controls_race(false).await;
}

/// Concurrency: the same while another watch holds the point.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_controls_racing_on_a_point_another_watch_holds() {
    two_controls_race(true).await;
}

/// Concurrency: adds racing the hub's shutdown. Whichever comes first, the
/// watch ends, its control is refused once it has, and the device is left
/// holding no subscription of the hub's: an add that took its points
/// before the shutdown has them cancelled with the rest, one after takes
/// none.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_add_racing_the_shutdown_leaves_nothing_subscribed() {
    let lab = Arc::new(Lab::start(WatchOptions::default()).await);
    let device = lab_device(&lab).await;
    let one = device.point("ai:1").unwrap();
    let mut watch = lab.client.watch([one]).unwrap();
    let wait = Duration::from_secs(10);
    tokio::time::timeout(wait, until(&mut watch, |u| u.state == State::Subscribed))
        .await
        .unwrap();
    let control = watch.control();
    let stop = Arc::new(AtomicBool::new(false));
    let adder = {
        let (stop, points) = (
            Arc::clone(&stop),
            [device.point("ai:2").unwrap(), device.point("ai:3").unwrap()],
        );
        tokio::spawn(async move {
            let mut refused = None;
            while !stop.load(Ordering::SeqCst) {
                match control.add(points.clone()) {
                    Ok(_) => {}
                    Err(error) => {
                        refused = Some(error);
                        break;
                    }
                }
                let _ = control.remove(points.iter());
                tokio::task::yield_now().await;
            }
            (refused, control)
        })
    };
    tokio::task::yield_now().await;
    assert!(lab.client.shutdown_watches(Duration::from_secs(5)).await);
    stop.store(true, Ordering::SeqCst);
    let (refused, control) = tokio::time::timeout(wait, adder).await.unwrap().unwrap();
    if let Some(refused) = refused {
        assert!(
            matches!(refused, WatchError::Closed | WatchError::Shut),
            "{refused:?}"
        );
    }
    assert!(
        matches!(
            control.add([device.point("ai:2").unwrap()]),
            Err(WatchError::Closed)
        ),
        "the watch ended"
    );
    assert!(
        tokio::time::timeout(wait, watch.next())
            .await
            .unwrap()
            .is_none()
    );
    table_holds(&lab, 0).await;
    drop((watch, control, device));
    hubs_gone(&lab).await;
    let lab = Arc::try_unwrap(lab).ok().expect("the lab's last holder");
    lab.stop().await;
}

/// A pooled hub whose last watch went, its cancel still out to a device gone
/// silent, is the hub the next watch of the same settings takes: the same
/// range of subscriber process identifiers. The point watched again takes a
/// new identifier, so the old cancel, whatever becomes of it, never ends
/// the new subscription, and the new watch gets its values.
#[tokio::test(start_paused = true)]
async fn a_pooled_hub_taken_again_while_its_cancel_is_out() {
    let lab = Lab::start(WatchOptions::default()).await;
    let device = lab_device(&lab).await;
    let mut watch = device.watch(["ai:1"]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    let old = lab.table().await[0].recipient.process_identifier;
    lab.quirks.muted.store(true, Ordering::Relaxed);
    drop(watch);
    // The cancel goes out and waits for an answer that doesn't come.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(pooled(&lab.client), 1, "the hub waits for its cancel");
    let mut again = device.watch(["ai:1"]).unwrap();
    assert_eq!(pooled(&lab.client), 1, "the same hub");
    assert_eq!(lab.client.shared.ranges.lock().unwrap().held(), 2);
    lab.quirks.muted.store(false, Ordering::Relaxed);
    until(&mut again, |u| u.state == State::Subscribed).await;
    // Past every wait of the old cancel's.
    tokio::time::sleep(Duration::from_secs(40)).await;
    settle().await;
    let table = lab.table().await;
    let new = table
        .iter()
        .map(|s| s.recipient.process_identifier)
        .find(|process| *process != old)
        .unwrap_or_else(|| panic!("the new subscription: {table:?}"));
    assert_eq!(range_of(new), range_of(old), "the same range");
    set(&lab, 1, 61.0).await;
    let update = until(&mut again, |u| number(u) == Some(61.0)).await;
    assert_eq!(update.point, device.point("ai:1").unwrap());
    drop((again, device));
    settle().await;
    lab.stop().await;
}
