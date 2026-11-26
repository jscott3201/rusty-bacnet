//! Two claimants (#2812): a new address naming the watched device is a
//! move only when the old address doesn't answer as it any more. The hub
//! asks the old address once more: still the device, nothing changes (a
//! responder elsewhere, an impostor's own host, ends no watch); timing
//! out, the points are given up as moved; another device, they are given
//! up for it, then said to have moved; the link failing the ask, nothing
//! changes. The reads' pace belongs to the watched address, whichever
//! addresses are named.

use std::time::Duration;

use super::State;
use super::move_cue_tests::{Handle, Lab, moved_to};
use super::stream_tests::{number, until};
use crate::device::ClientEvent;
use crate::device::test_bus::{N, at, server, settle, stop};

/// A responder at station 3 answers as the watched device, and the forged
/// pair names it there. Station 2 answered the hub's ask about the forged
/// claim as the device, which the client remembers: the handle's next
/// request goes there without asking again, and station 3 is never asked
/// (#2795). The watch goes on.
#[tokio::test(start_paused = true)]
async fn a_responder_naming_the_device_elsewhere_ends_no_watch() {
    let mut lab = Lab::new(Handle::Follows).await;
    let impostor = server(lab.bus.port(3), N, "impostor").await;
    lab.servers.push(impostor);
    lab.forge();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let before = lab.reads_at(2);
    let device = lab.device.as_ref().unwrap();
    device.read("ai:1 pv").await.unwrap();
    assert_eq!(device.address(), at(2), "the handle stays");
    lab.goes_on(31.0).await;
    lab.none_given_up(10).await;
    assert_eq!(lab.reads_at(3), 0, "station 3 never asked");
    assert_eq!(
        lab.reads_at(2) - before,
        0,
        "the device's answer remembered"
    );
    assert_eq!(lab.servers[1].cov_counters().subscriptions_active, 0);
    lab.goes_on(32.0).await;
    lab.stop().await;
}

/// The same with station 2 gone quiet after it answered the forged claim:
/// the handle's read there times out, the handle asks station 2 once
/// more (silent) and station 3 (the device), and moves there. The hub,
/// told, reads station
/// 3, asks station 2 once more, which doesn't answer, and the following
/// point goes to station 3.
#[tokio::test(start_paused = true)]
async fn a_responder_naming_the_device_is_a_move_once_its_old_address_is_silent() {
    let mut lab = Lab::new(Handle::Follows).await;
    let moved = server(lab.bus.port(3), N, "moved").await;
    lab.servers.push(moved);
    lab.forge();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let before = lab.reads_at(2);
    lab.bus.silence(2);
    lab.device.as_ref().unwrap().read("ai:1 pv").await.unwrap();
    assert_eq!(lab.device.as_ref().unwrap().address(), at(3));
    let update = until(&mut lab.watch, |u| u.moved.is_some()).await;
    assert_eq!(update.at, at(3), "{update:?}");
    assert!(lab.reads_at(3) >= 2, "the handle's read and the hub's");
    assert_eq!(
        lab.reads_at(2) - before,
        2,
        "the handle's ask and the hub's"
    );
    until(&mut lab.watch, |u| u.state == State::Subscribed).await;
    lab.bus.unsilence(2);
    lab.stop().await;
}

/// Another device took station 2 unannounced, and the watched one answers
/// at station 3: asked once more, station 2 names the other, so the point
/// is given up for it (its values dropped, as for any device replaced),
/// then said to have moved to station 3.
#[tokio::test(start_paused = true)]
async fn an_old_address_naming_another_device_gives_the_point_up_then_moved() {
    let mut lab = Lab::new(Handle::Pinned).await;
    stop(vec![lab.servers.remove(0)]).await;
    lab.servers
        .push(server(lab.bus.port(2), N + 1, "other").await);
    lab.servers.push(server(lab.bus.port(3), N, "moved").await);
    lab.bus.i_am(3, N);
    settle().await;
    lab.tell(3);
    let update = until(&mut lab.watch, |u| matches!(u.state, State::Gone { .. })).await;
    assert_eq!(moved_to(&update), Some(&at(3)), "{update:?}");
    assert_eq!(number(&update), None, "given up for the other first");
    assert_eq!(lab.client.said(&at(2)), Some(N + 1));
    lab.stop().await;
}

/// A cue that waited for its turn: a move told to station 4, where nothing
/// answers, spends the turn; one told to station 3, where a responder names
/// the device, waits and is read a minute later. Station 2, asked once
/// more then, still answers as the device: the watch goes on.
#[tokio::test(start_paused = true)]
async fn a_cue_read_in_its_turn_asks_the_old_address_again() {
    let mut lab = Lab::new(Handle::Follows).await;
    let impostor = server(lab.bus.port(3), N, "impostor").await;
    lab.servers.push(impostor);
    lab.tell(4);
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lab.reads_at(4), 1, "station 4 read at once");
    lab.tell(3);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(lab.reads_at(3), 0, "station 3 waits its turn");
    let before = lab.reads_at(2);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(lab.reads_at(3), 1, "read in its turn");
    assert_eq!(lab.reads_at(2) - before, 1, "station 2 asked again");
    lab.goes_on(33.0).await;
    lab.none_given_up(5).await;
    lab.stop().await;
}

/// The forged pair every 20 s for an hour, naming the next of ten silent
/// stations each time, and the hub told each time that a handle moved
/// there: the pace is the watched address's, so the hub reads the stations
/// 14 times in the hour between them, and the watch goes on. The pairs
/// move no handle (#2795).
#[tokio::test(start_paused = true)]
async fn rotating_new_addresses_share_the_watched_addresss_pace() {
    let mut lab = Lab::new(Handle::Follows).await;
    let stations = 3..=12u8;
    let mut moves = 0;
    for round in 0..180u8 {
        let station = 3 + round % 10;
        lab.bus.i_am(2, N + 1);
        lab.bus.i_am(station, N);
        settle().await;
        let there = lab.client.device(N).await.unwrap();
        while let Ok(event) = lab.events.try_recv() {
            moves += usize::from(matches!(event, ClientEvent::Moved { from, .. } if from == at(2)));
        }
        lab.tell(station);
        // The test's own word to the hub, not a handle's move.
        while lab.events.try_recv().is_ok() {}
        lab.bus.i_am(2, N);
        lab.bus.i_am(station, N + 2);
        settle().await;
        drop(there);
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
    assert_eq!(moves, 0, "no handle moved");
    let reads: usize = stations.map(|station| lab.reads_at(station)).sum();
    // At once, after 60, 120 and 240 s, then every 300 s.
    assert_eq!(reads, 14, "reads at the stations in the hour");
    lab.goes_on(34.0).await;
    lab.stop().await;
}

/// A move told while the scheduler is busy (asking station 2, which doesn't
/// answer, about a forged claim), its watch dropped before the read's turn
/// comes: nothing is read at the new address.
#[tokio::test(start_paused = true)]
async fn a_cue_whose_watch_is_dropped_before_its_turn_reads_nothing() {
    let lab = Lab::new(Handle::Pinned).await;
    lab.bus.silence(2);
    lab.bus.i_am(2, N + 1);
    settle().await;
    lab.tell(3);
    settle().await;
    let Lab {
        bus,
        servers,
        sent,
        device,
        watch,
        ..
    } = lab;
    drop((watch, device));
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        super::move_cue_tests::wildcard_reads(&sent, 3),
        0,
        "nothing read there"
    );
    bus.unsilence(2);
    stop(servers).await;
}

/// A handle's move to station 3, where a responder names the device, while
/// the link fails every send to station 2: the ask there fails at the
/// client, which says nothing of the device, and the watch goes on.
#[tokio::test(start_paused = true)]
async fn an_ask_the_link_fails_changes_nothing() {
    let mut lab = Lab::new(Handle::Follows).await;
    let impostor = server(lab.bus.port(3), N, "impostor").await;
    lab.servers.push(impostor);
    lab.bus.break_link(2);
    let before = lab.reads_at(2);
    lab.tell(3);
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(lab.reads_at(3), 1, "one read at station 3");
    assert_eq!(
        lab.reads_at(2) - before,
        1,
        "the ask, which the link failed"
    );
    lab.goes_on(35.0).await;
    lab.none_given_up(1).await;
    lab.stop().await;
}
