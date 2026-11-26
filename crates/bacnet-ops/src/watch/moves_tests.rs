//! The hub keyed by address (#2560 slice 6), on the device tests' bus: two
//! devices sharing an instance are two points; a watched device is given
//! up as moved only when the move is certain (its address now names
//! another instance and one other address claims its own, or a following
//! handle moved), never for a duplicate starting up while a request goes
//! unanswered; and the move doesn't stick: the old address announcing the
//! instance again starts its points again.

use std::time::Duration;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_server::server::BACnetServer;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::State;
use super::stream_tests::{latest, next, number, ready, until};
use super::*;
use crate::Device;
use crate::device::QUIET;
use crate::device::test_bus::{Bus, N, at, client, client_on, mac, requests, server, settle, stop};

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

async fn set(server: &BACnetServer<TestTransport>, value: f32) {
    server
        .set_present_value_local(&ai1(), PropertyValue::Real(value))
        .await
        .unwrap();
}

fn moved_to(update: &Update) -> Option<&crate::DeviceAddress> {
    match &update.state {
        State::Gone {
            why: Why::Moved { to },
        } => Some(to),
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
    assert_eq!(watch.points().len(), 2);
    let mut subscribed = std::collections::HashSet::new();
    while subscribed.len() < 2 {
        let update = until(&mut watch, |u| u.state == State::Subscribed).await;
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

/// The lab of a certain move: a pinned device's watched point, whose
/// address went quiet and now names another instance while its own
/// announced itself at station 3. Once its renewal fails, the point is
/// given up as moved there, found by the poll that follows the failed
/// renewal (240 s in), not by the subscription tried again five minutes
/// after it, and nothing more is sent for it.
struct Moved {
    bus: std::sync::Arc<Bus>,
    servers: Vec<BACnetServer<TestTransport>>,
    client: crate::Client<TestTransport>,
    device: Device<TestTransport>,
    watch: Watch,
}

impl Moved {
    async fn new() -> Self {
        let bus = Bus::new();
        let mut servers = vec![server(bus.port(2), N, "old").await];
        let (client, sent) = client(&bus).await;
        bus.i_am(2, N);
        settle().await;
        let device = client.device(at(2)).await.unwrap();
        assert!(!device.follows());
        let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
        until(&mut watch, |u| u.state == State::Subscribed).await;
        let subscribed = tokio::time::Instant::now();
        servers.push(server(bus.port(3), N, "new").await);
        bus.i_am(3, N);
        bus.silence(2);
        // Another device took the old address.
        bus.i_am(2, N + 1);
        let update = until(&mut watch, |u| matches!(u.state, State::Gone { .. })).await;
        assert_eq!(moved_to(&update), Some(&at(3)), "{update:?}");
        let found = subscribed.elapsed();
        assert!(found < Duration::from_secs(260), "found after {found:?}");
        let before = subscribes(&sent, 2);
        tokio::time::sleep(Duration::from_secs(400)).await;
        assert_eq!(subscribes(&sent, 2), before, "no renewal, no cancel");
        bus.unsilence(2);
        Self {
            bus,
            servers,
            client,
            device,
            watch,
        }
    }

    async fn stop(self) {
        drop((self.watch, self.device));
        settle().await;
        stop(self.servers).await;
    }
}

/// A certain move gives the point up; a point of the old address made
/// later starts as any other, from the table's other instance there. The
/// device there is still the watched one: its notifications name it, so it
/// is asked which it is (#2603), answers as the watched device, and the
/// given-up point starts again there, the later one taking that device
/// too.
#[tokio::test(start_paused = true)]
async fn a_device_whose_address_names_another_is_reported_moved() {
    let mut lab = Moved::new().await;
    let mut late = lab
        .client
        .watch([lab.device.point("ai:1 status-flags").unwrap()])
        .unwrap();
    let update = until(&mut late, |u| u.value.is_some() || u.error.is_some()).await;
    assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
    let update = until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    assert_eq!(update.at, at(2));
    let update = latest(&mut late).await;
    assert!(
        update.is_none_or(|u| !matches!(u.state, State::Gone { .. })),
        "the later point goes on"
    );
    drop(late);
    lab.stop().await;
}

/// A point given up as moved starts again when a start at its address
/// answers while the discovery table names its instance there, though no
/// announcement came.
#[tokio::test(start_paused = true)]
async fn a_start_answered_for_the_instance_brings_a_moved_point_back() {
    let mut lab = Moved::new().await;
    // Named there again by hand: the table sends no event for it.
    lab.client.low_level().add_device(N, &mac(2)).await.unwrap();
    let mut late = lab
        .client
        .watch([lab.device.point("ai:1 status-flags").unwrap()])
        .unwrap();
    until(&mut late, |u| u.value.is_some()).await;
    let update = until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    assert_eq!(update.at, at(2));
    drop(late);
    lab.stop().await;
}

/// The duplicate restart: a second device claiming the instance starts up
/// and announces itself while the watched one misses its requests. Its
/// announcement alone is no certain move: the point stays unreachable,
/// backing off, and goes on once its device answers again.
#[tokio::test(start_paused = true)]
async fn a_duplicate_starting_up_does_not_end_a_watch() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "first").await];
    let (client, _sent) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(at(2)).await.unwrap();
    let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    tokio::time::sleep(QUIET + Duration::from_secs(1)).await;
    servers.push(server(bus.port(3), N, "duplicate").await);
    bus.i_am(3, N);
    bus.silence(2);
    let lost = until(&mut watch, |u| matches!(u.state, State::Unreachable { .. })).await;
    assert!(
        matches!(
            lost.state,
            State::Unreachable {
                why: Why::Lapsed(_)
            }
        ),
        "{lost:?}"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    bus.unsilence(2);
    until(&mut watch, |u| {
        assert!(!matches!(u.state, State::Gone { .. }), "{u:?}");
        u.state == State::Subscribed
    })
    .await;
    drop((watch, device));
    settle().await;
    stop(servers).await;
}

/// A handle following an instance: when its device stops answering, the
/// hub asks where it went as the handle would, once, and once only the
/// other address has answered and named the device to the hub's read, the
/// point goes there (#2795): the same point, its update saying where it
/// went, then values from the new address. The hub took the move after
/// its own request at the old address went unanswered, without asking it
/// once more, so the handle stays (only a move whose old address was asked
/// moves handles); the point watched again from the handle is the same
/// point, still at the new address, where the hub follows the device.
#[tokio::test(start_paused = true)]
async fn a_following_device_that_moves_is_followed_by_the_hub() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "old").await];
    let (client, sent) = client(&bus).await;
    let device = client.device(N).await.unwrap();
    assert!(device.follows());
    assert_eq!(device.address(), at(2));
    let point = device.point("ai:1").unwrap();
    assert_eq!(
        point.device,
        Binding::Follows {
            instance: N,
            network: None
        }
    );
    let mut events = client.events();
    let mut watch = client.watch([point.clone()]).unwrap();
    let first = until(&mut watch, |u| u.state == State::Subscribed).await;
    assert_eq!(
        (first.at, first.instance, first.moved),
        (at(2), Some(N), None)
    );
    servers.push(server(bus.port(3), N, "new").await);
    bus.silence(2);
    let update = until(&mut watch, |u| {
        assert!(!matches!(u.state, State::Gone { .. }), "{u:?}");
        u.moved.is_some()
    })
    .await;
    assert_eq!(update.point, point, "the same point");
    let moved = update.moved.clone().unwrap();
    assert_eq!(
        (&moved.from, &moved.to, &update.at),
        (&at(2), &at(3), &at(3))
    );
    assert!(moved.when.elapsed().is_ok());
    assert_eq!(moved.count, 1);
    assert!(events.try_recv().is_err(), "no handle moved");
    assert_eq!(device.address(), at(2), "the handle stays");
    until(&mut watch, |u| u.state == State::Subscribed).await;
    set(&servers[1], 42.0).await;
    let update = until(&mut watch, |u| number(u) == Some(42.0)).await;
    assert_eq!(
        (update.at, update.moved.map(|m| m.to)),
        (at(3), Some(at(3)))
    );
    let mut again = client.watch([device.point("ai:1").unwrap()]).unwrap();
    let update = until(&mut again, |u| u.state == State::Subscribed).await;
    assert_eq!((update.point, update.at), (point, at(3)));
    assert_eq!(servers[1].cov_counters().subscriptions_active, 1);
    let before = subscribes(&sent, 2);
    drop((watch, again, device));
    settle().await;
    assert_eq!(
        servers[1].cov_counters().subscriptions_active,
        0,
        "cancelled"
    );
    assert_eq!(
        subscribes(&sent, 2),
        before,
        "nothing cancelled at station 2"
    );
    bus.unsilence(2);
    stop(servers).await;
}

/// A following handle that moves for a request of its own (once the new
/// address named the device to it) tells the hub, which takes the
/// following point there without waiting for one of its own requests to
/// fail, once the new address names the device to its own read and the old
/// one, asked once more, doesn't answer (#2812, #2795).
#[tokio::test(start_paused = true)]
async fn a_handle_moving_for_its_own_request_tells_the_hub() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "old").await];
    let (client, _sent) = client(&bus).await;
    let device: Device<TestTransport> = client.device(N).await.unwrap();
    let mut watch = client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| u.state == State::Subscribed).await;
    latest(&mut watch).await;
    servers.push(server(bus.port(3), N, "new").await);
    bus.silence(2);
    // The first read times out and asks; the next, past the resolve wait,
    // finds only station 3 answered and moves there.
    assert!(device.read("ai:1 pv").await.is_err());
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(device.read("ai:1 pv").await.is_ok());
    assert_eq!(device.address(), at(3));
    let update = until(&mut watch, |u| u.moved.is_some()).await;
    assert_eq!(update.at, at(3), "{update:?}");
    until(&mut watch, |u| u.state == State::Subscribed).await;
    drop((watch, device));
    settle().await;
    stop(servers).await;
}

/// One station named two ways (as the local station, and by the client's
/// own network's number, once the client knows it) is one point.
#[tokio::test(start_paused = true)]
async fn one_station_named_two_ways_is_one_point() {
    let (near, two) = (Bus::new(), Bus::new());
    let ports = [(&near, 1), (&two, 2)]
        .into_iter()
        .map(|(bus, network_number)| RouterPort::new(bus.port(254), network_number))
        .collect();
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let servers = vec![server(near.port(2), N, "near").await];
    let (client, _sent) = client_on(near.numbered_port(1)).await;
    let low = client.low_level();
    low.what_is_network_number(None).await.unwrap();
    while low.network_number().is_none() {
        settle().await;
    }
    near.i_am(2, N);
    settle().await;
    let device = client.device(at(2)).await.unwrap();
    let local = device.point("ai:1").unwrap();
    let routed = Point {
        device: Binding::At(crate::DeviceAddress::new(1, &mac(2)).unwrap()),
        ..local.clone()
    };
    assert_ne!(local, routed, "two ways of writing it");
    let mut watch = client.watch([local.clone(), routed]).unwrap();
    assert_eq!(watch.points(), std::slice::from_ref(&local));
    until(&mut watch, |u| u.state == State::Subscribed).await;
    assert_eq!(servers[0].cov_counters().subscriptions_active, 1);
    drop((watch, device));
    settle().await;
    stop(servers).await;
    router.stop().await.unwrap();
}

/// Two devices sharing an instance, both watched: station 2 goes quiet for
/// twenty minutes while station 3 keeps answering, and station 2's row
/// ages out of the discovery table. A row merely gone is no move: station
/// 2's point is never given up as moved to station 3, and goes on once
/// station 2 answers again, though it doesn't announce itself.
#[tokio::test(start_paused = true)]
async fn a_quiet_duplicate_aged_out_is_no_move() {
    let bus = Bus::new();
    let servers = vec![
        server(bus.port(2), N, "two").await,
        server(bus.port(3), N, "three").await,
    ];
    let (client, _sent) = client(&bus).await;
    bus.i_am(2, N);
    bus.i_am(3, N);
    settle().await;
    let two = client.device(at(2)).await.unwrap();
    let three = client.device(at(3)).await.unwrap();
    let points = [two.point("ai:1").unwrap(), three.point("ai:1").unwrap()];
    let mut watch = client.watch(points.clone()).unwrap();
    let mut subscribed = std::collections::HashSet::new();
    while subscribed.len() < 2 {
        let update = until(&mut watch, |u| u.state == State::Subscribed).await;
        subscribed.insert(update.point);
    }
    latest(&mut watch).await;
    bus.silence(2);
    for _ in 0..120 {
        tokio::time::sleep(Duration::from_secs(10)).await;
        while let Some(update) = ready(&mut watch) {
            assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
        }
    }
    assert!(
        client.low_level().get_device_at(0, &mac(2)).await.is_none(),
        "station 2's row aged out"
    );
    bus.unsilence(2);
    let back = until(&mut watch, |u| {
        assert!(!matches!(u.state, State::Gone { .. }), "{u:?}");
        u.point == points[0] && u.state == State::Subscribed
    })
    .await;
    assert_eq!(back.at, at(2));
    drop((watch, two, three));
    settle().await;
    stop(servers).await;
}

/// A given-up point's address announces the instance again just as its
/// last watch is dropped, in every interleaving a few yields give: nothing
/// stays subscribed on the device, and the hub ends.
#[tokio::test(start_paused = true)]
async fn a_revival_racing_the_last_drop_leaves_nothing() {
    for yields in 0..24u32 {
        let Moved {
            bus,
            servers,
            client,
            device,
            watch,
        } = Moved::new().await;
        assert_eq!(servers[0].cov_counters().subscriptions_active, 0, "lapsed");
        if yields % 2 == 1 {
            drop(watch);
            for _ in 0..yields / 2 {
                tokio::task::yield_now().await;
            }
            bus.i_am(2, N);
        } else {
            bus.i_am(2, N);
            for _ in 0..yields / 2 {
                tokio::task::yield_now().await;
            }
            drop(watch);
        }
        settle().await;
        tokio::time::sleep(Duration::from_secs(40)).await;
        settle().await;
        assert_eq!(
            servers[0].cov_counters().subscriptions_active,
            0,
            "yields {yields}: a subscription nobody holds"
        );
        assert!(
            client.shared.watch_pool.lock().unwrap().live() == 0,
            "yields {yields}: the hub ended"
        );
        drop(device);
        settle().await;
        stop(servers).await;
    }
}
