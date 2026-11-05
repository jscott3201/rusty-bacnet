//! A B/IPv6 foreign device records its BBMD's answers to its registrations
//! (#2388), through the receive loop: held or refused, unanswered when the
//! next registration goes first, and nothing for the release of a BBMD it
//! left (#2338).

use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;

use super::foreign_retarget_tests::{
    address_of, bbmd, changes, next, next_within, registration, started,
};
use super::frame::encoded;
use super::*;
use crate::port::TransportPort;

/// Send `transport` a BVLC-Result of `code` from `peer`.
async fn result(peer: &UdpSocket, transport: &Bip6Transport, code: Bvlc6ResultCode) {
    let result = Bvll6::BvlcResult {
        source: Bip6Vmac::new([0x0B, 0xB0, 0x01]),
        code,
    };
    let to = transport.local_address().unwrap();
    peer.send_to(&encoded(&result).unwrap(), to).await.unwrap();
}

fn status(transport: &Bip6Transport) -> Bip6ForeignStatus {
    transport.foreign_status().expect("a foreign device")
}

/// Wait, a millisecond at a time, until `transport` reports `want` as its
/// BBMD's latest answer.
async fn answered(transport: &Bip6Transport, want: Option<Bip6RegistrationAnswer>) {
    for _ in 0..10_000 {
        if status(transport).last_answer == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(status(transport).last_answer, want);
}

/// The BBMD's success marks the registration held, and its NAK to the next
/// one, sent at once for a new time-to-live, marks it refused.
#[tokio::test]
async fn the_bbmds_answers_mark_the_registration_held_or_refused() {
    let a = bbmd().await;
    let (mut transport, lent) = started(&a, 300).await;
    assert_eq!(status(&transport).last_answer, None);
    result(&a, &transport, Bvlc6ResultCode::SUCCESSFUL_COMPLETION).await;
    answered(&transport, Some(Bip6RegistrationAnswer::Held)).await;

    lent.activate(&changes(None, Some(120))).unwrap();
    assert_eq!(registration(&a).await, 120);
    // Still held until the BBMD answers the new one.
    assert_eq!(
        status(&transport).last_answer,
        Some(Bip6RegistrationAnswer::Held)
    );
    result(&a, &transport, Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK).await;
    answered(&transport, Some(Bip6RegistrationAnswer::Refused)).await;
    let refused = status(&transport);
    assert_eq!(
        (refused.registrations_refused, refused.registrations_sent),
        (1, 2)
    );
    // A stop forgets the answer, not the total.
    transport.stop().await.unwrap();
    let stopped = status(&transport);
    assert_eq!(
        (stopped.last_answer, stopped.registrations_refused),
        (None, 1)
    );
}

/// The BBMD's answer can arrive before the send that carried the
/// registration has returned. The registration awaits it from its pass on,
/// so the answer counts: the test holds the pass at the gate between the
/// two and answers meanwhile. The start's registration is refused first, so
/// the success can only be the new one's.
#[tokio::test]
async fn an_answer_that_beats_the_send_still_counts() {
    let a = bbmd().await;
    let (mut transport, lent) = started(&a, 300).await;
    result(&a, &transport, Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK).await;
    answered(&transport, Some(Bip6RegistrationAnswer::Refused)).await;
    let target = Arc::clone(transport.foreign_target().unwrap());
    target
        .gate
        .forget_permits(tokio::sync::Semaphore::MAX_PERMITS);
    lent.activate(&changes(None, Some(120))).unwrap();
    target.reached.notified().await;
    result(&a, &transport, Bvlc6ResultCode::SUCCESSFUL_COMPLETION).await;
    answered(&transport, Some(Bip6RegistrationAnswer::Held)).await;
    target.gate.add_permits(tokio::sync::Semaphore::MAX_PERMITS);
    assert_eq!(registration(&a).await, 120);
    for _ in 0..10_000 {
        if status(&transport).registrations_sent == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let held = status(&transport);
    assert_eq!(
        (held.last_answer, held.registrations_sent),
        (Some(Bip6RegistrationAnswer::Held), 2)
    );
    transport.stop().await.unwrap();
}

/// An activation's immediate registration supersedes the one still
/// awaiting its answer, which wasn't left unanswered: no NO ANSWER flickers
/// on before the BBMD answers the new one.
#[tokio::test]
async fn an_activation_supersedes_a_registration_rather_than_leave_it_unanswered() {
    let a = bbmd().await;
    let (mut transport, lent) = started(&a, 300).await;
    lent.activate(&changes(None, Some(120))).unwrap();
    assert_eq!(registration(&a).await, 120);
    let superseded = status(&transport);
    assert_eq!((superseded.last_answer, superseded.unanswered), (None, 0));
    result(&a, &transport, Bvlc6ResultCode::SUCCESSFUL_COMPLETION).await;
    answered(&transport, Some(Bip6RegistrationAnswer::Held)).await;
    transport.stop().await.unwrap();
}

/// After a move, the old BBMD's success answers its release, not the
/// registration with the new BBMD, and the old answer is gone.
#[tokio::test]
async fn a_success_for_the_release_of_the_bbmd_left_is_not_counted() {
    let (a, b) = (bbmd().await, bbmd().await);
    let (mut transport, lent) = started(&a, 300).await;
    transport.set_release_previous_bbmd(true);
    result(&a, &transport, Bvlc6ResultCode::SUCCESSFUL_COMPLETION).await;
    answered(&transport, Some(Bip6RegistrationAnswer::Held)).await;

    lent.activate(&changes(Some(address_of(&b)), None)).unwrap();
    assert_eq!(registration(&b).await, 300);
    next(&a, |message| {
        matches!(message, Bvll6::DeleteForeignDeviceTableEntry { .. }).then_some(())
    })
    .await;
    assert_eq!(status(&transport).last_answer, None, "A's answer went");
    result(&a, &transport, Bvlc6ResultCode::SUCCESSFUL_COMPLETION).await;
    for _ in 0..10_000 {
        if transport.previous_bbmd_releases().released == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(transport.previous_bbmd_releases().released, 1);
    assert_eq!(status(&transport).last_answer, None);
    result(&b, &transport, Bvlc6ResultCode::SUCCESSFUL_COMPLETION).await;
    answered(&transport, Some(Bip6RegistrationAnswer::Held)).await;
    transport.stop().await.unwrap();
}

/// A NAK leaves the renewal schedule as it was: the next registration still
/// goes half the time-to-live after the one before. One the BBMD leaves
/// unanswered until then is unanswered.
///
/// Time is paused, and real sockets let the clock jump while a datagram is
/// on its way, so the period is read at the gate, which a renewal reaches
/// with no I/O in between, and the answers from the status, a millisecond
/// at a time.
#[tokio::test(start_paused = true)]
async fn a_refusal_keeps_the_schedule_and_silence_is_unanswered() {
    let peer = bbmd().await;
    let (mut transport, _lent) = started(&peer, 60).await;
    let target = Arc::clone(transport.foreign_target().unwrap());
    let reached = || target.reached.notified();
    result(
        &peer,
        &transport,
        Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK,
    )
    .await;
    answered(&transport, Some(Bip6RegistrationAnswer::Refused)).await;

    reached().await;
    let renewed = tokio::time::Instant::now();
    // Still refused as the renewal goes out. Read at the gate: while the
    // test waits for the datagram below, paused time can jump to the next
    // renewal, which would count this one unanswered first.
    assert_eq!(
        status(&transport).last_answer,
        Some(Bip6RegistrationAnswer::Refused)
    );
    let long = Duration::from_secs(3600);
    let ttl = next_within(&peer, long, |message| match message {
        Bvll6::RegisterForeignDevice { ttl, .. } => Some(ttl),
        _ => None,
    })
    .await;
    assert_eq!(ttl, 60);
    reached().await;
    assert_eq!(renewed.elapsed(), Duration::from_secs(30), "the usual 30 s");
    answered(&transport, Some(Bip6RegistrationAnswer::Unanswered)).await;
    let silent = status(&transport);
    assert_eq!((silent.unanswered, silent.registrations_refused), (1, 1));
    transport.stop().await.unwrap();
}

/// The reviewer's case: activations every second against a silent BBMD.
/// Each registers at once and supersedes the one before, but once the BBMD
/// has been silent for the 3 s answer timeout the status reads unanswered.
/// Time is paused; each pass is read at the gate, and its bookkeeping a
/// millisecond later.
#[tokio::test(start_paused = true)]
async fn activations_against_a_silent_bbmd_reach_no_answer() {
    let peer = bbmd().await;
    let (mut transport, lent) = started(&peer, 300).await;
    let target = Arc::clone(transport.foreign_target().unwrap());
    for second in 1..=3 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        lent.activate(&changes(None, Some(300))).unwrap();
        target.reached.notified().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        let want = (second == 3).then_some(Bip6RegistrationAnswer::Unanswered);
        assert_eq!(status(&transport).last_answer, want, "after {second} s");
    }
    assert_eq!(status(&transport).unanswered, 1);
    transport.stop().await.unwrap();
}

/// A renewal whose send fails counts nothing, sends nothing, and leaves
/// the registration before it awaiting its answer, which still counts.
#[tokio::test(start_paused = true)]
async fn a_failed_send_counts_nothing_and_keeps_the_one_before() {
    let peer = bbmd().await;
    let (mut transport, _lent) = started(&peer, 60).await;
    let target = Arc::clone(transport.foreign_target().unwrap());
    target
        .fail_send
        .store(true, std::sync::atomic::Ordering::Relaxed);
    target.reached.notified().await;
    tokio::time::sleep(Duration::from_millis(1)).await;
    let failed = status(&transport);
    assert_eq!(
        (
            failed.last_answer,
            failed.unanswered,
            failed.registrations_sent
        ),
        (None, 0, 1)
    );
    let nothing = timeout(Duration::from_millis(500), registration(&peer)).await;
    assert!(nothing.is_err(), "the failed renewal sent {nothing:?}");
    result(&peer, &transport, Bvlc6ResultCode::SUCCESSFUL_COMPLETION).await;
    answered(&transport, Some(Bip6RegistrationAnswer::Held)).await;
    transport.stop().await.unwrap();
}
