//! The release of the BBMD a B/IPv6 foreign device left (#2338): with the
//! option on, a move sends the old BBMD one Delete-Foreign-Device-Table-Entry
//! naming this device's own address, after the registration with the new
//! BBMD, and counts how it ended; the move goes ahead whatever the answer.

use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::{Instant, timeout};

use super::foreign_retarget_tests::{address_of, bbmd, changes, next, registration, started};
use super::frame::encoded;
use super::*;
use crate::port::TransportPort;

/// The entry the next Delete-Foreign-Device-Table-Entry `peer` receives
/// names.
async fn release(peer: &UdpSocket) -> Bip6Address {
    next(peer, |message| match message {
        Bvll6::DeleteForeignDeviceTableEntry { entry, .. } => Some(entry),
        _ => None,
    })
    .await
}

/// Answer `transport` from `peer` with a BVLC-Result of `code`.
async fn answer(peer: &UdpSocket, transport: &Bip6Transport, code: Bvlc6ResultCode) {
    let result = Bvll6::BvlcResult {
        source: Bip6Vmac::new([0x0B, 0xB0, 0x01]),
        code,
    };
    let to = transport.local_address().unwrap();
    peer.send_to(&encoded(&result).unwrap(), to).await.unwrap();
}

fn counters(releases: u64, released: u64, refused: u64, unanswered: u64) -> Bip6ReleaseCounters {
    Bip6ReleaseCounters {
        releases,
        released,
        refused,
        unanswered,
    }
}

/// Wait, a millisecond at a time, until `transport` counts `want`.
async fn counted(transport: &Bip6Transport, want: Bip6ReleaseCounters) {
    for _ in 0..10_000 {
        if transport.previous_bbmd_releases() == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(transport.previous_bbmd_releases(), want);
}

/// The old BBMD's success counts as released and its NAK as refused; any
/// other result, or one from another peer, isn't the release's answer.
#[tokio::test]
async fn a_move_releases_the_bbmd_left_and_counts_its_answer() {
    let (a, b, c) = (bbmd().await, bbmd().await, bbmd().await);
    let (mut transport, lent) = started(&a, 300).await;
    transport.set_release_previous_bbmd(true);
    let own = transport.local_address().unwrap();
    let own = Bip6Address::new(*own.ip(), own.port());

    lent.activate(&changes(Some(address_of(&b)), None)).unwrap();
    assert_eq!(registration(&b).await, 300);
    assert_eq!(release(&a).await, own);
    // B's results are no answer to A's release.
    answer(
        &b,
        &transport,
        Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
    )
    .await;
    answer(&a, &transport, Bvlc6ResultCode::SUCCESSFUL_COMPLETION).await;
    counted(&transport, counters(1, 1, 0, 0)).await;

    lent.activate(&changes(Some(address_of(&c)), None)).unwrap();
    assert_eq!(registration(&c).await, 300);
    assert_eq!(release(&b).await, own);
    // Only the Delete-Foreign-Device-Table-Entry NAK refuses it (U.4.5.4).
    answer(&b, &transport, Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK).await;
    answer(
        &b,
        &transport,
        Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK,
    )
    .await;
    counted(&transport, counters(2, 1, 1, 0)).await;
    // The move went ahead: broadcasts go to C.
    assert_eq!(lent.bbmd(), address_of(&c));
    transport.stop().await.unwrap();
}

/// `wait`, with a paused clock held still while it runs.
///
/// A paused clock jumps to the next timer as soon as the runtime is idle,
/// whether or not a datagram is still on its way, so a socket read under
/// it can time out with the datagram a moment away. Tokio doesn't advance a
/// paused clock while a blocking task runs, so one parked for as long as
/// `wait` takes lets the datagram arrive in real time. After 5 real
/// seconds it lets go, and the read's own deadline then fails the test.
async fn in_real_time<T>(wait: impl Future<Output = T>) -> T {
    let (done, parked) = std::sync::mpsc::channel::<()>();
    let hold = tokio::task::spawn_blocking(move || {
        let _ = parked.recv_timeout(Duration::from_secs(5));
    });
    let out = wait.await;
    drop(done);
    hold.await.unwrap();
    out
}

/// An unanswered release is given up 3 s after it went.
///
/// Time is paused, so the two reads of a real socket, the start's
/// registration and the release, hold the clock still
/// ([`in_real_time`]). Between them nothing waits on a socket: the 3 s are
/// read from the counters, a millisecond at a time.
#[tokio::test(start_paused = true)]
async fn an_unanswered_release_is_given_up_after_3_s() {
    let (a, b) = (bbmd().await, bbmd().await);
    let (mut transport, lent) = in_real_time(started(&a, 300)).await;
    transport.set_release_previous_bbmd(true);
    let moved = Instant::now();
    lent.activate(&changes(Some(address_of(&b)), None)).unwrap();
    counted(&transport, counters(1, 0, 0, 1)).await;
    let waited = moved.elapsed();
    assert!(
        (Duration::from_secs(3)..Duration::from_millis(3_002)).contains(&waited),
        "{waited:?}"
    );
    let entry = in_real_time(release(&a)).await;
    assert_eq!(entry.port, transport.local_address().unwrap().port());
    transport.stop().await.unwrap();
}

/// Off by default: a move sends the old BBMD nothing.
#[tokio::test]
async fn without_the_option_a_move_releases_nothing() {
    let (a, b) = (bbmd().await, bbmd().await);
    let (mut transport, lent) = started(&a, 300).await;
    lent.activate(&changes(Some(address_of(&b)), None)).unwrap();
    assert_eq!(registration(&b).await, 300);
    // A release would have gone in the same pass, right after.
    let nothing = timeout(Duration::from_millis(300), release(&a)).await;
    assert!(nothing.is_err(), "{nothing:?}");
    assert_eq!(transport.previous_bbmd_releases(), counters(0, 0, 0, 0));
    transport.stop().await.unwrap();
}

/// Moves made while a re-registration pass is held after reading the
/// target: each queues the release of the BBMD it leaves and drops the one
/// queued before, unsent, as B/IP's node engine does, so the BBMD the
/// device returns to is never asked. Each release ends once, the one still
/// out when the transport stops included.
#[tokio::test]
async fn a_release_still_queued_is_dropped_by_the_next_move() {
    let (a, b, c) = (bbmd().await, bbmd().await, bbmd().await);
    let (mut transport, lent) = started(&a, 300).await;
    transport.set_release_previous_bbmd(true);
    let target = Arc::clone(transport.foreign_target().unwrap());
    target
        .gate
        .forget_permits(tokio::sync::Semaphore::MAX_PERMITS);
    // The pass for the move to B takes A's release, and waits at the gate.
    lent.activate(&changes(Some(address_of(&b)), None)).unwrap();
    target.reached.notified().await;
    // B's release is queued, then dropped by the move back to A, which
    // queues C's.
    lent.activate(&changes(Some(address_of(&c)), None)).unwrap();
    lent.activate(&changes(Some(address_of(&a)), None)).unwrap();
    assert_eq!(transport.previous_bbmd_releases(), counters(3, 0, 0, 1));
    target.gate.add_permits(tokio::sync::Semaphore::MAX_PERMITS);

    // The held pass registers with B and releases A; the next registers
    // with A again and releases C, which ends the wait for A's answer.
    assert_eq!(registration(&b).await, 300);
    release(&a).await;
    assert_eq!(registration(&a).await, 300);
    release(&c).await;
    assert_eq!(transport.previous_bbmd_releases(), counters(3, 0, 0, 2));
    let nothing = timeout(Duration::from_millis(300), release(&b)).await;
    assert!(nothing.is_err(), "B's release was dropped: {nothing:?}");
    // C's release is still out: the stop ends it.
    transport.stop().await.unwrap();
    assert_eq!(transport.previous_bbmd_releases(), counters(3, 0, 0, 3));
}
