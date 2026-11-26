//! A following handle's address is its device's own word (#2795): before
//! it believes the discovery table, or an answer at another address, it
//! asks its own address, and a write never goes anywhere it holds no
//! confirmation for. The threat: anyone can send any frame, and run a
//! responder answering as the device at an address of their own.

use bacnet_types::enums::ConfirmedServiceChoice;

use super::test_bus::{Bus, N, TurnAway, at, client, name_of, requests, server, settle, stop};
use super::*;

const WRITE: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;
const READ: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

/// Device N answers at station 2, a following handle on it; a responder
/// answering as N runs at station 3.
async fn lab() -> (
    std::sync::Arc<Bus>,
    Client<bacnet_transport::test_transport::TestTransport>,
    bacnet_transport::test_transport::SendLog,
    Device<bacnet_transport::test_transport::TestTransport>,
    Vec<bacnet_server::server::BACnetServer<bacnet_transport::test_transport::TestTransport>>,
) {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let servers = vec![
        server(bus.port(2), N, "A").await,
        server(bus.port(3), N, "impostor").await,
    ];
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    assert_eq!(name_of(&device).await.unwrap(), "A");
    (bus, client, sent, device, servers)
}

/// The reviewer's recipe T: a forged I-Am of another device from station
/// 2 and one of N from station 3, and the device at station 2 misses the
/// handle's one question. A write goes nowhere: the handle holds no word
/// of the device's at station 2 now, and a write never moves.
#[tokio::test(start_paused = true)]
async fn a_write_never_goes_to_the_claimed_address() {
    let (bus, _client, sent, device, servers) = lab().await;
    bus.i_am(2, N + 1);
    bus.i_am(3, N);
    settle().await;
    bus.lose(2, 1);
    let error = device.write("ai:1 oos", "true", None).await.unwrap_err();
    assert!(
        matches!(
            &error,
            OpsError::NotConfirmed { instance: N, at: here, candidate: Some(there) }
                if *here == at(2) && *there == at(3)
        ),
        "{error:?}"
    );
    assert_eq!(requests(&sent, WRITE, 2) + requests(&sent, WRITE, 3), 0);
    assert_eq!(requests(&sent, READ, 3), 0, "station 3 never asked");
    assert_eq!(device.address(), at(2));
    stop(servers).await;
}

/// The reviewer's recipe R: station 2's row ages out (here, the table is
/// cleared), station 3 announces N, and one read at station 2 is lost.
/// The handle asks station 2, which still answers as N: it stays, and
/// its next write goes there, never to station 3.
#[tokio::test(start_paused = true)]
async fn a_lost_read_moves_no_handle_while_its_address_answers() {
    let (bus, client, sent, device, servers) = lab().await;
    client.low_level().clear_devices().await;
    bus.i_am(3, N);
    settle().await;
    bus.lose(2, 1);
    assert!(name_of(&device).await.unwrap_err().timed_out());
    assert_eq!(device.address(), at(2), "station 2 still answers as N");
    device.write("ai:1 oos", "true", None).await.unwrap();
    assert_eq!(requests(&sent, WRITE, 2), 1);
    assert_eq!(requests(&sent, WRITE, 3) + requests(&sent, READ, 3), 0);
    stop(servers).await;
}

/// A read by instance at station 3 (the client takes it for a device
/// refusing the wildcard read) counts only once station 2 was asked and
/// didn't answer as N: here it does, and nothing is read at station 3.
#[tokio::test(start_paused = true)]
async fn a_read_by_instance_elsewhere_counts_only_after_asking_here() {
    let (bus, client, sent, device, servers) = lab().await;
    client.refuse_wildcard(&at(3));
    client.low_level().clear_devices().await;
    bus.i_am(3, N);
    settle().await;
    bus.lose(2, 1);
    assert!(name_of(&device).await.unwrap_err().timed_out());
    assert_eq!(device.address(), at(2));
    assert_eq!(requests(&sent, READ, 3), 0, "station 3 never read");
    stop(servers).await;
}

/// The device really left station 2 (silent), and the one at station 3
/// refuses the wildcard read but has N's Device object: once station 2
/// was asked and stayed silent, that answer counts, and a read moves the
/// handle there. A write after goes where the read confirmed it.
#[tokio::test(start_paused = true)]
async fn a_read_moves_the_handle_once_its_address_is_silent_and_a_write_follows() {
    let (bus, client, sent, device, servers) = lab().await;
    client.refuse_wildcard(&at(3));
    client.low_level().clear_devices().await;
    bus.i_am(3, N);
    settle().await;
    bus.silence(2);
    assert_eq!(name_of(&device).await.unwrap(), "impostor");
    assert_eq!(device.address(), at(3));
    device.write("ai:1 oos", "true", None).await.unwrap();
    assert_eq!(
        requests(&sent, WRITE, 3),
        1,
        "the write goes where the read went"
    );
    assert_eq!(requests(&sent, WRITE, 2), 0);
    stop(servers).await;
}

/// One forged I-Am of another device from station 2, no other claim: the
/// handle asks station 2, which answers as N, and its read goes there
/// (it used to fail unknown, sending nothing, for every following handle).
/// The same with two forged claims of N elsewhere.
#[tokio::test(start_paused = true)]
async fn a_forged_row_alone_blocks_no_following_handle() {
    for claims in [0u8, 2] {
        let (bus, _client, sent, device, servers) = lab().await;
        bus.i_am(2, N + 1);
        for station in 4..4 + claims {
            bus.i_am(station, N);
        }
        settle().await;
        assert_eq!(name_of(&device).await.unwrap(), "A", "{claims} claims");
        assert_eq!(device.address(), at(2));
        assert_eq!(requests(&sent, READ, 3), 0);
        stop(servers).await;
    }
}

/// Another device answers at station 2 now and no address claims N: a
/// write fails as not confirmed with no candidate, the one question to
/// station 2 its only request.
#[tokio::test(start_paused = true)]
async fn a_write_to_a_reused_address_fails_not_confirmed() {
    let bus = Bus::new();
    let (client, sent) = client(&bus).await;
    let mut first = server(bus.port(2), N, "A").await;
    bus.i_am(2, N);
    settle().await;
    let device = client.device(N).await.unwrap();
    first.stop().await.unwrap();
    let other = server(bus.port(2), N + 1, "other").await;
    bus.i_am(2, N + 1);
    settle().await;
    let before = requests(&sent, READ, 2);
    let error = device.write("ai:1 oos", "true", None).await.unwrap_err();
    assert!(
        matches!(
            &error,
            OpsError::NotConfirmed { candidate: None, at: here, .. } if *here == at(2)
        ),
        "{error:?}"
    );
    assert_eq!(requests(&sent, READ, 2) - before, 1, "the question");
    assert_eq!(requests(&sent, WRITE, 2), 0);
    stop(vec![other]).await;
}

/// S1: station 3 claims N, and station 2 turns the handle's questions
/// away with a Reject, then with an Abort: no word of which device it is,
/// so the handle stays and nothing is read at station 3. Before a read
/// (another device announced at station 2) the read goes to station 2;
/// after a lost read (station 2's row aged out) its timeout stands.
#[tokio::test(start_paused = true)]
async fn a_turned_away_question_keeps_the_handle() {
    for how in [TurnAway::Reject, TurnAway::Abort] {
        let (bus, _client, sent, device, servers) = lab().await;
        bus.i_am(2, N + 1);
        bus.i_am(3, N);
        settle().await;
        bus.turn_away(2, 2, how);
        assert_eq!(name_of(&device).await.unwrap(), "A", "{how:?}");
        assert_eq!(device.address(), at(2), "{how:?}");
        assert_eq!(
            requests(&sent, READ, 3),
            0,
            "{how:?}: nothing read at station 3"
        );
        stop(servers).await;

        let (bus, client, sent, device, servers) = lab().await;
        client.low_level().clear_devices().await;
        bus.i_am(3, N);
        settle().await;
        bus.lose(2, 1);
        bus.turn_away(2, 2, how);
        assert!(name_of(&device).await.unwrap_err().timed_out(), "{how:?}");
        assert_eq!(device.address(), at(2), "{how:?}");
        assert_eq!(
            requests(&sent, READ, 3),
            0,
            "{how:?}: nothing read at station 3"
        );
        stop(servers).await;
    }
}
