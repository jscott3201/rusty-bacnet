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
