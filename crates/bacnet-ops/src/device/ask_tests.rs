//! A following handle asking where its device went (#2560): after a
//! request times out it sends one Who-Is for the instance, at most one
//! every `QUIET`, and a later timeout moves it only if the other address
//! answered and its own has been silent since. A clone that sends no
//! Who-Is goes by the I-Am times instead. On the in-memory link
//! (`test_bus`).

use std::time::Duration;

use bacnet_types::error::Error;

use super::test_bus::{Bus, N, at, client, mac, name_of, server, settle, stop, who_is};
use super::*;
use crate::client::{RESOLVE_WAIT, timed_out};

/// A device that moves soon after its last I-Am, within `QUIET` of it, is
/// followed all the same: the timeout asks once where it went, and the
/// next timeout, the Who-Is answered by the new address alone, moves the
/// handle there (the reviewer's probe P2).
#[tokio::test(start_paused = true)]
async fn a_move_soon_after_an_i_am_is_followed() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let old = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    tokio::time::sleep(Duration::from_secs(10)).await;
    bus.silence(2);
    let new = server(bus.port(3), N, "B").await;
    bus.i_am(3, N);
    settle().await;
    let mut events = client.events();
    let error = name_of(&device).await.unwrap_err();
    assert!(timed_out(&error), "{error}");
    assert_eq!(device.address(), at(2), "one timeout moves nothing");
    assert_eq!(who_is(&sent), 1, "one Who-Is asks where it went");
    // A second timeout before the Who-Is has had the resolve wait for its
    // answers decides nothing yet.
    assert!(name_of(&device).await.is_err());
    assert_eq!(device.address(), at(2), "the answers aren't all in");
    tokio::time::sleep(RESOLVE_WAIT).await;
    assert_eq!(name_of(&device).await.unwrap(), "B");
    assert_eq!(device.address(), at(3));
    assert_eq!(
        events.try_recv().unwrap(),
        ClientEvent::Moved {
            instance: N,
            from: at(2),
            to: at(3),
        }
    );
    assert_eq!(who_is(&sent), 1, "asked once");
    stop(vec![old, new]).await;
}

/// A device gone with no other address claiming it costs one Who-Is every
/// `QUIET`, however often it is read.
#[tokio::test(start_paused = true)]
async fn a_gone_device_is_asked_for_once_each_quiet() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let lab = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    let alongside = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    bus.silence(2);
    for handle in [&device, &alongside, &device, &alongside, &device] {
        let error = name_of(handle).await.unwrap_err();
        assert!(timed_out(&error), "{error}");
    }
    assert_eq!(who_is(&sent), 1, "one Who-Is for five reads by two handles");
    tokio::time::sleep(QUIET).await;
    assert!(name_of(&device).await.is_err());
    assert_eq!(who_is(&sent), 2, "another once QUIET has passed");
    assert_eq!(device.address(), at(2));
    stop(vec![lab]).await;
}

/// A clone that sends no Who-Is never asks: its following handle goes by
/// the I-Am times alone, as `moved_to` does, staying while the other
/// address announced within `QUIET` of its own and moving once its own
/// last announced itself `QUIET` before the other did.
#[tokio::test(start_paused = true)]
async fn a_clone_that_sends_nothing_goes_by_the_announcements() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
    ];
    bus.i_am(2, N);
    settle().await;
    let device = client.with_resolve_wait(None).device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    bus.i_am(3, N);
    settle().await;
    bus.silence(2);
    let error = name_of(&device).await.unwrap_err();
    assert!(timed_out(&error), "{error}");
    assert_eq!(device.address(), at(2), "announced together: no move");
    tokio::time::sleep(QUIET * 2).await;
    bus.i_am(3, N);
    settle().await;
    assert_eq!(name_of(&device).await.unwrap(), "B");
    assert_eq!(device.address(), at(3));
    assert_eq!(who_is(&sent), 0, "never asked");
    stop(servers).await;
}

/// A duplicate that announces itself long after the followed device last
/// did doesn't take the handle when the device misses a read: the Who-Is
/// that timeout sends goes unanswered by the device, but before a second
/// timeout the device is heard again, and from then on missing a read
/// moves nothing (the reviewer's probe P3: the handle ends on the device).
#[tokio::test(start_paused = true)]
async fn a_late_duplicate_does_not_take_the_handle_on_a_missed_read() {
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
    tokio::time::sleep(QUIET * 2).await;
    bus.i_am(3, N);
    settle().await;
    let mut events = client.events();
    bus.silence(2);
    let error = name_of(&device).await.unwrap_err();
    assert!(timed_out(&error), "{error}");
    assert_eq!(
        device.address(),
        at(2),
        "the announcements alone move nothing"
    );
    // A Who-Is hears them both.
    bus.unsilence(2);
    bus.i_am(2, N);
    bus.i_am(3, N);
    settle().await;
    assert_eq!(name_of(&device).await.unwrap(), "A");
    // Heard since the Who-Is went: missing another read moves nothing.
    tokio::time::sleep(RESOLVE_WAIT).await;
    bus.silence(2);
    assert!(name_of(&device).await.is_err());
    assert_eq!(device.address(), at(2));
    assert!(events.try_recv().is_err(), "nothing moved");
    assert!(matches!(
        client.device(N).await,
        Err(Error::AmbiguousDevice { instance: N, .. })
    ));
    stop(servers).await;
}

/// A row registered by hand has no I-Am time, and that alone makes it no
/// quieter than a device that just announced the same instance: after a
/// timeout a following handle stays, and so does one of a clone that
/// sends nothing, and `moved_to` names no move (the reviewer's probe P4).
#[tokio::test(start_paused = true)]
async fn a_row_registered_by_hand_is_not_quiet_on_its_own() {
    let bus = Bus::new();
    let (client, _) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "B").await,
    ];
    client.low_level().add_device(N, &mac(2)).await.unwrap();
    let asking = client.device(N).await.unwrap();
    let silent = client.with_resolve_wait(None).device(N).await.unwrap();
    let pinned = client.device(at(2)).await.unwrap();
    assert_eq!(name_of(&asking).await.unwrap(), "A");
    bus.i_am(3, N);
    settle().await;
    assert_eq!(pinned.moved_to().await, None);
    bus.silence(2);
    for handle in [&silent, &asking] {
        let error = name_of(handle).await.unwrap_err();
        assert!(timed_out(&error), "{error}");
        assert_eq!(handle.address(), at(2), "{handle:?}");
    }
    stop(servers).await;
}
