//! Following points (#2795): a point watched through a following handle
//! follows its device's instance from address to address, the same point
//! throughout, once the hub's own reads say the device went; nothing an
//! unsolicited frame says moves one. On the device tests' bus.

use std::time::Duration;

use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::State;
use super::move_cue_tests::{Handle, Lab};
use super::stream_tests::{number, pooled, ready, until};
use super::*;
use crate::device::test_bus::{Bus, N, at, client, requests, server, settle, stop};

fn ai(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

const SUBSCRIBE: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV;

/// Device N moves to station 3 (station 2 silent, station 3 claiming it):
/// the lab's following point is followed there.
async fn move_to_three(lab: &mut Lab) -> Update {
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.i_am(3, N);
    lab.bus.silence(2);
    lab.tell(3);
    until(&mut lab.watch, |u| {
        assert!(!matches!(u.state, State::Gone { .. }), "{u:?}");
        u.moved.is_some()
    })
    .await
}

/// The device comes back to station 2 after its move to station 3: the
/// point, at station 3, finds station 3 silent, looks for its device, and
/// follows it back once station 2 names it to the hub's read.
#[tokio::test(start_paused = true)]
async fn a_device_that_comes_back_is_followed_back() {
    let mut lab = Lab::new(Handle::Follows).await;
    let moved = move_to_three(&mut lab).await;
    assert_eq!(moved.at, at(3));
    until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    lab.bus.silence(3);
    lab.bus.unsilence(2);
    let back = until(&mut lab.watch, |u| {
        u.moved.as_ref().is_some_and(|m| m.to == at(2))
    })
    .await;
    let moved = back.moved.clone().unwrap();
    assert_eq!((moved.from, back.at, moved.count), (at(3), at(2), 2));
    until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    lab.goes_on(51.0).await;
    lab.bus.unsilence(3);
    lab.stop().await;
}

/// After its move the device goes quiet at station 3 too: the point is
/// unreachable there, where it was followed to, and goes on there when the
/// device answers again.
#[tokio::test(start_paused = true)]
async fn a_device_silent_after_its_move_is_unreachable_there() {
    let mut lab = Lab::new(Handle::Follows).await;
    move_to_three(&mut lab).await;
    until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    lab.bus.silence(3);
    let lost = until(&mut lab.watch, |u| {
        matches!(u.state, State::Unreachable { .. })
    })
    .await;
    assert_eq!(lost.at, at(3), "{lost:?}");
    assert_eq!(lost.moved.as_ref().map(|m| &m.to), Some(&at(3)));
    lab.bus.unsilence(3);
    let back = until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    assert_eq!(back.at, at(3));
    lab.bus.unsilence(2);
    lab.stop().await;
}

/// Two other stations claim the instance while station 2 is silent: no
/// move while the instance is contested; the point stays unreachable at
/// station 2, and goes on there once the device answers again.
#[tokio::test(start_paused = true)]
async fn two_other_claimants_move_no_following_point() {
    let mut lab = Lab::new(Handle::Follows).await;
    lab.servers.push(server(lab.bus.port(3), N, "three").await);
    lab.servers.push(server(lab.bus.port(4), N, "four").await);
    lab.bus.i_am(3, N);
    lab.bus.i_am(4, N);
    lab.bus.silence(2);
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(10)).await;
        while let Some(update) = ready(&mut lab.watch) {
            assert!(update.moved.is_none(), "{update:?}");
            assert_eq!(update.at, at(2), "{update:?}");
        }
    }
    for station in [3, 4] {
        assert_eq!(
            lab.servers[station - 2].cov_counters().subscriptions_active,
            0
        );
    }
    lab.bus.unsilence(2);
    lab.servers[0]
        .set_present_value_local(&ai(1), PropertyValue::Real(52.0))
        .await
        .unwrap();
    let update = until(&mut lab.watch, |u| number(u) == Some(52.0)).await;
    assert_eq!(
        (update.at, update.moved),
        (at(2), None),
        "polled there again"
    );
    lab.stop().await;
}

/// Another device takes station 2, and a forged I-Am names the watched
/// instance at station 3, where nothing answers: the following point is
/// unreachable for the replacement while the hub looks for its device
/// (never given up), its values gone, nothing of the device now at station
/// 2 taken, and the hub's read at station 3, unanswered, moves nothing.
#[tokio::test(start_paused = true)]
async fn a_taken_address_with_its_device_nowhere_leaves_the_point_unreachable() {
    let mut lab = Lab::new(Handle::Follows).await;
    stop(vec![lab.servers.remove(0)]).await;
    lab.servers
        .push(server(lab.bus.port(2), N + 1, "taker").await);
    lab.bus.i_am(2, N + 1);
    let update = until(&mut lab.watch, |u| {
        matches!(
            u.state,
            State::Unreachable {
                why: Why::Replaced { .. }
            }
        )
    })
    .await;
    assert_eq!(
        update.state,
        State::Unreachable {
            why: Why::Replaced {
                watched: N,
                found: N + 1
            }
        }
    );
    assert_eq!((update.at, update.value.is_none()), (at(2), true));
    lab.bus.i_am(3, N);
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(10)).await;
        while let Some(update) = ready(&mut lab.watch) {
            assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
            assert!(
                update.moved.is_none() && update.value.is_none(),
                "{update:?}"
            );
        }
    }
    assert!(lab.reads_at(3) >= 1, "the hub read station 3");
    assert_eq!(lab.servers[0].cov_counters().subscriptions_active, 0);
    lab.stop().await;
}

/// A move's update waiting in the mailbox when the point is removed: no
/// update of the point comes after `remove` returns, and its subscription
/// at the new address is cancelled.
#[tokio::test(start_paused = true)]
async fn a_removed_point_gets_nothing_of_its_move() {
    let mut lab = Lab::new(Handle::Follows).await;
    let control = lab.watch.control();
    let point = lab.device.as_ref().unwrap().point("ai:1").unwrap();
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.i_am(3, N);
    lab.bus.silence(2);
    lab.tell(3);
    while lab.device.as_ref().unwrap().address() != at(3) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    settle().await;
    assert_eq!(control.remove([&point]).unwrap(), 1);
    assert!(ready(&mut lab.watch).is_none(), "nothing after remove");
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(ready(&mut lab.watch).is_none(), "nothing after remove");
    assert_eq!(lab.servers[1].cov_counters().subscriptions_active, 0);
    lab.bus.unsilence(2);
    lab.stop().await;
}

/// A following point added while the hub checks a move (its read at
/// station 3 named the device, its ask at station 2 is waiting) starts
/// where the hub follows the device then, station 2, and goes to station 3
/// with the others once the move is taken: it was never subscribed at
/// station 2.
#[tokio::test(start_paused = true)]
async fn a_point_added_mid_move_goes_with_the_others() {
    let mut lab = Lab::new(Handle::Follows).await;
    let control = lab.watch.control();
    let added = lab
        .device
        .as_ref()
        .unwrap()
        .point("ai:1 status-flags")
        .unwrap();
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.silence(2);
    lab.tell(3);
    settle().await;
    assert!(lab.reads_at(3) >= 1, "station 3 read");
    let before = requests(&lab.sent, SUBSCRIBE, 2);
    assert_eq!(control.add([added.clone()]).unwrap(), 1);
    let mut at_three = std::collections::HashSet::new();
    while at_three.len() < 2 {
        let update = until(&mut lab.watch, |u| u.state == State::Subscribed).await;
        assert_eq!(update.at, at(3), "{update:?}");
        at_three.insert(update.point);
    }
    assert!(at_three.contains(&added));
    assert_eq!(lab.servers[1].cov_counters().subscriptions_active, 2);
    assert_eq!(
        requests(&lab.sent, SUBSCRIBE, 2),
        before,
        "nothing subscribed at station 2"
    );
    lab.bus.unsilence(2);
    lab.stop().await;
}

/// The link fails every send to station 3 while the hub is told the device
/// went there: the client's own failure says nothing of the device (#2820),
/// so nothing moves and nothing is given up.
#[tokio::test(start_paused = true)]
async fn a_local_failure_at_the_new_address_moves_nothing() {
    let mut lab = Lab::new(Handle::Follows).await;
    lab.servers
        .push(server(lab.bus.port(3), N, "unreachable").await);
    lab.bus.break_link(3);
    lab.tell(3);
    lab.none_given_up(5).await;
    lab.goes_on(53.0).await;
    lab.stop().await;
}

/// A pinned point and a following point of one object watched with the
/// client's settings share its pooled hub (two subscriptions, two process
/// identifiers). When the device moves, the pinned one is given up and the
/// following one goes with it; dropping both lets the hub and its range go.
#[tokio::test(start_paused = true)]
async fn pinned_and_following_points_share_a_pooled_hub() {
    let bus = Bus::new();
    let mut servers = vec![server(bus.port(2), N, "old").await];
    let (client, sent) = client(&bus).await;
    bus.i_am(2, N);
    settle().await;
    let pinned = client.device(at(2)).await.unwrap();
    let following = client.device(N).await.unwrap();
    let mut a = client.watch([pinned.point("ai:1").unwrap()]).unwrap();
    let mut b = client.watch([following.point("ai:1").unwrap()]).unwrap();
    until(&mut a, |u| u.state == State::Subscribed).await;
    until(&mut b, |u| u.state == State::Subscribed).await;
    assert_eq!(pooled(&client), 1, "one hub for the client's settings");
    assert_eq!(servers[0].cov_counters().subscriptions_active, 2);
    let held = client.shared.ranges.lock().unwrap().held();
    servers.push(server(bus.port(3), N, "new").await);
    bus.i_am(3, N);
    bus.silence(2);
    bus.i_am(2, N + 1);
    let gone = until(&mut a, |u| matches!(u.state, State::Gone { .. })).await;
    assert_eq!(
        gone.state,
        State::Gone {
            why: Why::Moved { to: at(3) }
        }
    );
    let went = until(&mut b, |u| u.moved.is_some()).await;
    assert_eq!(went.at, at(3));
    until(&mut b, |u| u.state == State::Subscribed).await;
    servers[1]
        .set_present_value_local(&ai(1), PropertyValue::Real(54.0))
        .await
        .unwrap();
    until(&mut b, |u| number(u) == Some(54.0)).await;
    assert_eq!(
        client.shared.ranges.lock().unwrap().held(),
        held,
        "no range taken"
    );
    let subscribed_there = requests(&sent, SUBSCRIBE, 2);
    drop((a, b, pinned, following));
    settle().await;
    tokio::time::sleep(Duration::from_secs(40)).await;
    assert_eq!(pooled(&client), 0, "the hub went");
    assert_eq!(
        client.shared.ranges.lock().unwrap().held(),
        0,
        "its range back"
    );
    assert_eq!(
        servers[1].cov_counters().subscriptions_active,
        0,
        "cancelled"
    );
    assert_eq!(
        requests(&sent, SUBSCRIBE, 2),
        subscribed_there,
        "nothing sent to station 2 for the moved device"
    );
    bus.unsilence(2);
    stop(servers).await;
}

/// Another device took station 2 unannounced while the device answers at
/// station 3: the hub, told of a move there, reads station 3 (the device),
/// asks station 2 once more (another device), and the following point,
/// subscribed until then, goes to station 3. Nothing is cancelled at
/// station 2, whose device never held the subscription (#2801).
#[tokio::test(start_paused = true)]
async fn an_old_address_naming_another_device_hands_the_point_over_uncancelled() {
    let mut lab = Lab::new(Handle::Follows).await;
    stop(vec![lab.servers.remove(0)]).await;
    lab.servers
        .push(server(lab.bus.port(2), N + 1, "taker").await);
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.i_am(3, N);
    settle().await;
    let before = requests(&lab.sent, SUBSCRIBE, 2);
    lab.tell(3);
    let went = until(&mut lab.watch, |u| {
        assert!(!matches!(u.state, State::Gone { .. }), "{u:?}");
        u.moved.is_some()
    })
    .await;
    assert_eq!(went.at, at(3));
    assert_eq!(lab.client.said(&at(2)), Some(N + 1));
    until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    assert_eq!(
        requests(&lab.sent, SUBSCRIBE, 2),
        before,
        "nothing sent at station 2"
    );
    assert_eq!(lab.servers[0].cov_counters().subscriptions_active, 0);
    assert_eq!(lab.servers[1].cov_counters().subscriptions_active, 1);
    lab.stop().await;
}

/// A following point placed by a stale or forged table row (device N at
/// station 2, where device N + 1 answers): its start asks the device there
/// which it is, and its answer, another instance, gives the point up there
/// before anything of the start is taken. No value of the device at
/// station 2 is ever the point's, and nothing is subscribed there.
#[tokio::test(start_paused = true)]
async fn a_following_point_takes_nothing_from_another_device_at_its_start() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    let taker = server(bus.port(2), N + 1, "taker").await;
    taker
        .set_present_value_local(&ai(1), PropertyValue::Real(999.0))
        .await
        .unwrap();
    bus.i_am(2, N);
    settle().await;
    let point = Point {
        device: Binding::Follows {
            instance: N,
            network: None,
        },
        object: ai(1),
        watched: Watched::Object,
    };
    let mut watch = client.watch([point.clone()]).unwrap();
    let update = until(&mut watch, |u| {
        assert_ne!(number(u), Some(999.0), "{u:?}");
        !matches!(u.state, State::Starting)
    })
    .await;
    assert_eq!(
        (update.point, update.at, update.state),
        (
            point,
            at(2),
            State::Unreachable {
                why: Why::Replaced {
                    watched: N,
                    found: N + 1
                }
            }
        )
    );
    settle().await;
    assert_eq!(taker.cov_counters().subscriptions_active, 0, "none asked");
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_secs(10)).await;
        while let Some(update) = ready(&mut watch) {
            assert_ne!(number(&update), Some(999.0), "{update:?}");
            assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
        }
    }
    assert_eq!(taker.cov_counters().subscriptions_active, 0);
    drop(watch);
    settle().await;
    stop(vec![taker]).await;
}

/// Concurrency: a move the hub takes racing the point's removal on another
/// thread, at a spread of delays. Whichever comes first, the removed point
/// is never left subscribed at the new address, nothing of it is handed
/// out after `remove` returned, and the hub ends once the watch goes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_move_racing_a_removal_leaves_nothing_at_the_new_address() {
    for delay in [0u64, 1, 2, 5, 10] {
        let mut lab = Lab::new(Handle::Follows).await;
        let control = lab.watch.control();
        let point = lab.device.as_ref().unwrap().point("ai:1").unwrap();
        stop(vec![lab.servers.remove(0)]).await;
        lab.servers
            .push(server(lab.bus.port(2), N + 1, "taker").await);
        lab.servers.push(server(lab.bus.port(3), N, "moved").await);
        settle().await;
        lab.tell(3);
        let removing = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            control.remove([&point]).unwrap();
            control
        });
        let control = removing.await.unwrap();
        // The watch's one point is gone: nothing comes after `remove`.
        assert!(ready(&mut lab.watch).is_none(), "{delay} ms");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while lab.servers[1].cov_counters().subscriptions_active > 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{delay} ms: left subscribed at the new address"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(ready(&mut lab.watch).is_none(), "{delay} ms");
        drop(control);
        let client = lab.client.clone();
        lab.stop().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
        while pooled(&client) > 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{delay} ms: the hub"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
