//! Following handles (#2560): when one moves, when it stays, and what the
//! client keeps for them, on the in-memory link (`test_bus`).

use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;

use super::follow::{REMEMBERED, keep_room};
use super::test_bus::{Bus, N, at, client, name_of, requests, server, settle, stop, who_is};
use super::*;
use crate::client::RESOLVE_WAIT;

/// A following handle whose device has gone quiet and stops answering
/// moves to the one other address claiming its instance once only that
/// address answered the Who-Is its first timeout sent: it says so, and
/// reads there.
#[tokio::test(start_paused = true)]
async fn a_following_handle_moves_to_the_one_other_claim() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
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
    let error = name_of(&device).await.unwrap_err();
    assert!(error.timed_out(), "{error}");
    assert_eq!(device.address(), at(2), "the timeout asked, and stayed");
    assert_eq!(who_is(&sent), 1);
    tokio::time::sleep(RESOLVE_WAIT).await;
    assert_eq!(name_of(&device).await.unwrap(), "B");
    assert_eq!(device.address(), at(3));
    assert_eq!(alongside.address(), at(3), "the handles move together");
    assert_eq!(who_is(&sent), 1, "asked once");
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
    assert!(error.timed_out(), "{error}");
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
        matches!(error, OpsError::Ambiguous { instance: N, .. }),
        "{error}"
    );
    assert_eq!(device.address(), at(2));
    assert!(events.try_recv().is_err(), "nothing moved");
    stop(servers).await;
}

/// No other address claims the instance: the one request times out and
/// goes nowhere else, and one Who-Is asks where the device went (the
/// reviewer's probe D).
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
    assert!(error.unreachable(), "{error}");
    assert_eq!(requests(&sent, rp, 2) - before, 1, "one request");
    assert_eq!(device.address(), at(2));
    assert_eq!(who_is(&sent), 1);
    stop(vec![lab]).await;
}

/// Another device announcing itself at a following handle's address takes
/// the address, not the handle: before its next request the handle asks
/// the device there which it is (another), then the one other address
/// claiming the instance (the device), and only then moves there, a clone
/// with it; the request itself never goes to the old address (the
/// reviewer's probe P1, #2795).
#[tokio::test(start_paused = true)]
async fn a_reused_address_moves_a_following_handle_before_it_sends() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let mut first = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    let copy = device.clone();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    // Device N leaves station 2 for station 3; device 2002 takes station 2.
    first.stop().await.unwrap();
    let servers = vec![
        server(bus.port(2), 2002, "other").await,
        server(bus.port(3), N, "B").await,
    ];
    bus.i_am(2, 2002);
    bus.i_am(3, N);
    settle().await;
    let mut events = client.events();
    let rp = ConfirmedServiceChoice::READ_PROPERTY;
    let (before, there) = (requests(&sent, rp, 2), requests(&sent, rp, 3));
    assert_eq!(name_of(&device).await.unwrap(), "B");
    assert_eq!(requests(&sent, rp, 2) - before, 1, "one ask at station 2");
    assert_eq!(
        requests(&sent, rp, 3) - there,
        2,
        "the ask there, then the read"
    );
    assert_eq!(device.address(), at(3));
    assert_eq!(copy.address(), at(3), "a clone moves with it");
    assert_eq!(
        events.try_recv().unwrap(),
        ClientEvent::Moved {
            instance: N,
            from: at(2),
            to: at(3),
        }
    );
    stop(servers).await;
}

/// Another device announcing itself at a following handle's address, with
/// no other address claiming the instance: the device there is asked which
/// it is, and as another, the request fails as unknown and isn't sent
/// (#2795).
#[tokio::test(start_paused = true)]
async fn a_reused_address_with_no_other_claim_fails_unsent() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let mut first = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    first.stop().await.unwrap();
    let other = server(bus.port(2), 2002, "other").await;
    bus.i_am(2, 2002);
    settle().await;
    let before = sent.len();
    let error = name_of(&device).await.unwrap_err();
    assert!(
        matches!(&error, OpsError::UnknownDevice { target, asked: None } if target == "1001"),
        "{error}"
    );
    assert_eq!(sent.len() - before, 1, "only the question");
    assert_eq!(device.address(), at(2));
    stop(vec![other]).await;
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
    assert!(
        matches!(
            error,
            OpsError::Unreachable {
                source: Error::Transport(_),
                ..
            }
        ),
        "{error}"
    );
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

/// A bulk read of a following handle whose device moved reads again where
/// it went: the first batch timed out with nothing read, the handle moved
/// to the one other address that answered the Who-Is an earlier timeout
/// sent, and the readings come from there, with no stop.
#[tokio::test(start_paused = true)]
async fn a_bulk_read_follows_a_device_that_moved() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let old = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    tokio::time::sleep(QUIET * 2).await;
    let new = server(bus.port(3), N, "B").await;
    bus.i_am(3, N);
    settle().await;
    bus.silence(2);
    // The first timeout asks where the device went, and stays.
    assert!(name_of(&device).await.unwrap_err().timed_out());
    tokio::time::sleep(RESOLVE_WAIT).await;
    let read = device.read_many(["ai:1 object-name"]).await.unwrap();
    assert!(read.stopped.is_none(), "{read:?}");
    let name = read.readings[0].value.as_ref().map(|v| v.as_str());
    assert_eq!(name, Ok(Some("B")), "{read:?}");
    assert_eq!(device.address(), at(3));
    stop(vec![old, new]).await;
}

/// A write whose request timed out may have been applied, so it is never
/// sent again, even by a following handle whose device another address
/// now claims: one WriteProperty, to the address it went to, and the
/// timeout as the error.
#[tokio::test(start_paused = true)]
async fn a_timed_out_write_is_never_sent_again() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let old = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    tokio::time::sleep(QUIET * 2).await;
    let new = server(bus.port(3), N, "B").await;
    bus.i_am(3, N);
    settle().await;
    bus.silence(2);
    // A read's timeout asks where the device went; only station 3 answers.
    assert!(name_of(&device).await.unwrap_err().timed_out());
    tokio::time::sleep(RESOLVE_WAIT).await;
    let wp = ConfirmedServiceChoice::WRITE_PROPERTY;
    let error = device.write("ai:1 oos", "true", None).await.unwrap_err();
    assert!(error.timed_out(), "{error}");
    assert_eq!(requests(&sent, wp, 2), 1, "one write, where it went");
    assert_eq!(requests(&sent, wp, 3), 0, "never sent again elsewhere");
    stop(vec![old, new]).await;
}
