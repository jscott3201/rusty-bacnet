//! A following handle moves only on its device's own answer (#2795): the
//! discovery table, which I-Ams nobody asked for fill, can say another
//! device took its address and the instance answers elsewhere, but a
//! handle asks before it believes it. Forged announcements must never take
//! a write somewhere else.

use std::time::Duration;

use bacnet_types::enums::ConfirmedServiceChoice;

use super::test_bus::{Bus, N, at, client, name_of, requests, server, settle, stop};
use super::*;
use crate::client::RESOLVE_WAIT;

const WRITE: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;
const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

/// The forged pair: an I-Am of another device from the handle's address,
/// and one of its instance from station 3, where nothing answers, or where
/// a responder answers as the device (an impostor's own host). The device
/// at station 2 still answers as itself when asked: the handle stays, and
/// the write goes there, never to station 3.
#[tokio::test(start_paused = true)]
async fn a_forged_pair_takes_no_write_elsewhere() {
    for responder in [false, true] {
        let bus = Bus::new();
        let (client, sent) = client(&bus).await;
        let mut servers = vec![server(bus.port(2), N, "A").await];
        if responder {
            servers.push(server(bus.port(3), N, "impostor").await);
        }
        bus.i_am(2, N);
        settle().await;
        let device = client.device(N).await.unwrap();
        assert_eq!(name_of(&device).await.unwrap(), "A");
        let mut events = client.events();
        bus.i_am(2, N + 1);
        bus.i_am(3, N);
        settle().await;
        device.write("ai:1 oos", "true", None).await.unwrap();
        assert_eq!(requests(&sent, WRITE, 2), 1, "{responder}: the write at A");
        assert_eq!(requests(&sent, WRITE, 3), 0, "{responder}: none elsewhere");
        assert_eq!(requests(&sent, READ, 3), 0, "{responder}: B never asked");
        assert_eq!(device.address(), at(2));
        assert!(events.try_recv().is_err(), "{responder}: nothing moved");
        // The device's answer is remembered: the next request asks nothing.
        let asks = requests(&sent, READ, 2);
        assert_eq!(name_of(&device).await.unwrap(), "A");
        assert_eq!(requests(&sent, READ, 2) - asks, 1, "the read alone");
        stop(servers).await;
    }
}

/// Re-resolving the instance while the table holds the forged pair (one
/// claim, at station 3) moves no handle following it: the new handle joins
/// the place the others reach the device at.
#[tokio::test(start_paused = true)]
async fn re_resolving_moves_no_live_handle() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    let servers = vec![server(bus.port(2), N, "A").await];
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    let mut events = client.events();
    bus.i_am(2, N + 1);
    bus.i_am(3, N);
    settle().await;
    let again = client.device(N).await.unwrap();
    assert_eq!((device.address(), again.address()), (at(2), at(2)));
    assert!(events.try_recv().is_err(), "nothing moved");
    assert_eq!(name_of(&again).await.unwrap(), "A");
    stop(servers).await;
}

/// The device stops answering at station 2 and station 3 claims its
/// instance (a forged I-Am after the handle's Who-Is): after the next
/// timeout the handle asks station 3, which doesn't answer, and stays.
#[tokio::test(start_paused = true)]
async fn a_candidate_that_doesnt_name_the_device_takes_no_handle() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let servers = vec![server(bus.port(2), N, "A").await];
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    tokio::time::sleep(QUIET * 2).await;
    bus.silence(2);
    let mut events = client.events();
    assert!(name_of(&device).await.unwrap_err().timed_out());
    bus.i_am(3, N);
    tokio::time::sleep(RESOLVE_WAIT).await;
    assert!(name_of(&device).await.unwrap_err().timed_out());
    assert_eq!(requests(&sent, READ, 3), 1, "station 3 asked once");
    assert_eq!(device.address(), at(2), "the handle stays");
    assert!(events.try_recv().is_err(), "nothing moved");
    bus.unsilence(2);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(name_of(&device).await.unwrap(), "A");
    stop(servers).await;
}

/// Another device took the handle's address and the instance's only other
/// claim is a station where nothing answers: a write fails as not
/// confirmed, naming that station, with the address's device asked and
/// nothing sent anywhere else (#2795); a read asks that station and fails
/// as unknown.
#[tokio::test(start_paused = true)]
async fn a_reused_address_with_a_silent_claim_sends_no_write() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let mut first = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    first.stop().await.unwrap();
    let other = server(bus.port(2), N + 1, "other").await;
    bus.i_am(2, N + 1);
    bus.i_am(3, N);
    settle().await;
    let error = device.write("ai:1 oos", "true", None).await.unwrap_err();
    assert!(
        matches!(&error, OpsError::NotConfirmed { candidate: Some(c), .. } if *c == at(3)),
        "{error}"
    );
    assert_eq!(requests(&sent, WRITE, 2) + requests(&sent, WRITE, 3), 0);
    assert_eq!(
        requests(&sent, READ, 3),
        0,
        "a write asks nothing elsewhere"
    );
    let error = name_of(&device).await.unwrap_err();
    assert!(matches!(error, OpsError::UnknownDevice { .. }), "{error}");
    assert_eq!(requests(&sent, READ, 3), 1, "a read asks station 3");
    assert_eq!(device.address(), at(2));
    stop(vec![other]).await;
}
