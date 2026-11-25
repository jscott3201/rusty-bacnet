//! Each live hub on a client holds a range of subscriber process
//! identifiers of its own (#2185, #2561): a hub takes the lowest free one
//! and gives it back only once it is gone, its cancels done, so however
//! many hubs come and go none shares a range with a hub alive, or with one
//! still cancelling; with every range held, no hub is made.

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

#[tokio::test(start_paused = true)]
async fn hubs_made_and_dropped_never_take_a_live_hubs_range() {
    let lab = Lab::start(WatchOptions::default()).await;
    assert_eq!(base(&lab.hub), range(0), "the lab's hub, kept");
    let device = lab_device(&lab).await;
    // The client's own hub takes the next range, for the client's life.
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    for _ in 0..300 {
        let hub = Hub::new(lab.client.clone(), WatchOptions::default()).unwrap();
        // The lowest free range, given back each time.
        assert_eq!(base(&hub), range(2));
        drop(hub);
    }
    drop((watch, device));
    settle().await;
    lab.stop().await;
}

#[tokio::test(start_paused = true)]
async fn with_every_range_held_no_hub_is_made() {
    let lab = Lab::start(WatchOptions::default()).await;
    let mut live: Vec<_> = (1..PROCESS_RANGES)
        .map(|_| Hub::new(lab.client.clone(), WatchOptions::default()).unwrap())
        .collect();
    let refused = Hub::new(lab.client.clone(), WatchOptions::default()).err();
    assert_eq!(
        refused,
        Some(WatchError::Hubs {
            max: PROCESS_RANGES as usize
        })
    );
    // The client's own hub can't be made either.
    let point = object(ai(1));
    assert!(lab.client.watch([point]).is_err());
    // One gone frees one.
    let freed = base(&live.swap_remove(7));
    let again = Hub::new(lab.client.clone(), WatchOptions::default()).unwrap();
    assert_eq!(base(&again), freed);
    drop((again, live));
    // Their notification receivers end on their next turn.
    settle().await;
    lab.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_dropped_hubs_range_waits_for_its_cancels() {
    let lab = Lab::start(WatchOptions::default()).await;
    let hub = Hub::new(lab.client.clone(), WatchOptions::default()).unwrap();
    assert_eq!(base(&hub), range(1));
    let mut watch = hub.stream([object(ai(1))]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    // The device goes silent: the cancel waits out its request's time.
    lab.quirks
        .muted
        .store(true, std::sync::atomic::Ordering::Relaxed);
    drop((watch, hub));
    settle().await;
    let meanwhile = Hub::new(lab.client.clone(), WatchOptions::default()).unwrap();
    assert_eq!(base(&meanwhile), range(2), "range 1 is still cancelling");
    drop(meanwhile);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = Hub::new(lab.client.clone(), WatchOptions::default()).unwrap();
    assert_eq!(
        base(&after),
        range(1),
        "the cancel gave up: range 1 is free"
    );
    drop(after);
    settle().await;
    lab.quirks
        .muted
        .store(false, std::sync::atomic::Ordering::Relaxed);
    lab.stop().await;
}
