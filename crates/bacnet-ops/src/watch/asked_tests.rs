//! The old address asked once more (#2812), against the lab device, which
//! a station at [`ELSEWHERE`] claims to be: a handle's move is asked about
//! even while the address looks failing, and each answer the ask can draw
//! for a device that refuses the wildcard read is taken as the module
//! `watch::device::moves` says.

use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_types::enums::ErrorCode;
use bacnet_types::primitives::PropertyValue;
use tokio::time::Instant;

use super::State;
use super::hub::WatchOptions;
use super::stream_tests::{lab_device, latest, number, ready, until};
use super::test_lab::{ELSEWHERE, LAB, Lab, READ_PROPERTY, RPM, ai, lab_at};
use super::*;
use crate::device::{ClientEvent, DeviceAddress};

fn elsewhere() -> DeviceAddress {
    DeviceAddress::local(&[ELSEWHERE]).unwrap()
}

/// Tell the hub that a handle following the lab device moved it to
/// [`ELSEWHERE`], where a device names itself so.
fn moved_away(lab: &Lab) {
    *lab.quirks.elsewhere.lock().unwrap() = Some(LAB);
    let (from, to) = (lab_at(), elsewhere());
    let moved = ClientEvent::Moved {
        instance: LAB,
        from,
        to,
    };
    lab.client.shared.events.send(moved).unwrap();
}

/// No update for `minutes` gives the point up.
async fn none_given_up(watch: &mut Watch, minutes: u64) {
    for _ in 0..minutes * 6 {
        tokio::time::sleep(Duration::from_secs(10)).await;
        while let Some(update) = ready(watch) {
            assert!(!matches!(update.state, State::Gone { .. }), "{update:?}");
        }
    }
}

/// A polled point whose device missed one poll, so the address is in its
/// backoff and the point unreachable, though the device is back: a
/// handle's move there is still asked about, the device answers as itself,
/// and the watch goes on (two claimants).
#[tokio::test(start_paused = true)]
async fn a_handles_move_is_asked_about_though_the_address_looks_failing() {
    let lab = Lab::start(WatchOptions::default()).await;
    lab.quirks.refuse_cov.store(true, Ordering::Relaxed);
    let device = lab_device(&lab).await;
    let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
    until(&mut watch, |u| matches!(u.state, State::Polled { .. })).await;
    latest(&mut watch).await;
    lab.quirks.muted.store(true, Ordering::Relaxed);
    until(&mut watch, |u| matches!(u.state, State::Unreachable { .. })).await;
    lab.quirks.muted.store(false, Ordering::Relaxed);
    // The name read the unanswered poll cued (#2603) goes first.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let from = Instant::now();
    moved_away(&lab);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(lab.sent_to(READ_PROPERTY, ELSEWHERE, from), 1, "read there");
    assert_eq!(lab.sent_to(READ_PROPERTY, 2, from), 1, "asked here");
    none_given_up(&mut watch, 2).await;
    lab.server
        .set_present_value_local(&ai(1), PropertyValue::Real(36.0))
        .await
        .unwrap();
    let update = until(&mut watch, |u| number(u) == Some(36.0)).await;
    assert!(matches!(update.state, State::Polled { .. }), "{update:?}");
    assert!(lab.sent(RPM, from) > 0, "polled again");
    drop((watch, device));
    lab.stop().await;
}

/// The lab device refuses the wildcard read, so the ask reads its Device
/// object by its instance. Answered with that identifier, it is still
/// there (two claimants); with another identifier, or no such object, it
/// left, and the point is given up as moved; with another error it can't
/// say, and nothing changes.
#[tokio::test(start_paused = true)]
async fn each_answer_to_the_ask_by_the_instance() {
    let cases = [
        ("its own identifier", None, false),
        ("another identifier", Some(Ok(LAB + 3)), true),
        ("no such object", Some(Err(ErrorCode::UNKNOWN_OBJECT)), true),
        (
            "another error",
            Some(Err(ErrorCode::UNKNOWN_PROPERTY)),
            false,
        ),
    ];
    for (case, own, moves) in cases {
        let lab = Lab::start(WatchOptions::default()).await;
        let device = lab_device(&lab).await;
        let mut watch = lab.client.watch([device.point("ai:1").unwrap()]).unwrap();
        until(&mut watch, |u| u.state == State::Subscribed).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        latest(&mut watch).await;
        lab.quirks.refuse_wildcard.store(true, Ordering::Relaxed);
        *lab.quirks.own_device.lock().unwrap() = own;
        let from = Instant::now();
        moved_away(&lab);
        tokio::time::sleep(Duration::from_secs(3)).await;
        let there = lab.sent_to(READ_PROPERTY, ELSEWHERE, from);
        assert_eq!(there, 1, "{case}: read there");
        let here = lab.sent_to(READ_PROPERTY, 2, from);
        assert_eq!(
            here, 2,
            "{case}: asked here by the wildcard, then the instance"
        );
        match moves {
            true => {
                let update = latest(&mut watch).await.expect("given up");
                let to = elsewhere();
                let moved = State::Gone {
                    why: Why::Moved { to },
                };
                assert_eq!(update.state, moved, "{case}");
            }
            false => none_given_up(&mut watch, 2).await,
        }
        drop((watch, device));
        lab.stop().await;
    }
}
