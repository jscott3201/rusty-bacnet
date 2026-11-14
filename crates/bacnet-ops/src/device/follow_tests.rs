//! Following handles (#2560): when one moves, when it stays, and what the
//! client keeps for them, on the in-memory link (`test_bus`).

use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;

use super::follow::{REMEMBERED, keep_room};
use super::test_bus::{Bus, N, at, client, name_of, requests, server, settle, stop};
use super::*;
use crate::client::{timed_out, unreachable};

/// A following handle whose device has gone quiet and stops answering
/// moves to the one other address claiming its instance, says so, and
/// reads there.
#[tokio::test(start_paused = true)]
async fn a_following_handle_moves_to_the_one_other_claim() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let old = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    let alongside = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    // Station 2 last announced itself long before the new one did.
    tokio::time::sleep(QUIET * 2).await;
    let new = server(bus.port(3), N, "B").await;
    bus.i_am(3, N);
    settle().await;
    bus.silence(2);
    let mut events = client.events();
    assert_eq!(name_of(&device).await.unwrap(), "B");
    assert_eq!(device.address(), at(3));
    assert_eq!(alongside.address(), at(3), "the handles move together");
    assert_eq!(
        events.try_recv().unwrap(),
        ClientEvent::Moved {
            instance: N,
            from: at(2),
            to: at(3),
        }
    );
    assert!(events.try_recv().is_err(), "one move, told once");
    stop(vec![old, new]).await;
}

/// A duplicate claiming the instance while the followed address still
/// announces it is no move: one timeout there leaves the handle where it
/// was, and it reads the device again once it answers (the reviewer's
/// probe B).
#[tokio::test(start_paused = true)]
async fn a_duplicate_does_not_capture_a_following_handle() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
    ];
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    // The duplicate appears: commissioning, or a forged I-Am.
    bus.i_am(3, N);
    settle().await;
    let mut events = client.events();
    bus.silence(2);
    let error = name_of(&device).await.unwrap_err();
    assert!(timed_out(&error), "{error}");
    bus.unsilence(2);
    assert_eq!(name_of(&device).await.unwrap(), "A");
    assert_eq!(device.address(), at(2));
    assert!(events.try_recv().is_err(), "nothing moved");
    stop(servers).await;
}

/// Two other addresses claiming the instance leave a following handle where
/// it was, with the ambiguity as its error, and nothing moved.
#[tokio::test(start_paused = true)]
async fn a_following_handle_stays_when_two_others_claim_it() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
        server(bus.port(4), N, "C").await,
    ];
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    bus.i_am(3, N);
    bus.i_am(4, N);
    settle().await;
    bus.silence(2);
    let mut events = client.events();
    let error = name_of(&device).await.unwrap_err();
    assert!(
        matches!(error, Error::AmbiguousDevice { instance: N, .. }),
        "{error}"
    );
    assert_eq!(device.address(), at(2));
    assert!(events.try_recv().is_err(), "nothing moved");
    stop(servers).await;
}

/// No other address claims the instance: the one request times out and
/// nothing is sent again (the reviewer's probe D).
#[tokio::test(start_paused = true)]
async fn no_other_claim_sends_once() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    bus.silence(2);
    let rp = ConfirmedServiceChoice::READ_PROPERTY;
    let before = requests(&sent, rp, 2);
    let error = name_of(&device).await.unwrap_err();
    assert!(unreachable(&error), "{error}");
    assert_eq!(requests(&sent, rp, 2) - before, 1, "one request");
    assert_eq!(device.address(), at(2));
    stop(vec![lab]).await;
}

/// A send that fails at this end of the link says nothing of the device:
/// the handle stays, though another address could take the instance.
#[tokio::test(start_paused = true)]
async fn a_local_link_failure_moves_nothing() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
    ];
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    tokio::time::sleep(QUIET * 2).await;
    bus.i_am(3, N);
    settle().await;
    bus.break_link(2);
    let error = name_of(&device).await.unwrap_err();
    assert!(matches!(error, Error::Transport(_)), "{error}");
    assert_eq!(device.address(), at(2));
    let rp = ConfirmedServiceChoice::READ_PROPERTY;
    assert_eq!(requests(&sent, rp, 3), 0, "nothing went to the other");
    stop(servers).await;
}

/// The follow table holds an instance while a handle follows it, and lets
/// it go with the last one.
#[tokio::test(start_paused = true)]
async fn the_follow_table_lets_go_with_its_handles() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    let live = |client: &Client<_>| {
        let follows = lock(&client.shared.follows);
        follows
            .values()
            .filter(|entry| entry.strong_count() > 0)
            .count()
    };
    assert_eq!(live(&client), 1);
    drop(device);
    assert_eq!(live(&client), 0);
    // A lost row prunes the dead entry.
    bus.i_am(2, 2002);
    settle().await;
    client.sweep();
    assert!(lock(&client.shared.follows).is_empty());
    stop(vec![lab]).await;
}

/// The client's tables stay under their cap: entries nothing uses go
/// first, then any one.
#[test]
fn keep_room_bounds_a_table() {
    let mut map: std::collections::HashMap<u32, bool> = (0..u32::try_from(REMEMBERED).unwrap())
        .map(|n| (n, n == 7))
        .collect();
    keep_room(&mut map, |alive| *alive);
    assert_eq!(map.len(), 1, "only the live entry stays");
    let mut map: std::collections::HashMap<u32, bool> = (0..u32::try_from(REMEMBERED).unwrap())
        .map(|n| (n, true))
        .collect();
    keep_room(&mut map, |alive| *alive);
    assert_eq!(map.len(), REMEMBERED - 1, "room for one more");
}
