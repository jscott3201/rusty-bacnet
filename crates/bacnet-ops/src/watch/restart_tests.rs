//! A watched device that restarts (#2603), on the device tests' bus: its
//! Time_Of_Device_Restart, read once when it is first subscribed, is read
//! again when it announces itself (at most once a lifetime), and only a
//! change subscribes its points again; one without the property costs
//! nothing more and is recovered at its renewal. Many devices answering one
//! Who-Is are asked one after another, within the client's rate.

use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::Apdu;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::EngineeringUnits;
use tokio::time::Instant;

use super::State;
use super::identity_tests::{Pinned, reads, set, subscribes};
use super::stream_tests::{number, until};
use super::*;
use crate::device::test_bus::{Bus, N, at, config, server, settle, stop};
use crate::limits::RequestLimits;

/// A lifetime of the client's own hub, and a little.
const LIFETIME: Duration = Duration::from_secs(301);

/// Acceptance (#2603 B): a watched device restarts, losing its
/// subscriptions, and announces itself: its restart time moved, so its
/// point is subscribed again within the spread, and its next change comes
/// as a notification long before the renewal.
#[tokio::test(start_paused = true)]
async fn a_restarted_device_is_subscribed_again_when_it_announces_itself() {
    let mut first = None;
    let mut lab = Pinned::timed(&mut first, Some(1)).await;
    tokio::time::sleep(Duration::from_secs(40)).await;
    let started = Instant::now();
    stop(vec![first.unwrap()]).await;
    let again = server(lab.bus.port(2), N, "again").await;
    lab.bus.restart_time(2, Some(2));
    lab.bus.i_am(2, N);
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert_eq!(again.cov_counters().subscriptions_active, 1);
    set(&again, 42.0).await;
    let update = until(&mut lab.watch, |u| number(u) == Some(42.0)).await;
    assert_eq!(update.via, Some(Via::Notification), "{update:?}");
    assert_eq!(update.state, State::Subscribed);
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "before renewal"
    );
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![again]).await;
}

/// A restarted device announces itself several times over a few seconds:
/// its restart time is read once and its point subscribed again once.
#[tokio::test(start_paused = true)]
async fn a_burst_of_announcements_subscribes_again_once() {
    let mut first = None;
    let lab = Pinned::timed(&mut first, Some(1)).await;
    tokio::time::sleep(Duration::from_secs(40)).await;
    let (before, read) = (subscribes(&lab.sent, 2), reads(&lab.sent, 2));
    lab.bus.restart_time(2, Some(2));
    for _ in 0..5 {
        lab.bus.i_am(2, N);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    tokio::time::sleep(Duration::from_secs(15)).await;
    assert_eq!(subscribes(&lab.sent, 2) - before, 1, "one subscription");
    assert_eq!(reads(&lab.sent, 2) - read, 1, "one read of the time");
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![first.unwrap()]).await;
}

/// A device that announces itself every five seconds without restarting:
/// over ten minutes its restart time is read once a lifetime, and its
/// point is subscribed only by its renewals (at 240 s and 480 s).
#[tokio::test(start_paused = true)]
async fn a_device_announcing_itself_often_costs_a_read_a_lifetime() {
    let mut first = None;
    let lab = Pinned::timed(&mut first, Some(1)).await;
    let (before, read) = (subscribes(&lab.sent, 2), reads(&lab.sent, 2));
    let end = Instant::now() + Duration::from_secs(600);
    while Instant::now() < end {
        lab.bus.i_am(2, N);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    assert_eq!(subscribes(&lab.sent, 2) - before, 2, "its renewals alone");
    let read = reads(&lab.sent, 2) - read;
    assert!((2..=3).contains(&read), "{read} reads");
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![first.unwrap()]).await;
}

/// The instants each confirmed request in `sent` went at, watching for
/// `seconds` from now in steps of 10 ms.
async fn request_times(sent: &SendLog, seconds: u64) -> Vec<Instant> {
    let mut seen = sent.len();
    let mut times = Vec::new();
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let frames = sent.frames();
        let new = frames[seen..]
            .iter()
            .filter(|frame| !frame.mac.is_empty())
            .filter(|frame| matches!(frame.apdu(), Apdu::ConfirmedRequest(_)))
            .count();
        times.extend(std::iter::repeat_n(Instant::now(), new));
        seen = frames.len();
    }
    times
}

/// A device `instance` on `port` with `points` analog inputs.
async fn device_of(port: TestTransport, instance: u32, points: u32) -> BACnetServer<TestTransport> {
    let identity = DeviceIdentity::new(instance, 42).unwrap();
    let objects: Vec<Box<dyn BACnetObject>> = (1..=points)
        .map(|n| {
            let ai = AnalogInputObject::new(n, format!("AI {n}"), EngineeringUnits::PERCENT);
            Box::new(ai.unwrap()) as Box<dyn BACnetObject>
        })
        .collect();
    let database = build_database_with_extra(&identity, objects).unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .database(database)
        .transport(port)
        .build()
        .await
        .unwrap()
}

/// `devices` devices of `points` points each on a new bus, watched by a
/// client limited to `rate` requests a second (0 for none), all
/// subscribed; with each device's Time_Of_Device_Restart `time`, if set.
async fn fleet(
    devices: u8,
    points: u32,
    rate: u32,
    time: Option<u8>,
) -> (Arc<Bus>, Vec<BACnetServer<TestTransport>>, SendLog, Watch) {
    let bus = Bus::new();
    let mut servers = Vec::new();
    for n in 0..devices {
        bus.restart_time(2 + n, time);
        servers.push(device_of(bus.port(2 + n), N + u32::from(n), points).await);
    }
    let port = bus.port(1);
    let sent = port.sent();
    let low = BACnetClient::start(config(), port).await.unwrap();
    let client = crate::Client::new(Arc::new(low), Arc::new(RequestLimits::new(rate)));
    let mut watched = Vec::new();
    for n in 0..devices {
        bus.i_am(2 + n, N + u32::from(n));
        settle().await;
        let device = client.device(at(2 + n)).await.unwrap();
        for point in 1..=points {
            watched.push(device.point(format!("ai:{point}").as_str()).unwrap());
        }
    }
    let mut watch = client.watch(watched).unwrap();
    for _ in 0..usize::from(devices) * points as usize {
        until(&mut watch, |u| u.state == State::Subscribed).await;
    }
    // The restart time each device's first subscription reads.
    tokio::time::sleep(Duration::from_secs(10)).await;
    (bus, servers, sent, watch)
}

/// Six watched devices restart at once, as after a power cut, and
/// announce themselves: each restart time is read and each point read and
/// subscribed again no faster than the client's rate (four requests a
/// second), one at a time, not in a burst.
#[tokio::test(start_paused = true)]
async fn devices_announcing_at_once_are_subscribed_again_within_the_rate() {
    const DEVICES: u8 = 6;
    let (bus, servers, sent, watch) = fleet(DEVICES, 1, 4, Some(1)).await;
    tokio::time::sleep(Duration::from_secs(40)).await;
    for n in 0..DEVICES {
        bus.restart_time(2 + n, Some(2));
        bus.i_am(2 + n, N + u32::from(n));
    }
    let times = request_times(&sent, 20).await;
    // A read of the restart time, then a first read and a subscription.
    assert_eq!(times.len(), 3 * usize::from(DEVICES), "{times:?}");
    for pair in times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(gap >= Duration::from_millis(240), "{gap:?} apart");
    }
    drop(watch);
    settle().await;
    stop(servers).await;
}

/// Twenty devices of five points each, with Time_Of_Device_Restart, answer
/// a Who-Is a lifetime on: each is asked its restart time (one read each,
/// one device after another), and none is subscribed again. A lifetime
/// later one that did restart announces itself: its points alone are
/// subscribed again, once.
#[tokio::test(start_paused = true)]
async fn a_who_is_answered_by_many_devices_resubscribes_none_that_did_not_restart() {
    const DEVICES: u8 = 20;
    let (bus, servers, sent, watch) = fleet(DEVICES, 5, 0, Some(1)).await;
    tokio::time::sleep(LIFETIME).await;
    let all = |of: fn(&SendLog, u8) -> usize| (0..DEVICES).map(|n| of(&sent, 2 + n)).sum::<usize>();
    let (subscribed, read) = (all(subscribes), all(reads));
    for n in 0..DEVICES {
        bus.i_am(2 + n, N + u32::from(n));
    }
    let times = request_times(&sent, 20).await;
    assert_eq!(all(subscribes), subscribed, "none subscribed again");
    assert_eq!(all(reads), read + usize::from(DEVICES), "one read each");
    assert_eq!(times.len(), usize::from(DEVICES), "nothing more");
    let mut instants = times.clone();
    instants.dedup();
    assert!(instants.len() >= 10, "spread, not at once: {times:?}");
    let span = times[times.len() - 1] - times[0];
    assert!(span >= Duration::from_secs(5), "over {span:?}");
    tokio::time::sleep(LIFETIME).await;
    bus.restart_time(5, Some(2));
    let (subscribed, at_five) = (all(subscribes), subscribes(&sent, 5));
    bus.i_am(5, N + 3);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(subscribes(&sent, 5) - at_five, 5, "its five points");
    assert_eq!(all(subscribes) - subscribed, 5, "no other's");
    drop(watch);
    settle().await;
    stop(servers).await;
}

/// The same twenty devices without Time_Of_Device_Restart: after the one
/// read each when first subscribed, a Who-Is a lifetime on sends nothing;
/// they are recovered at their renewals.
#[tokio::test(start_paused = true)]
async fn a_who_is_costs_nothing_for_devices_without_a_restart_time() {
    const DEVICES: u8 = 20;
    let (bus, servers, sent, watch) = fleet(DEVICES, 5, 0, None).await;
    tokio::time::sleep(LIFETIME).await;
    for n in 0..DEVICES {
        bus.i_am(2 + n, N + u32::from(n));
    }
    let times = request_times(&sent, 20).await;
    assert!(times.is_empty(), "{times:?}");
    drop(watch);
    settle().await;
    stop(servers).await;
}
