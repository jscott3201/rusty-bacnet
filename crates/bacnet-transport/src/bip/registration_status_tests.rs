//! What a foreign device reports about its registration (#1622): the lease
//! in its snapshot, and one log line each when it becomes registered, starts
//! being refused, or lapses.

use std::time::Duration;

use tracing::Level;

use super::registration_schedule_tests::Device;
use crate::log_capture::LogCapture;

const REGISTERED: &str = "B/IP foreign device registered";
const REFUSED: &str = "B/IP BBMD refused";
const LAPSED: &str = "B/IP foreign-device registration lapsed";
const RENEWED: &str = "Foreign registration renewed";

/// Registered at once, refused at 10 s, then every attempt lost until the
/// lease, proven from 0 s for the 60 s time-to-live, lapses; a success at
/// 65 s registers it again.
#[tokio::test(start_paused = true)]
async fn each_transition_is_logged_once_and_the_lease_follows_it() {
    let (logs, _guard) = LogCapture::start();
    let device = Device::new(Duration::from_secs(10)).await;
    device.at(13).await;
    device.bbmd_says(0).await;
    device.at(10_013).await;
    device.bbmd_says(0x30).await;
    device.at(59_999).await;
    let live = device.ctx.node.snapshot().foreign_registration;
    assert!(live.registered);
    assert_eq!(live.lease_remaining, Some(Duration::from_millis(1)));
    device.at(60_500).await;
    let lapsed = device.ctx.node.snapshot().foreign_registration;
    assert!(!lapsed.registered);
    assert_eq!(lapsed.lease_remaining, None);
    assert_eq!(lapsed.lapses, 1);
    // A broadcast with no live lease goes, and is counted.
    device
        .ctx
        .node
        .send_broadcast(&[0x01, 0x00], None)
        .await
        .unwrap();
    assert_eq!(
        device
            .ctx
            .node
            .snapshot()
            .foreign_registration
            .broadcasts_unregistered,
        1
    );
    // The attempt at 65 s is answered.
    device.at(65_013).await;
    device.bbmd_says(0).await;
    assert!(device.ctx.node.snapshot().foreign_registration.registered);

    assert_eq!(logs.levels(REGISTERED), [Level::INFO, Level::INFO]);
    assert_eq!(logs.levels(REFUSED), [Level::WARN]);
    assert_eq!(logs.levels(LAPSED), [Level::WARN]);
    // Nothing else warns: the losses and renewals are routine.
    assert_eq!(logs.warnings().len(), 2, "{:?}", logs.warnings());
}

/// A BBMD that keeps refusing draws one warning: the refusals after it in a
/// row log at debug. A success ends the run, so a refusal after it warns
/// again. A renewal logs at debug.
#[tokio::test(start_paused = true)]
async fn a_run_of_refusals_warns_once() {
    let (logs, _guard) = LogCapture::start();
    let device = Device::new(Duration::from_secs(10)).await;
    device.at(13).await;
    device.bbmd_says(0).await;
    for at in [10_013, 20_013, 30_013] {
        device.at(at).await;
        device.bbmd_says(0x30).await;
    }
    // Renewed while the first lease is live.
    device.at(40_013).await;
    device.bbmd_says(0).await;
    device.at(50_013).await;
    device.bbmd_says(0x30).await;

    let (warn, debug) = (Level::WARN, Level::DEBUG);
    assert_eq!(logs.levels(REFUSED), [warn, debug, debug, warn]);
    assert_eq!(logs.levels(RENEWED), [debug]);
    assert_eq!(logs.levels(REGISTERED), [Level::INFO]);
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!((registration.accepted, registration.refused), (2, 4));
}

/// A registration and a lapse each end a run of refusals too: a refusal
/// after either warns again.
#[tokio::test(start_paused = true)]
async fn a_registration_or_a_lapse_ends_a_run_of_refusals() {
    let (logs, _guard) = LogCapture::start();
    let device = Device::new(Duration::from_secs(10)).await;
    // Refused at first, registered at 10 s from that attempt, refused again.
    device.at(13).await;
    device.bbmd_says(0x30).await;
    device.at(10_013).await;
    device.bbmd_says(0).await;
    device.at(20_013).await;
    device.bbmd_says(0x30).await;
    // Every attempt lost from 30 s: the lease, from 10 s, lapses at 70 s,
    // and the attempt at 75 s is refused.
    device.at(75_013).await;
    device.bbmd_says(0x30).await;

    let warn = Level::WARN;
    assert_eq!(logs.levels(REFUSED), [warn, warn, warn]);
    assert_eq!(logs.levels(REGISTERED), [Level::INFO]);
    assert_eq!(logs.levels(LAPSED), [warn]);
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!((registration.refused, registration.lapses), (3, 1));
}

/// The link's returns (#2572). A start's first registration is none: the
/// broadcasts made before it were held for it. A registration after a lease
/// lapsed, or after the BBMD refused an attempt, is one: broadcasts may have
/// gone nowhere meanwhile. A renewal while the lease is live is none.
#[tokio::test(start_paused = true)]
async fn a_registration_after_a_loss_is_a_return_of_the_link() {
    let device = Device::new(Duration::from_secs(10)).await;
    let returns = device.ctx.node.link_returns();
    device.at(13).await;
    device.bbmd_says(0).await;
    device.at(10_013).await;
    device.bbmd_says(0).await;
    assert_eq!(*returns.borrow(), 0, "a start's registration and a renewal");
    // Every attempt lost from 20 s: the lease, renewed from 10 s, lapses
    // at 70 s, and the attempt at 75 s is answered.
    device.at(75_013).await;
    device.bbmd_says(0).await;
    assert_eq!(*returns.borrow(), 1, "registered again after the lapse");

    let device = Device::new(Duration::from_secs(10)).await;
    let returns = device.ctx.node.link_returns();
    device.at(13).await;
    device.bbmd_says(0x30).await;
    device.at(10_013).await;
    device.bbmd_says(0).await;
    assert_eq!(*returns.borrow(), 1, "registered after a refusal");
}

/// A start whose first attempt goes unanswered sends the broadcasts it held
/// unregistered: its next success is a return of the link (#2572).
#[tokio::test(start_paused = true)]
async fn a_success_after_an_unanswered_first_attempt_is_a_return_of_the_link() {
    let device = Device::new(Duration::from_secs(10)).await;
    let returns = device.ctx.node.link_returns();
    // The attempt at start goes unanswered; the retry goes at 6 s.
    device.at(6_013).await;
    assert_eq!(device.attempts(), [0, 6_000]);
    device.bbmd_says(0).await;
    assert!(device.ctx.node.snapshot().foreign_registration.registered);
    assert_eq!(*returns.borrow(), 1);
}

/// The BBMD refusing a broadcast with the Distribute-Broadcast-To-Network
/// NAK holds no entry for this device, though the lease says live: the
/// next success, a renewal, is a return of the link (#2572). The same NAK
/// from another peer changes nothing.
#[tokio::test(start_paused = true)]
async fn a_renewal_after_the_bbmd_refused_a_broadcast_is_a_return_of_the_link() {
    let device = Device::new(Duration::from_secs(10)).await;
    let returns = device.ctx.node.link_returns();
    device.at(13).await;
    device.bbmd_says(0).await;
    device.at(10_013).await;
    device.bbmd_says(0).await;
    device.at(12_000).await;
    device.says(([192, 0, 2, 30], 0xBAC0), 0x60).await;
    device.at(12_500).await;
    assert_eq!(*returns.borrow(), 0, "another peer's NAK");
    device.bbmd_says(0x60).await;
    // The NAK brings the next attempt forward, to 13 s; its success is a
    // renewal of the lease from 10 s.
    device.at(13_013).await;
    device.bbmd_says(0).await;
    assert_eq!(*returns.borrow(), 1);
    assert_eq!(
        device
            .ctx
            .node
            .snapshot()
            .foreign_registration
            .renewal_hints,
        1
    );
}
