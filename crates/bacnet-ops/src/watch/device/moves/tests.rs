//! The move cues' parts on a scheduler built bare beside the lab's hub: a
//! queued read or ask whose points went sends nothing, the two-claimant
//! warning is kept per instance and address within a bound, and only a
//! timeout is the old address's silence.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use bacnet_npdu::{Mac, RejectMessageReason};
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::error::{Error, NetworkReject};

use super::super::super::hub::WatchOptions;
use super::super::super::jobs::Answer;
use super::super::super::shared::DeviceSlot;
use super::super::super::test_lab::{ELSEWHERE, LAB, Lab, READ_PROPERTY, lab_at};
use super::super::{Device, Op};
use super::{Cue, Moves, WARNED};
use crate::device::DeviceAddress;
use crate::device::confirm::left_silence;

/// A scheduler at the lab device's address with no points.
fn bare(lab: &Lab) -> Device<TestTransport> {
    Device {
        inner: Arc::clone(&lab.hub.inner),
        address: lab_at(),
        slot: Arc::new(DeviceSlot::new()),
        locating: BTreeSet::new(),
        ids: Default::default(),
        moves: Moves::default(),
        rows: BTreeMap::new(),
        ops: VecDeque::new(),
        backoff: Duration::ZERO,
        retry_at: None,
        last_polled: None,
    }
}

fn elsewhere(station: u8) -> DeviceAddress {
    DeviceAddress::local(&[station]).unwrap()
}

fn cue(to: &DeviceAddress) -> Option<Cue> {
    Some(Cue {
        instance: LAB,
        to: to.clone(),
        looked: false,
    })
}

/// A read at the new address, or an ask at the old one, queued while its
/// points went (their watches dropped): launched, it sends nothing.
#[tokio::test(start_paused = true)]
async fn a_queued_read_or_ask_whose_points_went_sends_nothing() {
    let lab = Lab::start(WatchOptions::default()).await;
    let mut device = bare(&lab);
    let to = elsewhere(ELSEWHERE);
    let from = tokio::time::Instant::now();
    device.moves.reading = cue(&to);
    device.ops.push_back(Op::Arrived(LAB, to.clone()));
    assert!(device.launch().is_none(), "no read at the new address");
    assert!(device.moves.reading.is_none());
    device.moves.asking = cue(&to);
    device.ops.push_back(Op::Left(LAB));
    assert!(device.launch().is_none(), "no ask at the old address");
    assert!(device.moves.asking.is_none());
    assert!(device.ops.is_empty());
    assert_eq!(lab.sent(READ_PROPERTY, from), 0);
    drop(device);
    lab.stop().await;
}

/// Two claimants are logged as a warning once a lifetime for each instance
/// and other address: another address within the lifetime is warned of
/// too, and no more than [`WARNED`] are remembered.
#[tokio::test(start_paused = true)]
async fn two_claimants_are_warned_of_per_address_within_a_bound() {
    let lab = Lab::start(WatchOptions::default()).await;
    let mut device = bare(&lab);
    device.claimants(LAB, &elsewhere(9));
    let first = device.moves.warned[&(LAB, elsewhere(9))];
    tokio::time::sleep(Duration::from_secs(10)).await;
    device.claimants(LAB, &elsewhere(9));
    assert_eq!(
        device.moves.warned[&(LAB, elsewhere(9))],
        first,
        "not again"
    );
    device.claimants(LAB, &elsewhere(10));
    assert_eq!(device.moves.warned.len(), 2, "another address is warned of");
    for station in 11..40 {
        device.claimants(LAB, &elsewhere(station));
    }
    assert_eq!(device.moves.warned.len(), WARNED);
    tokio::time::sleep(Duration::from_secs(301)).await;
    device.claimants(LAB, &elsewhere(9));
    assert!(
        device.moves.warned[&(LAB, elsewhere(9))] > first,
        "again a lifetime on"
    );
    assert_eq!(device.moves.warned.len(), 1, "the old ones lapsed");
    drop(device);
    lab.stop().await;
}

/// Asked whether the device left, only a timeout is its silence: a
/// router's reject, which anyone on a routed path can send, and the link
/// failing the send say nothing.
#[test]
fn only_a_timeout_is_the_old_addresss_silence() {
    let timeout = Error::Timeout(Duration::from_secs(3));
    assert_eq!(left_silence(&timeout), Some(Answer::Silent));
    let reject = NetworkReject::new(
        Mac::from_array([5]),
        None,
        9,
        RejectMessageReason::from_raw(1),
    );
    let rejected = Error::NetworkRejected(Box::new(reject));
    assert_eq!(left_silence(&rejected), Some(Answer::Failed));
    let link = Error::Transport(std::io::ErrorKind::BrokenPipe.into());
    assert_eq!(left_silence(&link), Some(Answer::Failed));
}
