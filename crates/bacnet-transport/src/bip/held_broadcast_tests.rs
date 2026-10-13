//! A foreign device holds its broadcasts until the first registration
//! attempt after a start resolves (#1622): up to 16, sent in order once
//! that attempt succeeds or goes unanswered, dropped if the BBMD refuses
//! it or the node stops, and sent at once after that, in the order made.

use std::time::Duration;

use super::registration_schedule_tests::Device;

/// The Register-Foreign-Device every attempt sends (time-to-live 60 s).
const REGISTER: [u8; 6] = [0x81, 0x05, 0x00, 0x06, 0x00, 60];

/// A one-octet-payload NPDU, numbered.
fn npdu(n: u8) -> [u8; 3] {
    [0x01, 0x00, n]
}

/// Its Distribute-Broadcast-To-Network.
fn distribute(n: u8) -> Vec<u8> {
    vec![0x81, 0x09, 0x00, 0x07, 0x01, 0x00, n]
}

async fn broadcast(device: &Device, n: u8) -> Result<(), bacnet_types::error::Error> {
    device.ctx.node.send_broadcast(&npdu(n), None).await
}

/// Two broadcasts made while the first attempt waits go nowhere until its
/// success, then go in order; the next goes at once.
#[tokio::test(start_paused = true)]
async fn held_broadcasts_go_in_order_once_the_first_attempt_succeeds() {
    let device = Device::new(Duration::from_secs(10)).await;
    device.at(5).await;
    broadcast(&device, 1).await.unwrap();
    broadcast(&device, 2).await.unwrap();
    assert_eq!(
        device.sent(),
        [(0, REGISTER.to_vec())],
        "nothing but the attempt"
    );
    let held = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(held.broadcasts_held, 2);

    device.at(13).await;
    device.bbmd_says(0).await;
    device.at(20).await;
    assert_eq!(device.sent(), [(13, distribute(1)), (13, distribute(2))]);
    broadcast(&device, 3).await.unwrap();
    assert_eq!(device.sent(), [(20, distribute(3))]);

    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(
        (
            registration.broadcasts_held,
            registration.broadcasts_released,
            registration.broadcasts_discarded,
            registration.broadcasts_unregistered,
        ),
        (2, 2, 0, 0)
    );
}

/// When the first attempt goes unanswered, its timeout (3 s) releases what
/// it held: the device can't tell a lost answer from a BBMD that never
/// got the request.
#[tokio::test(start_paused = true)]
async fn held_broadcasts_go_when_the_first_attempt_times_out() {
    let device = Device::new(Duration::from_secs(10)).await;
    broadcast(&device, 1).await.unwrap();
    device.at(3_100).await;
    assert_eq!(
        device.sent(),
        [(0, REGISTER.to_vec()), (3_000, distribute(1))]
    );
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(registration.broadcasts_released, 1);
    // Sent with no lease: counted when it was made, not when held.
    assert_eq!(registration.broadcasts_unregistered, 0);
}

/// A refusal drops what the first attempt held; later broadcasts go at
/// once, without a lease.
#[tokio::test(start_paused = true)]
async fn held_broadcasts_are_dropped_when_the_bbmd_refuses() {
    let device = Device::new(Duration::from_secs(10)).await;
    broadcast(&device, 1).await.unwrap();
    broadcast(&device, 2).await.unwrap();
    device.at(13).await;
    device.bbmd_says(0x30).await;
    device.at(20).await;
    assert_eq!(device.sent(), [(0, REGISTER.to_vec())]);
    broadcast(&device, 3).await.unwrap();
    assert_eq!(device.sent(), [(20, distribute(3))]);
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(
        (
            registration.broadcasts_held,
            registration.broadcasts_discarded,
            registration.broadcasts_unregistered,
        ),
        (2, 2, 1)
    );
}

/// Sixteen are held; a seventeenth is refused with an error the caller
/// sees, and counted.
#[tokio::test(start_paused = true)]
async fn a_full_hold_refuses_the_next_broadcast() {
    let device = Device::new(Duration::from_secs(10)).await;
    for n in 0..16 {
        broadcast(&device, n).await.unwrap();
    }
    let full = broadcast(&device, 16).await.unwrap_err();
    assert!(full.to_string().contains("no room to hold"), "{full}");
    // Counted with the registration, not as a send over the budget.
    assert_eq!(device.ctx.node.send_counters().npdu_too_long, 0);
    device.at(13).await;
    device.bbmd_says(0).await;
    device.at(20).await;
    let released: Vec<_> = device.sent().into_iter().skip(1).collect();
    let expected: Vec<_> = (0..16).map(|n| (13, distribute(n))).collect();
    assert_eq!(released, expected);
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(
        (
            registration.broadcasts_held,
            registration.broadcasts_released,
            registration.broadcasts_hold_full,
        ),
        (16, 16, 1)
    );
}

/// A broadcast made after the release, while the released ones still wait
/// for the driver, queues behind them: broadcasts go in the order they were
/// made.
#[tokio::test(start_paused = true)]
async fn a_broadcast_made_while_released_ones_wait_goes_after_them() {
    let device = Device::new(Duration::from_secs(10)).await;
    broadcast(&device, 1).await.unwrap();
    device.at(13).await;
    // The success releases the first; the driver hasn't run yet when the
    // second is made.
    device.bbmd_says(0).await;
    broadcast(&device, 2).await.unwrap();
    device.at(20).await;
    assert_eq!(
        device.sent(),
        [
            (0, REGISTER.to_vec()),
            (13, distribute(1)),
            (13, distribute(2))
        ]
    );
    // Once they are out, a broadcast goes from its caller again.
    broadcast(&device, 3).await.unwrap();
    assert_eq!(device.sent(), [(20, distribute(3))]);
}

/// The same while the driver is sending the released ones: a broadcast
/// made between two of its sends waits for the rest.
#[tokio::test(start_paused = true)]
async fn a_broadcast_made_between_the_drivers_sends_goes_after_them() {
    let device = Device::new(Duration::from_secs(10)).await;
    device.sink.yield_on_send(true);
    broadcast(&device, 1).await.unwrap();
    broadcast(&device, 2).await.unwrap();
    device.at(13).await;
    device.bbmd_says(0).await;
    // The driver starts on the released ones and yields at its first send.
    tokio::task::yield_now().await;
    broadcast(&device, 3).await.unwrap();
    device.at(20).await;
    let sent: Vec<_> = device.sent().into_iter().skip(1).collect();
    assert_eq!(
        sent,
        [
            (13, distribute(1)),
            (13, distribute(2)),
            (13, distribute(3))
        ]
    );
}

/// When the first attempt times out with the next attempt already due, as
/// with a renewal interval no longer than the response wait, the driver
/// sends the new attempt before the broadcasts the timeout released.
#[tokio::test(start_paused = true)]
async fn the_next_attempt_goes_before_the_broadcasts_it_releases() {
    let device = Device::new(Duration::from_millis(300)).await;
    broadcast(&device, 1).await.unwrap();
    device.at(350).await;
    assert_eq!(
        device.sent(),
        [
            (0, REGISTER.to_vec()),
            (300, REGISTER.to_vec()),
            (300, distribute(1))
        ]
    );
}

/// A stop drops what the node still holds, counted as discarded, so every
/// held broadcast is counted as released or discarded.
#[tokio::test(start_paused = true)]
async fn held_broadcasts_count_as_discarded_when_the_node_stops() {
    let device = Device::new(Duration::from_secs(10)).await;
    broadcast(&device, 1).await.unwrap();
    broadcast(&device, 2).await.unwrap();
    device.ctx.node.stop();
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(
        (
            registration.broadcasts_held,
            registration.broadcasts_released,
            registration.broadcasts_discarded,
        ),
        (2, 0, 2)
    );
    // Nothing is counted once stopped: not a broadcast held by a call that
    // raced the stop, nor its drop at a second stop.
    broadcast(&device, 3).await.unwrap();
    device.ctx.node.stop();
    let again = device.ctx.node.snapshot().foreign_registration;
    assert_eq!((again.broadcasts_held, again.broadcasts_discarded), (2, 2));
}

/// Broadcasts made while released ones go queue behind them, up to the
/// hold's 16; one more is refused as a full hold, counted.
#[tokio::test(start_paused = true)]
async fn the_queue_behind_released_broadcasts_is_bounded() {
    let device = Device::new(Duration::from_secs(10)).await;
    device.sink.yield_on_send(true);
    broadcast(&device, 0).await.unwrap();
    device.at(13).await;
    device.bbmd_says(0).await;
    tokio::task::yield_now().await;
    for n in 1..=16 {
        broadcast(&device, n).await.unwrap();
    }
    let full = broadcast(&device, 17).await.unwrap_err();
    assert!(full.to_string().contains("no room to hold"), "{full}");
    device.at(20).await;
    let sent: Vec<_> = device.sent().into_iter().skip(1).collect();
    let expected: Vec<_> = (0..=16).map(|n| (13, distribute(n))).collect();
    assert_eq!(sent, expected);
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(registration.broadcasts_hold_full, 1);
    assert_eq!(device.ctx.node.send_counters().npdu_too_long, 0);
}

/// A stop drops what is queued behind released broadcasts, counted as
/// discarded.
#[tokio::test(start_paused = true)]
async fn queued_broadcasts_count_as_discarded_when_the_node_stops() {
    let device = Device::new(Duration::from_secs(10)).await;
    device.sink.yield_on_send(true);
    broadcast(&device, 0).await.unwrap();
    device.at(13).await;
    device.bbmd_says(0).await;
    tokio::task::yield_now().await;
    broadcast(&device, 1).await.unwrap();
    broadcast(&device, 2).await.unwrap();
    device.ctx.node.stop();
    device.at(20).await;
    let sent: Vec<_> = device.sent().into_iter().skip(1).collect();
    assert_eq!(sent, [(13, distribute(0))], "nothing queued went out");
    let registration = device.ctx.node.snapshot().foreign_registration;
    assert_eq!(
        (
            registration.broadcasts_held,
            registration.broadcasts_released,
            registration.broadcasts_discarded,
        ),
        (1, 1, 2)
    );
}
