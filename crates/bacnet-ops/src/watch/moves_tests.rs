//! The hub keyed by address (#2560 slice 6), on the device tests' bus: two
//! devices sharing an instance are two points, and a watched device that
//! answers at another address is reported moved, whether the hub finds
//! it, by the discovery table or as a following handle would, or a
//! following handle's own request does.

use std::time::Duration;

use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use futures_util::StreamExt;

use super::stream_tests::{latest, next, number};
use super::*;
use crate::Device;
use crate::device::test_bus::{Bus, N, at, client, requests, server, settle, stop};
use crate::device::{ClientEvent, QUIET};

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

async fn set(server: &BACnetServer<TestTransport>, value: f32) {
    server
        .set_present_value_local(&ai1(), PropertyValue::Real(value))
        .await
        .unwrap();
}

/// Updates until `done` holds for the latest one, within ten minutes of
/// the paused clock.
async fn until(watch: &mut Watch, done: impl Fn(&Update) -> bool) -> Update {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    loop {
        let update = tokio::time::timeout_at(deadline, watch.next())
            .await
            .expect("the update looked for, within ten minutes")
            .expect("the watch goes on");
        if done(&update) {
            return update;
        }
    }
}

fn moved_to(update: &Update) -> Option<&crate::DeviceAddress> {
    match &update.why {
        Some(Why::Moved { to }) => Some(to),
        _ => None,
    }
}

fn subscribes(sent: &SendLog, station: u8) -> usize {
    requests(sent, ConfirmedServiceChoice::SUBSCRIBE_COV, station)
}

/// Acceptance: two devices claiming one instance, each named by its
/// address, are two points: each holds its own subscription, and each
/// notification reaches the point at the address it came from.
#[tokio::test(start_paused = true)]
async fn two_devices_sharing_an_instance_are_two_points() {
    let bus = Bus::new();
    let servers = vec![
        server(bus.port(2), N, "two").await,
        server(bus.port(3), N, "three").await,
    ];
    let (client, _sent) = client(&bus).await;
    bus.i_am(2, N);
    bus.i_am(3, N);
    settle().await;
    assert_eq!(
        client.duplicates().await.len(),
        1,
        "one instance, two claims"
    );
    let two = client.device(at(2)).await.unwrap();
    let three = client.device(at(3)).await.unwrap();
    let points = [two.point("ai:1").unwrap(), three.point("ai:1").unwrap()];
    let mut watch = client.watch(points.clone()).unwrap();
    assert_eq!(watch.points().count(), 2);
    let mut subscribed = std::collections::HashSet::new();
    while subscribed.len() < 2 {
        let update = until(&mut watch, |u| u.source == Some(Source::Cov)).await;
        subscribed.insert(update.point);
    }
    latest(&mut watch).await;
    for server in &servers {
        assert_eq!(server.cov_counters().subscriptions_active, 1);
    }
    set(&servers[0], 31.0).await;
    let update = next(&mut watch).await;
    assert_eq!((&update.point, number(&update)), (&points[0], Some(31.0)));
    set(&servers[1], 32.0).await;
    let update = next(&mut watch).await;
    assert_eq!((&update.point, number(&update)), (&points[1], Some(32.0)));
    drop((watch, two, three));
    settle().await;
    for server in &servers {
        assert_eq!(server.cov_counters().subscriptions_active, 0, "cancelled");
    }
    stop(servers).await;
}

/// A pinned device whose address goes quiet while its instance announces
/// itself at another, more than [`QUIET`] later: once its renewal fails,
/// the point is given up as moved there, a point of the old address
/// watched later is given up at once, and nothing more is sent there.
#[tokio::test(start_paused = true)]
async fn a_device_heard_elsewhere_is_reported_moved() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "old").await];
    let (client, sent) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(at(2)).await.unwrap();
    assert!(!device.follows());
    let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.source == Some(Source::Cov)).await;
    let subscribed = tokio::time::Instant::now();
    tokio::time::sleep(QUIET + Duration::from_secs(1)).await;
    servers.push(server(bus.port(3), N, "new").await);
    bus.i_am(3, N);
    bus.silence(2);
    let update = until(&mut watch, |u| u.state == State::Gone).await;
    assert_eq!(moved_to(&update), Some(&at(3)), "{update:?}");
    assert_eq!(update.state, State::Gone);
    // Found by the poll that follows the failed renewal (240 s in), not by
    // the subscription tried again five minutes after it.
    let found = subscribed.elapsed();
    assert!(found < Duration::from_secs(260), "found after {found:?}");
    let (before, unicasts) = (subscribes(&sent, 2), sent.unicasts().len());
    // A point of the old address watched now is given up at once.
    let mut late = client
        .watch([device.point("ai:1 status-flags").unwrap()])
        .unwrap();
    let update = next(&mut late).await;
    assert_eq!(
        (moved_to(&update), update.state),
        (Some(&at(3)), State::Gone)
    );
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(subscribes(&sent, 2), before, "no renewal, no cancel");
    assert_eq!(sent.unicasts().len(), unicasts, "nothing sent at all");
    drop((watch, late, device));
    stop(servers).await;
}

/// A handle following an instance: when its device stops answering, the
/// hub asks where it went as the handle would, once, and once only the
/// other address has answered, the point is reported moved there, the
/// handle moves with it, and the point watched again from the handle is
/// subscribed at the new address.
#[tokio::test(start_paused = true)]
async fn a_following_device_that_moves_is_reported_by_the_hub() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "old").await];
    let (client, _sent) = client(&bus).await;
    let device = client.device(N).await.unwrap();
    assert!(device.follows());
    assert_eq!(device.address(), at(2));
    let mut events = client.events();
    let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.source == Some(Source::Cov)).await;
    servers.push(server(bus.port(3), N, "new").await);
    bus.silence(2);
    let update = until(&mut watch, |u| u.state == State::Gone).await;
    assert_eq!(moved_to(&update), Some(&at(3)), "{update:?}");
    assert_eq!(
        events.try_recv().unwrap(),
        ClientEvent::Moved {
            instance: N,
            from: at(2),
            to: at(3)
        }
    );
    assert_eq!(device.address(), at(3), "the handle moved too");
    let mut again = client.watch([device.point("ai:1").unwrap()]).unwrap();
    let update = until(&mut again, |u| u.source == Some(Source::Cov)).await;
    assert_eq!(update.point.device, at(3));
    assert_eq!(servers[1].cov_counters().subscriptions_active, 1);
    drop((watch, again, device));
    settle().await;
    stop(servers).await;
}

/// A following handle that moves for a request of its own tells the hub,
/// which gives up the points at the old address at once, without waiting
/// for one of its own to fail.
#[tokio::test(start_paused = true)]
async fn a_handle_moving_for_its_own_request_tells_the_hub() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "old").await];
    let (client, _sent) = client(&bus).await;
    let device: Device<TestTransport> = client.device(N).await.unwrap();
    let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.source == Some(Source::Cov)).await;
    latest(&mut watch).await;
    servers.push(server(bus.port(3), N, "new").await);
    bus.silence(2);
    // The first read times out and asks; the next, past the resolve wait,
    // finds only station 3 answered and moves there.
    assert!(device.read("ai:1 pv").await.is_err());
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(device.read("ai:1 pv").await.is_ok());
    assert_eq!(device.address(), at(3));
    let update = latest(&mut watch).await.expect("told of the move");
    assert_eq!(moved_to(&update), Some(&at(3)), "{update:?}");
    assert_eq!(update.state, State::Gone);
    drop((watch, device));
    settle().await;
    stop(servers).await;
}
