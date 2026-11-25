//! A watched device changing under its watch (#2603), on the device tests'
//! bus: another device taking a watched address is never taken for the
//! watched one (a pinned watch gives its point up until its device comes
//! back; one made through a following handle reports where it went), and a
//! device that restarts is subscribed again when it announces itself, at
//! most once a lifetime, within the client's request limits.

use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::Apdu;
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use tokio::time::Instant;

use super::State;
use super::stream_tests::{number, ready, until};
use super::*;
use crate::device::test_bus::{Bus, N, at, client, config, requests, server, settle, stop};
use crate::limits::RequestLimits;

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

async fn set(server: &BACnetServer<TestTransport>, value: f32) {
    server
        .set_present_value_local(&ai1(), PropertyValue::Real(value))
        .await
        .unwrap();
}

fn subscribes(sent: &SendLog, station: u8) -> usize {
    requests(sent, ConfirmedServiceChoice::SUBSCRIBE_COV, station)
}

fn gone_for(update: &Update) -> Option<&Why> {
    match &update.state {
        State::Gone { why } => Some(why),
        _ => None,
    }
}

/// A pinned watch of device `N` at station 2, subscribed, on a new bus.
struct Pinned {
    bus: Arc<Bus>,
    client: crate::Client<TestTransport>,
    sent: SendLog,
    device: crate::Device<TestTransport>,
    watch: Watch,
}

impl Pinned {
    async fn new(first: &mut Option<BACnetServer<TestTransport>>) -> Self {
        let bus = Bus::new();
        *first = Some(server(bus.port(2), N, "first").await);
        let (client, sent) = client(&bus).await;
        bus.i_am(2, N);
        settle().await;
        let device = client.device(at(2)).await.unwrap();
        assert!(!device.follows());
        let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
        until(&mut watch, |u| u.state == State::Subscribed).await;
        Self {
            bus,
            client,
            sent,
            device,
            watch,
        }
    }
}

/// Acceptance (#2603 A): the watched device leaves its address and another,
/// with the same objects, takes it and announces itself. The point is
/// given up for it at once, and over the old subscription's renewal and
/// lifetime no value of the new device's is ever reported as the point's,
/// and nothing is sent to it.
#[tokio::test(start_paused = true)]
async fn a_device_taking_a_watched_address_is_never_the_watched_one() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    let first = first.unwrap();
    set(&first, 21.0).await;
    until(&mut lab.watch, |u| number(u) == Some(21.0)).await;
    stop(vec![first]).await;
    let second = server(lab.bus.port(2), N + 1, "second").await;
    lab.bus.i_am(2, N + 1);
    let update = until(&mut lab.watch, |u| gone_for(u).is_some()).await;
    let replaced = Why::Replaced {
        watched: N,
        found: N + 1,
    };
    assert_eq!(gone_for(&update), Some(&replaced));
    assert_eq!(number(&update), Some(21.0), "the last value of its own");
    let subscribed = subscribes(&lab.sent, 2);
    let end = Instant::now() + Duration::from_secs(400);
    let mut value = 99.0;
    while Instant::now() < end {
        set(&second, value).await;
        value += 1.0;
        tokio::time::sleep(Duration::from_secs(5)).await;
        while let Some(update) = ready(&mut lab.watch) {
            assert!(
                number(&update).is_none_or(|v| v < 99.0),
                "the new device's value as the point's: {update:?}"
            );
        }
    }
    assert_eq!(subscribes(&lab.sent, 2), subscribed, "nothing sent to it");
    assert_eq!(second.cov_counters().subscriptions_active, 0);
    // Let go, it is cancelled with nothing sent; watched there again, it
    // starts as any point does, with the device there now.
    drop(lab.watch);
    settle().await;
    assert_eq!(subscribes(&lab.sent, 2), subscribed, "no cancel either");
    let mut late = lab
        .client
        .watch([lab.device.point("ai:1").unwrap()])
        .unwrap();
    let update = until(&mut late, |u| u.state == State::Subscribed).await;
    assert!(number(&update).is_some_and(|v| v >= 99.0), "{update:?}");
    drop((late, lab.device));
    settle().await;
    stop(vec![second]).await;
}

/// The device of a pinned watch given up for another device at its address
/// comes back there and announces itself: the point starts again, and its
/// values are taken again.
#[tokio::test(start_paused = true)]
async fn the_watched_device_coming_back_brings_a_pinned_watch_back() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    stop(vec![first.unwrap()]).await;
    let second = server(lab.bus.port(2), N + 1, "second").await;
    lab.bus.i_am(2, N + 1);
    until(&mut lab.watch, |u| gone_for(u).is_some()).await;
    stop(vec![second]).await;
    let back = server(lab.bus.port(2), N, "back").await;
    lab.bus.i_am(2, N);
    until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    set(&back, 33.0).await;
    let update = until(&mut lab.watch, |u| number(u) == Some(33.0)).await;
    assert_eq!(update.via, Some(Via::Notification));
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![back]).await;
}

/// A watch made through a following handle, whose device moved to station
/// 3 while another device took station 2: the point is reported moved
/// there, the handle follows it, and the point watched again from the
/// handle is subscribed at station 3. The watch itself stays at its
/// address, as the handle's points are made there (`Device::point`).
#[tokio::test(start_paused = true)]
async fn a_following_watch_whose_address_was_taken_reports_the_move() {
    let bus = Bus::new();
    let old = server(bus.port(2), N, "old").await;
    let (client, _sent) = client(&bus).await;
    let device = client.device(N).await.unwrap();
    assert!(device.follows());
    let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    stop(vec![old]).await;
    let moved = server(bus.port(3), N, "moved").await;
    bus.i_am(3, N);
    let taker = server(bus.port(2), N + 1, "taker").await;
    bus.i_am(2, N + 1);
    let update = until(&mut watch, |u| {
        matches!(gone_for(u), Some(Why::Moved { .. }))
    })
    .await;
    assert_eq!(gone_for(&update), Some(&Why::Moved { to: at(3) }));
    assert_eq!(device.address(), at(3), "the handle followed it");
    let mut again = client.watch([device.point("ai:1").unwrap()]).unwrap();
    let update = until(&mut again, |u| u.state == State::Subscribed).await;
    assert_eq!(update.point.device, at(3));
    assert_eq!(moved.cov_counters().subscriptions_active, 1);
    assert_eq!(taker.cov_counters().subscriptions_active, 0);
    drop((watch, again, device));
    settle().await;
    stop(vec![moved, taker]).await;
}

/// Acceptance (#2603 B): a watched device restarts, losing its
/// subscriptions, and announces itself: its point is subscribed again at
/// once, so its next change comes as a notification long before the old
/// subscription's renewal would have made it again.
#[tokio::test(start_paused = true)]
async fn a_restarted_device_is_subscribed_again_when_it_announces_itself() {
    let mut first = None;
    let mut lab = Pinned::new(&mut first).await;
    let started = Instant::now();
    stop(vec![first.unwrap()]).await;
    let again = server(lab.bus.port(2), N, "again").await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    lab.bus.i_am(2, N);
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(
        again.cov_counters().subscriptions_active,
        1,
        "subscribed again on its announcement"
    );
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

/// A device starting up announces itself several times over a few
/// seconds: its point is subscribed again once.
#[tokio::test(start_paused = true)]
async fn a_burst_of_announcements_subscribes_again_once() {
    let mut first = None;
    let lab = Pinned::new(&mut first).await;
    let before = subscribes(&lab.sent, 2);
    for _ in 0..5 {
        lab.bus.i_am(2, N);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(subscribes(&lab.sent, 2) - before, 1, "one subscription");
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![first.unwrap()]).await;
}

/// A device that announces itself every five seconds is subscribed again
/// at most once a lifetime (five minutes): over ten minutes, its point's
/// subscriptions are the early ones (at most three) and the renewals of
/// each, not one an announcement.
#[tokio::test(start_paused = true)]
async fn a_device_announcing_itself_often_is_subscribed_again_once_a_lifetime() {
    let mut first = None;
    let lab = Pinned::new(&mut first).await;
    let before = subscribes(&lab.sent, 2);
    let end = Instant::now() + Duration::from_secs(600);
    while Instant::now() < end {
        lab.bus.i_am(2, N);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let sent = subscribes(&lab.sent, 2) - before;
    // Early at 0 s and 300 s (and 600 s at most), each renewed 240 s after;
    // with renewals alone, two.
    assert!(
        (3..=6).contains(&sent),
        "{sent} subscriptions in ten minutes"
    );
    drop((lab.watch, lab.device));
    settle().await;
    stop(vec![first.unwrap()]).await;
}

/// The confirmed requests in `sent` from index `seen` on.
fn confirmed(sent: &SendLog, seen: usize) -> usize {
    sent.frames()[seen..]
        .iter()
        .filter(|frame| !frame.mac.is_empty())
        .filter(|frame| matches!(frame.apdu(), Apdu::ConfirmedRequest(_)))
        .count()
}

/// Six watched devices announce themselves at once, as after a power cut:
/// their points are read and subscribed again no faster than the client's
/// rate (four requests a second), one at a time, not in a burst.
#[tokio::test(start_paused = true)]
async fn devices_announcing_at_once_are_subscribed_again_within_the_rate() {
    const DEVICES: u8 = 6;
    let bus = Bus::new();
    let mut servers = Vec::new();
    for n in 0..DEVICES {
        servers.push(server(bus.port(2 + n), N + u32::from(n), "device").await);
    }
    let port = bus.port(1);
    let sent = port.sent();
    let low = BACnetClient::start(config(), port).await.unwrap();
    let client = crate::Client::new(Arc::new(low), Arc::new(RequestLimits::new(4)));
    let mut points = Vec::new();
    for n in 0..DEVICES {
        bus.i_am(2 + n, N + u32::from(n));
        settle().await;
        let device = client.device(at(2 + n)).await.unwrap();
        points.push(device.point("ai:1").unwrap());
    }
    let mut watch = client.watch(points).unwrap();
    for _ in 0..DEVICES {
        until(&mut watch, |u| u.state == State::Subscribed).await;
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    let mut seen = sent.len();
    for n in 0..DEVICES {
        bus.i_am(2 + n, N + u32::from(n));
    }
    let mut times = Vec::new();
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let new = confirmed(&sent, seen);
        times.extend(std::iter::repeat_n(Instant::now(), new));
        seen = sent.len();
    }
    // A first read and a subscription a point.
    assert_eq!(times.len(), 2 * usize::from(DEVICES), "{times:?}");
    for pair in times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            gap >= Duration::from_millis(240),
            "two requests {gap:?} apart"
        );
    }
    for server in &servers {
        assert_eq!(server.cov_counters().subscriptions_active, 1);
    }
    drop(watch);
    settle().await;
    stop(servers).await;
}
