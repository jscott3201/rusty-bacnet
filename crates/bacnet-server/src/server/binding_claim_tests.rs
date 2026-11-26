//! An I-Am the server did not ask for neither binds a device nor moves one
//! (#2825): a write in another device goes where the server's own Who-Is
//! found it.
//!
//! The harness is `remote_write_discovery_tests`': CMD-1's list 1 writes AO-1
//! in Device 9, which the server holds no binding for. [`STRANGER`] is
//! another station on the link that claims Device 9 in I-Ams of its own.
use super::*;

/// Another station on the link, which claims Device 9 unasked.
const STRANGER: [u8; 6] = [10, 0, 0, 66, 0xBA, 0xC0];

/// The station each WriteProperty the server sent went to, in order.
fn write_stations(h: &Harness) -> Vec<Vec<u8>> {
    let unicasts = h.server.test_network().transport().sent().unicasts();
    unicasts
        .into_iter()
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::ConfirmedRequest(request)
                if request.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY)
        })
        .map(|frame| frame.mac.to_vec())
        .collect()
}

/// Write AO-1 in Device 9 once it is bound, answer the write from the
/// harness peer, and check the write was made.
async fn write_answered(h: &Harness) {
    let write = start_write(h, 9);
    let invoke_id = remote_write(h).await;
    h.respond(ack(invoke_id)).await;
    assert_eq!(write.await.unwrap().0, Ok(()));
}

/// Device 9, bound to the harness peer by its answer to the server's own
/// Who-Is, keeps that binding when another station claims Device 9 in an
/// I-Am the server did not ask for: the next write goes to the peer.
#[tokio::test(start_paused = true)]
async fn a_strangers_i_am_does_not_move_a_bound_command_target() {
    let h = start_unbound().await;
    let first = start_write(&h, 9);
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    deliver(&h, &i_am(9), &PEER, None).await;
    let invoke_id = remote_write(&h).await;
    h.respond(ack(invoke_id)).await;
    assert_eq!(first.await.unwrap().0, Ok(()));

    deliver(&h, &i_am(9), &STRANGER, None).await;
    h.settle().await;
    write_answered(&h).await;
    assert_eq!(write_stations(&h), [PEER.to_vec(), PEER.to_vec()]);
}

/// An I-Am the server did not ask for binds nothing: the first write to
/// Device 9 still asks with its own Who-Is, and goes where the answer to
/// that came from.
#[tokio::test(start_paused = true)]
async fn an_unsolicited_i_am_binds_no_command_target() {
    let h = start_unbound().await;
    deliver(&h, &i_am(9), &STRANGER, None).await;
    h.settle().await;
    let write = start_write(&h, 9);
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    deliver(&h, &i_am(9), &PEER, None).await;
    let invoke_id = remote_write(&h).await;
    h.respond(ack(invoke_id)).await;
    assert_eq!(write.await.unwrap().0, Ok(()));
    assert_eq!(write_stations(&h), [PEER.to_vec()]);
}

/// The device and another station both answer the server's Who-Is: none
/// is bound, the write sends nothing to either and fails as undiscovered,
/// and the contest is counted (decision, maintainer, 2026-11-26).
#[tokio::test(start_paused = true)]
async fn two_answers_to_the_first_who_is_bind_nothing() {
    let h = start_unbound().await;
    let write = start_write(&h, 9);
    assert_eq!(next_who_is(&h).await, (everywhere(), targeted(9)));
    deliver(&h, &i_am(9), &PEER, None).await;
    deliver(&h, &i_am(9), &STRANGER, None).await;
    assert_eq!(
        write.await.unwrap().0,
        Err(RemoteRequestError::Undiscovered)
    );
    assert!(write_stations(&h).is_empty());
    assert_eq!(h.server.device_binding_conflicts().await, 1);
}

/// Device 9 bound to the harness peer by its answer to the server's own
/// Who-Is, its first write answered.
async fn bound_to_peer(h: &Harness) {
    let first = start_write(h, 9);
    assert_eq!(next_who_is(h).await, (everywhere(), targeted(9)));
    deliver(h, &i_am(9), &PEER, None).await;
    let invoke_id = remote_write(h).await;
    h.respond(ack(invoke_id)).await;
    assert_eq!(first.await.unwrap().0, Ok(()));
}

/// The stations each Who-Is sent straight to one went to, all for Device 9.
fn checked(h: &Harness) -> Vec<Vec<u8>> {
    directed_who_is_sent(h)
        .into_iter()
        .map(|(mac, (destination, who_is))| {
            assert_eq!((destination, who_is), (None, targeted(9)));
            mac
        })
        .collect()
}

/// A stranger's claim cues a check that asks both addresses straight; the
/// bound device answers, so the binding stays where it is.
#[tokio::test(start_paused = true)]
async fn a_check_asks_both_addresses_and_the_answering_device_keeps_its_binding() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    deliver(&h, &i_am(9), &STRANGER, None).await;
    h.settle().await;
    assert_eq!(checked(&h), [PEER.to_vec(), STRANGER.to_vec()]);
    deliver(&h, &i_am(9), &PEER, None).await;
    deliver(&h, &i_am(9), &STRANGER, None).await;
    tokio::time::sleep(WAIT).await;
    write_answered(&h).await;
    assert_eq!(write_stations(&h), [PEER.to_vec(), PEER.to_vec()]);
    assert!(who_is_sent(&h).is_empty());
}

/// A device that really moved: its old address is silent through the
/// check and it answers at the new one, so the next write goes there.
#[tokio::test(start_paused = true)]
async fn a_device_that_moved_is_followed_once_its_old_address_stays_silent() {
    const MOVED: [u8; 6] = [10, 0, 0, 19, 0xBA, 0xC0];
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    deliver(&h, &i_am(9), &MOVED, None).await;
    h.settle().await;
    assert_eq!(checked(&h), [PEER.to_vec(), MOVED.to_vec()]);
    deliver(&h, &i_am(9), &MOVED, None).await;
    tokio::time::sleep(WAIT).await;
    let write = start_write(&h, 9);
    let invoke_id = remote_write(&h).await;
    deliver(&h, &ack(invoke_id), &MOVED, None).await;
    assert_eq!(write.await.unwrap().0, Ok(()));
    assert_eq!(write_stations(&h), [PEER.to_vec(), MOVED.to_vec()]);
}

/// Forged I-Ams for Device 9, one every five seconds for an hour, each from
/// an address of its own, while the device answers every check: at most
/// two Who-Is a minute, all straight to one station, none broadcast, and
/// the binding never moves.
#[tokio::test(start_paused = true)]
async fn a_flood_of_forged_i_ams_costs_two_directed_who_is_a_minute() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    let mut asked = 0;
    for forged in 0..720u16 {
        let [high, low] = forged.to_be_bytes();
        let stranger = [10, 1, high, low, 0xBA, 0xC0];
        deliver(&h, &i_am(9), &stranger, None).await;
        h.settle().await;
        let sent = checked(&h);
        if !sent.is_empty() {
            assert_eq!(sent, [PEER.to_vec(), stranger.to_vec()]);
            asked += sent.len();
            deliver(&h, &i_am(9), &PEER, None).await;
        }
        tokio::time::sleep(Duration::from_secs(5) - Duration::from_millis(1)).await;
    }
    assert_eq!(asked, 120, "two Who-Is per minute");
    assert!(who_is_sent(&h).is_empty());
    write_answered(&h).await;
    assert_eq!(write_stations(&h), [PEER.to_vec(), PEER.to_vec()]);
}
