//! Each live hub on a client holds a range of subscriber process
//! identifiers of its own (#2185, #2561): a hub takes the next free one,
//! going round, and gives it back only once it is gone, its cancels done,
//! so however many hubs come and go none shares a range with a hub alive,
//! or with one still cancelling, and a range given back is taken again
//! only after the other free ones; with every range held, no hub is made.

use std::time::Duration;

use super::hub::{Hub, PROCESS_BASE, PROCESS_RANGES, WatchOptions};
use super::stream_tests::{lab_device, until};
use super::test_lab::{Lab, ai, object, settle};
use super::{State, WatchError};

/// The first process identifier of `hub`'s range.
fn base<T: bacnet_transport::port::TransportPort + 'static>(hub: &Hub<T>) -> u32 {
    hub.inner.registry.process_base
}

/// The first process identifier of range `index`.
fn range(index: u32) -> u32 {
    PROCESS_BASE + index * 0x1_0000
}

/// A hub on the lab's client with the default settings.
fn hub(lab: &Lab) -> Result<Hub<bacnet_transport::test_transport::TestTransport>, WatchError> {
    Hub::new(lab.client.clone(), WatchOptions::default())
}

#[tokio::test(start_paused = true)]
async fn hubs_made_and_dropped_never_take_a_live_hubs_range() {
    let lab = Lab::start(WatchOptions::default()).await;
    assert_eq!(base(&lab.hub), range(0), "the lab's hub, kept");
    let device = lab_device(&lab).await;
    // The client's own hub takes the next range, for the client's life.
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    let mut last = None;
    for _ in 0..300 {
        let hub = hub(&lab).unwrap();
        let taken = base(&hub);
        assert!(taken != range(0) && taken != range(1), "a live hub's range");
        assert_ne!(Some(taken), last, "the range given back just now");
        last = Some(taken);
        drop(hub);
    }
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_range_given_back_is_not_the_next_taken_while_others_are_free() {
    let lab = Lab::start(WatchOptions::default()).await;
    let first = hub(&lab).unwrap();
    assert_eq!(base(&first), range(1));
    drop(first);
    let second = hub(&lab).unwrap();
    assert_eq!(base(&second), range(2), "range 1 waits its turn");
    drop(second);
    settle().await;
    lab.stop().await;
}

#[tokio::test(start_paused = true)]
async fn with_every_range_held_no_hub_is_made() {
    let lab = Lab::start(WatchOptions::default()).await;
    let mut live: Vec<_> = (1..PROCESS_RANGES).map(|_| hub(&lab).unwrap()).collect();
    assert_eq!(
        hub(&lab).err(),
        Some(WatchError::Hubs {
            max: PROCESS_RANGES as usize
        })
    );
    // The client's own hub can't be made either.
    let point = object(ai(1));
    assert!(lab.client.watch([point]).is_err());
    // One gone frees one.
    let freed = base(&live.swap_remove(7));
    let again = hub(&lab).unwrap();
    assert_eq!(base(&again), freed);
    drop((again, live));
    // Their notification receivers end on their next turn.
    settle().await;
    lab.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_dropped_hubs_range_waits_for_its_cancels() {
    let lab = Lab::start(WatchOptions::default()).await;
    let cancelling = hub(&lab).unwrap();
    assert_eq!(base(&cancelling), range(1));
    // Every other range held: range 1 is the only one that could be free.
    let others: Vec<_> = (2..PROCESS_RANGES).map(|_| hub(&lab).unwrap()).collect();
    let mut watch = cancelling.stream([object(ai(1))]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    // The device goes silent: the cancel waits out its request's time.
    lab.quirks
        .muted
        .store(true, std::sync::atomic::Ordering::Relaxed);
    drop((watch, cancelling));
    settle().await;
    assert!(
        matches!(hub(&lab), Err(WatchError::Hubs { .. })),
        "range 1 is still cancelling"
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = hub(&lab).unwrap();
    assert_eq!(
        base(&after),
        range(1),
        "the cancel gave up: range 1 is free"
    );
    drop((after, others));
    settle().await;
    lab.quirks
        .muted
        .store(false, std::sync::atomic::Ordering::Relaxed);
    lab.stop().await;
}

/// Outside a Tokio runtime no hub is made, and no range is taken.
#[test]
fn outside_a_runtime_a_hub_is_refused() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let lab = runtime.block_on(Lab::start(WatchOptions::default()));
    let refused = hub(&lab).err();
    assert_eq!(refused, Some(WatchError::NoRuntime));
    let held = lab.client.shared.ranges.lock().unwrap().held();
    assert_eq!(held, 1, "the lab's hub's alone");
    runtime.block_on(async {
        settle().await;
        lab.stop().await;
    });
}
