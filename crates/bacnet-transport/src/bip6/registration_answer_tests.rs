//! A B/IPv6 foreign device records its BBMD's answers to its registrations
//! (#2388), through the receive loop: held or refused, unanswered when the
//! next registration goes first, and nothing for the release of a BBMD it
//! left (#2338).

use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;

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
    let long = Duration::from_secs(3600);
    let ttl = next_within(&peer, long, |message| match message {
        Bvll6::RegisterForeignDevice { ttl, .. } => Some(ttl),
        _ => None,
    })
    .await;
    assert_eq!(ttl, 60);
    // Still refused while the renewal waits for its answer.
    assert_eq!(
        status(&transport).last_answer,
        Some(Bip6RegistrationAnswer::Refused)
    );
    reached().await;
    assert_eq!(renewed.elapsed(), Duration::from_secs(30), "the usual 30 s");
    answered(&transport, Some(Bip6RegistrationAnswer::Unanswered)).await;
    let silent = status(&transport);
    assert_eq!((silent.unanswered, silent.registrations_refused), (1, 1));
    transport.stop().await.unwrap();
}
