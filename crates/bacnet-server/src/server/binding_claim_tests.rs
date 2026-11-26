//! An I-Am the server did not ask for neither binds a device nor moves one
//! (#2825): a write in another device goes where the server's own Who-Is
//! found it, and another address claiming the device starts a check of the
//! bound address, a confirmed read of its Device identifier.
//!
//! The harness is `remote_write_discovery_tests`': CMD-1's list 1 writes AO-1
//! in Device 9, which the server holds no binding for. [`STRANGER`] is
//! another station on the link that claims Device 9 in I-Ams of its own. The
//! APDU timeout is the default 3 seconds, with three retries, so a read of a
//! silent station ends 12 seconds after it first goes out.
use super::*;
use crate::server::device_bindings::{CHECK_HOLD_OFF, DeviceResolution};

/// Another station on the link, which claims Device 9 unasked.
const STRANGER: [u8; 6] = [10, 0, 0, 66, 0xBA, 0xC0];

/// A read of a silent station: its first attempt and three retries.
const SILENT_READ: Duration = Duration::from_secs(12);

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

/// The station each Who-Is sent straight to one went to, all for Device 9.
fn checked(h: &Harness) -> Vec<Vec<u8>> {
    directed_who_is_sent(h)
        .into_iter()
        .map(|(mac, (destination, who_is))| {
            assert_eq!((destination, who_is), (None, targeted(9)));
            mac
        })
        .collect()
}

/// The station Device 9 is bound at now.
async fn bound(h: &Harness) -> Vec<u8> {
    let table = h.server.device_bindings.read().await;
    match table.resolve_at(&device(9), runtime_clock::now(), |_| false) {
        DeviceResolution::ResolvedLocal { peer_mac, .. } => peer_mac.to_vec(),
        other => panic!("Device 9 bound on this network, not {other:?}"),
    }
}

/// The Device the wildcard instance names.
fn any_device() -> ObjectIdentifier {
    device(ObjectIdentifier::WILDCARD_INSTANCE)
}

/// Wait for the check's next read, which goes to the harness peer, and
/// return its invoke ID and the Device it asks for. Retries of it are
/// taken too.
async fn next_read(h: &Harness) -> (u8, ObjectIdentifier) {
    let reads = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let reads = identity_reads(h);
            if !reads.is_empty() {
                return reads;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("a read of the bound address");
    let [(mac, invoke_id, object)] = &reads[..] else {
        panic!("one read, not {reads:?}");
    };
    assert_eq!(mac.as_slice(), PEER);
    (*invoke_id, *object)
}

/// Cue a check of Device 9 from `claimant`, which answers the Who-Is the
/// check sends it: the invoke ID of the read of the bound address.
async fn cue_answered(h: &Harness, claimant: &[u8]) -> u8 {
    deliver(h, &i_am(9), claimant, None).await;
    h.settle().await;
    assert_eq!(checked(h), [claimant.to_vec()], "the claimant asked");
    deliver(h, &i_am(9), claimant, None).await;
    let (invoke_id, object) = next_read(h).await;
    assert_eq!(object, any_device(), "by the wildcard instance");
    invoke_id
}

/// The second check, due a hold-off after the first struck: wait for its
/// Who-Is, broadcast on this network (decision, maintainer, 2026-11-26),
/// which `claimant` `answers` or not, and return its read's invoke ID.
async fn second_check(h: &Harness, claimant: &[u8], answers: bool) -> u8 {
    tokio::time::sleep(CHECK_HOLD_OFF - Duration::from_millis(10)).await;
    assert!(who_is_sent(h).is_empty(), "not before the hold-off");
    // The first check's retries.
    identity_reads(h);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(who_is_sent(h), [(None, targeted(9))], "a local broadcast");
    assert!(checked(h).is_empty(), "nothing straight to the claimant");
    if answers {
        deliver(h, &i_am(9), claimant, None).await;
    }
    let (invoke_id, object) = next_read(h).await;
    assert_eq!(object, any_device());
    invoke_id
}

/// No further check: nothing is read or asked for two hold-offs.
async fn no_further_check(h: &Harness) {
    tokio::time::sleep(CHECK_HOLD_OFF * 2).await;
    assert!(identity_reads(h).is_empty(), "no read");
    assert!(checked(h).is_empty(), "no Who-Is");
}

/// The bound device answers the check's read naming itself: the binding
/// stays, and no second check follows.
#[tokio::test(start_paused = true)]
async fn the_bound_device_naming_itself_keeps_its_binding() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    let invoke_id = cue_answered(&h, &STRANGER).await;
    h.respond(names(invoke_id, 9)).await;
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
    write_answered(&h).await;
    assert_eq!(write_stations(&h), [PEER.to_vec(), PEER.to_vec()]);
}

/// One lost answer moves nothing: the read is sent again under its invoke
/// ID after the APDU timeout, and the device's answer to the retry ends the
/// check.
#[tokio::test(start_paused = true)]
async fn one_lost_answer_moves_nothing() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    let invoke_id = cue_answered(&h, &STRANGER).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(next_read(&h).await, (invoke_id, any_device()), "a retry");
    h.respond(names(invoke_id, 9)).await;
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
}

/// A read that times out through its retries, the claimant answering, is
/// one strike: a second check comes a hold-off later, and the device naming
/// itself then keeps its binding, with no third.
#[tokio::test(start_paused = true)]
async fn a_full_timeout_once_moves_nothing() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    cue_answered(&h, &STRANGER).await;
    tokio::time::sleep(SILENT_READ).await;
    assert_eq!(identity_reads(&h).len(), 3, "the retries");
    let invoke_id = second_check(&h, &STRANGER, true).await;
    h.respond(names(invoke_id, 9)).await;
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
}

/// A device that really moved: its old address is silent through two
/// checks a minute apart while it answers at the new one, so the binding
/// moves there 84 seconds after its I-Am cued the first. Until then every
/// write goes to the old address, none to the new one.
#[tokio::test(start_paused = true)]
async fn a_device_that_moved_is_followed_after_two_checks() {
    const MOVED: [u8; 6] = [10, 0, 0, 19, 0xBA, 0xC0];
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    let cued = TokioInstant::now();
    cue_answered(&h, &MOVED).await;
    // A write meanwhile still goes to the bound address, which is silent.
    let write = start_write(&h, 9);
    assert_eq!(write.await.unwrap().0, Err(RemoteRequestError::Unanswered));
    sent_writes(&h);
    second_check(&h, &MOVED, true).await;
    while bound(&h).await == PEER {
        assert!(!write_stations(&h).contains(&MOVED.to_vec()));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let moved = cued.elapsed();
    assert!(
        moved >= Duration::from_secs(84) && moved < Duration::from_secs(85),
        "moved after {moved:?}"
    );
    assert_eq!(bound(&h).await, MOVED);
    let write = start_write(&h, 9);
    let invoke_id = remote_write(&h).await;
    deliver(&h, &ack(invoke_id), &MOVED, None).await;
    assert_eq!(write.await.unwrap().0, Ok(()));
    assert_eq!(write_stations(&h).last(), Some(&MOVED.to_vec()));
}

/// After one strike, the old address answers a write of the server's: the
/// second check, though its read times out too, moves nothing.
#[tokio::test(start_paused = true)]
async fn the_old_address_answering_between_the_checks_resets_them() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    cue_answered(&h, &STRANGER).await;
    tokio::time::sleep(SILENT_READ).await;
    identity_reads(&h);
    write_answered(&h).await;
    second_check(&h, &STRANGER, true).await;
    tokio::time::sleep(SILENT_READ).await;
    identity_reads(&h);
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
}

/// The device is silent at its bound address but alive at a third one,
/// which answers the second check's broadcast beside the claimant: a
/// contest, counted, and nothing moves.
#[tokio::test(start_paused = true)]
async fn a_device_alive_elsewhere_contests_the_second_check() {
    const ALIVE: [u8; 6] = [10, 0, 0, 77, 0xBA, 0xC0];
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    cue_answered(&h, &STRANGER).await;
    tokio::time::sleep(SILENT_READ).await;
    second_check(&h, &STRANGER, true).await;
    deliver(&h, &i_am(9), &ALIVE, None).await;
    tokio::time::sleep(SILENT_READ).await;
    identity_reads(&h);
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
    assert_eq!(h.server.device_binding_conflicts().await, 1);
}

/// After one strike, the bound station's own I-Am, which it sent unasked,
/// is word from it all the same (decision, maintainer, 2026-11-26): the
/// second check, though its read times out, moves nothing.
#[tokio::test(start_paused = true)]
async fn the_bound_stations_i_am_between_the_checks_resets_them() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    cue_answered(&h, &STRANGER).await;
    tokio::time::sleep(SILENT_READ).await;
    deliver(&h, &i_am(9), &PEER, None).await;
    second_check(&h, &STRANGER, true).await;
    tokio::time::sleep(SILENT_READ).await;
    identity_reads(&h);
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
    assert_eq!(h.server.device_binding_conflicts().await, 0);
}

/// After a strike with one claimant, another claims the device during the
/// second check, the first silent: nothing moves.
#[tokio::test(start_paused = true)]
async fn different_claimants_reset_the_checks() {
    const OTHER_CLAIMANT: [u8; 6] = [10, 0, 0, 88, 0xBA, 0xC0];
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    cue_answered(&h, &STRANGER).await;
    tokio::time::sleep(SILENT_READ).await;
    identity_reads(&h);
    second_check(&h, &STRANGER, false).await;
    deliver(&h, &i_am(9), &OTHER_CLAIMANT, None).await;
    tokio::time::sleep(SILENT_READ).await;
    identity_reads(&h);
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
    assert_eq!(h.server.device_binding_conflicts().await, 1, "a contest");
}

/// A third station claims the device during the first check: its silent
/// read strikes nothing, and no second check follows.
#[tokio::test(start_paused = true)]
async fn a_third_claimant_in_a_check_moves_nothing() {
    const THIRD: [u8; 6] = [10, 0, 0, 77, 0xBA, 0xC0];
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    cue_answered(&h, &STRANGER).await;
    deliver(&h, &i_am(9), &THIRD, None).await;
    tokio::time::sleep(SILENT_READ).await;
    identity_reads(&h);
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
}

/// The bound station rejects the read by the wildcard instance, then the
/// one by its own: an unclear answer, no strike.
#[tokio::test(start_paused = true)]
async fn a_reject_from_the_bound_station_moves_nothing() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    let invoke_id = cue_answered(&h, &STRANGER).await;
    h.respond(rejects(invoke_id)).await;
    let (invoke_id, object) = next_read(&h).await;
    assert_eq!(object, device(9), "by the bound instance");
    h.respond(rejects(invoke_id)).await;
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
}

/// A device older than the wildcard instance refuses that read, and is
/// asked for the bound instance's Device object: naming it keeps the
/// binding. A station there with no such Device object fails the check.
#[tokio::test(start_paused = true)]
async fn a_device_that_refuses_the_wildcard_is_asked_by_its_instance() {
    for (has_it, struck) in [(true, false), (false, true)] {
        let h = start_unbound().await;
        bound_to_peer(&h).await;
        let invoke_id = cue_answered(&h, &STRANGER).await;
        h.respond(refuses(invoke_id, ErrorCode::UNKNOWN_OBJECT))
            .await;
        let (invoke_id, object) = next_read(&h).await;
        assert_eq!(object, device(9));
        h.respond(if has_it {
            names(invoke_id, 9)
        } else {
            refuses(invoke_id, ErrorCode::UNKNOWN_OBJECT)
        })
        .await;
        if struck {
            // It strikes once the claimant's answer has counted its wait.
            tokio::time::sleep(Duration::from_secs(3)).await;
            second_check(&h, &STRANGER, true).await;
        } else {
            no_further_check(&h).await;
        }
        assert_eq!(bound(&h).await, PEER);
    }
}

/// The link fails to send the read's retries: that asks the bound address
/// nothing, so its silence strikes nothing, and no second check follows.
#[tokio::test(start_paused = true)]
async fn a_read_the_link_failed_to_send_strikes_nothing() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    cue_answered(&h, &STRANGER).await;
    h.fail_notifications.store(true, Ordering::Release);
    tokio::time::sleep(SILENT_READ).await;
    h.fail_notifications.store(false, Ordering::Release);
    identity_reads(&h);
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
}

/// Nothing is checked while DeviceCommunicationControl restricts
/// initiation: a claim then sends nothing, and a strike's second check,
/// due under it, sends nothing and ends the checks.
#[tokio::test(start_paused = true)]
async fn nothing_is_checked_under_dcc() {
    let h = start_unbound().await;
    bound_to_peer(&h).await;
    disable_initiation(&h);
    deliver(&h, &i_am(9), &STRANGER, None).await;
    tokio::time::sleep(SILENT_READ).await;
    assert!(checked(&h).is_empty());
    assert!(identity_reads(&h).is_empty());

    let h = start_unbound().await;
    bound_to_peer(&h).await;
    cue_answered(&h, &STRANGER).await;
    tokio::time::sleep(SILENT_READ).await;
    identity_reads(&h);
    disable_initiation(&h);
    no_further_check(&h).await;
    h.server.comm_state.set_for_test(DccState::Enable);
    no_further_check(&h).await;
    assert_eq!(bound(&h).await, PEER);
}

/// The server stops while a check's read is out: nothing more is sent and
/// the binding stays.
#[tokio::test(start_paused = true)]
async fn a_stop_mid_check_ends_it() {
    let mut h = start_unbound().await;
    bound_to_peer(&h).await;
    // The check runs as one of the server's workers, which the stop ends.
    let workers = || h.server.notification_transactions.workers_empty();
    assert!(workers(), "none before the cue");
    cue_answered(&h, &STRANGER).await;
    assert!(!workers(), "the check's");
    // The network goes with the stop; its send log stays.
    let log = h.server.test_network().transport().sent();
    let sent = log.len();
    h.server.stop().await.unwrap();
    tokio::time::sleep(SILENT_READ + CHECK_HOLD_OFF * 2).await;
    assert_eq!(log.len(), sent, "nothing sent once stopped");
    assert_eq!(bound(&h).await, PEER);
}

// Floods of forged I-Ams (#2825).
#[path = "binding_flood_tests.rs"]
mod floods;
