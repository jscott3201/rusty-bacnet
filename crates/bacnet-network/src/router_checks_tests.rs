//! The router rule without I/O (#2823, #2831): what one router query
//! found, the two strikes, and the pacing.

use super::*;
use crate::layer::ReceivedNetworkReport;
use bacnet_encoding::npdu::NpduAddress;

const NETWORK: u16 = 100;
const OLD: [u8; 1] = [2];

fn mac(octets: &[u8]) -> MacAddr {
    MacAddr::from_slice(octets)
}

fn announcement(from: &[u8], networks: &[u16]) -> ReceivedNetworkReport {
    ReceivedNetworkReport {
        source_mac: mac(from),
        source_network: None,
        report: NetworkReport::IAmRouterToNetwork(networks.to_vec()),
    }
}

fn answers(reports: &[ReceivedNetworkReport], old: Option<&[u8]>) -> Routers {
    let mut routers = Routers::default();
    for report in reports {
        routers.record(report, NETWORK, old);
    }
    routers
}

#[test]
fn only_a_local_announcement_listing_the_network_answers() {
    let mut relayed = announcement(&[7], &[NETWORK]);
    relayed.source_network = Some(NpduAddress {
        network: 5,
        mac_address: mac(&[1]),
    });
    let other_network = announcement(&[7], &[NETWORK + 1]);
    let not_a_router_answer = ReceivedNetworkReport {
        source_mac: mac(&[7]),
        source_network: None,
        report: NetworkReport::NetworkNumberIs {
            network: NETWORK,
            configured: true,
        },
    };
    let routers = answers(&[relayed, other_network, not_a_router_answer], Some(&OLD));
    assert_eq!(routers, Routers::default());
    assert_eq!(routers.answer(), Answer::Nobody);
}

#[test]
fn the_answer_follows_who_answered() {
    let one = answers(&[announcement(&[7], &[1, NETWORK])], Some(&OLD));
    assert_eq!(one.answer(), Answer::One(mac(&[7])));
    // The same router twice is one router.
    let twice = answers(
        &[
            announcement(&[7], &[NETWORK]),
            announcement(&[7], &[NETWORK]),
        ],
        Some(&OLD),
    );
    assert_eq!(twice.answer(), Answer::One(mac(&[7])));
    let several = answers(
        &[
            announcement(&[7], &[NETWORK]),
            announcement(&[8], &[NETWORK]),
            announcement(&[9], &[NETWORK]),
        ],
        Some(&OLD),
    );
    assert_eq!(several.others.len(), 2, "two are enough to refuse");
    assert_eq!(several.answer(), Answer::Several);
    // The old router answering keeps it, whoever else answers.
    let old = answers(
        &[
            announcement(&[7], &[NETWORK]),
            announcement(&OLD, &[NETWORK]),
        ],
        Some(&OLD),
    );
    assert_eq!(old.answer(), Answer::Old);
    // With no router known, the old router is just another station.
    let first = answers(&[announcement(&OLD, &[NETWORK])], None);
    assert_eq!(first.answer(), Answer::One(mac(&OLD)));
    assert!(Answer::Old.kept_why().is_some());
    assert!(Answer::One(mac(&[7])).kept_why().is_none());
}

#[test]
fn two_strikes_for_the_same_router_move_it() {
    let began = Instant::now();
    let to = mac(&[7]);
    let first = match Strike::step(None, to.clone(), began, false) {
        Step::Struck(strike) => strike,
        step => panic!("{step:?}"),
    };
    assert_eq!(first.since, began);
    let later = began + STRIKE_GAP;
    assert_eq!(Strike::since(Some(&first), later), began);
    assert_eq!(Strike::since(None, later), later);
    assert_eq!(
        Strike::step(Some(&first), to.clone(), later, false),
        Step::Moved(to.clone())
    );
    assert_eq!(
        Strike::step(Some(&first), mac(&[8]), later, false),
        Step::Differs
    );
    // Anything through the old router since the first began clears it.
    assert_eq!(
        Strike::step(Some(&first), to.clone(), later, true),
        Step::Answered
    );
    assert_eq!(Strike::step(None, to, began, true), Step::Answered);
}

fn pacing() -> Pacing<u16, u8> {
    Pacing::new(2)
}

#[test]
fn a_key_is_checked_at_once_then_paced_by_a_doubling_hold() {
    let mut paced = pacing();
    let at = Instant::now();
    paced.cue(1, 0, at);
    assert_eq!(paced.next_due(), Some(at));
    assert_eq!(paced.due(at, 8, |_| Some(0)), [1]);
    let mut now = at;
    for hold in [60, 120, 240, 480, 600, 600] {
        paced.checked(&1, Checked::Kept, now);
        // No cue, nothing due.
        assert_eq!(paced.next_due(), None);
        paced.cue(1, 0, now);
        let due = now + Duration::from_secs(hold);
        assert_eq!(paced.next_due(), Some(due), "hold {hold}");
        assert!(
            paced
                .due(due - Duration::from_secs(1), 8, |_| Some(0))
                .is_empty()
        );
        assert_eq!(paced.due(due, 8, |_| Some(0)), [1]);
        now = due;
    }
}

#[test]
fn a_strike_brings_the_second_check_a_gap_later_and_a_kept_one_clears_it() {
    let mut paced = pacing();
    let at = Instant::now();
    paced.cue(1, 0, at);
    let strike = Strike {
        to: mac(&[7]),
        since: at,
    };
    paced.checked(&1, Checked::Struck(strike.clone()), at);
    assert_eq!(paced.strike(&1), Some(strike.clone()));
    assert_eq!(
        paced.next_due(),
        Some(at + STRIKE_GAP),
        "wanted with no cue"
    );
    paced.checked(&1, Checked::Kept, at + STRIKE_GAP);
    assert_eq!(paced.strike(&1), None);
    paced.checked(&1, Checked::Struck(strike), at);
    paced.clear_strike(&1);
    assert_eq!(paced.strike(&1), None);
    paced.checked(&1, Checked::Ended, at);
    assert!(!paced.contains(&1));
}

#[test]
fn a_full_store_keeps_the_higher_ranked_keys() {
    let mut paced = pacing();
    let at = Instant::now();
    paced.cue(1, 5, at);
    paced.cue(2, 3, at);
    // Ranked below everything held: dropped.
    paced.cue(3, 1, at);
    assert!(!paced.contains(&3));
    // Outranks the lowest: takes its place.
    paced.cue(4, 4, at);
    assert!(paced.contains(&4) && !paced.contains(&2));
    // Due keys come highest ranked first, up to the limit.
    assert_eq!(paced.due(at, 8, |key| Some(*key as u8)), [4, 1]);
    assert_eq!(paced.due(at, 1, |key| Some(*key as u8)), [4]);
    // A key the owner no longer ranks is forgotten.
    assert_eq!(paced.due(at, 8, |key| (*key == 1).then_some(5)), [1]);
    assert!(!paced.contains(&4));
}

#[test]
fn a_key_left_alone_for_a_whole_last_hold_is_forgotten() {
    let mut paced = pacing();
    let at = Instant::now();
    paced.cue(1, 0, at);
    paced.checked(&1, Checked::Kept, at);
    let due = at + FIRST_HOLD;
    assert!(
        paced
            .due(due + LAST_HOLD - Duration::from_secs(1), 8, |_| Some(0))
            .is_empty()
    );
    assert!(paced.contains(&1));
    assert!(paced.due(due + LAST_HOLD, 8, |_| Some(0)).is_empty());
    assert!(!paced.contains(&1));
    // A cue keeps it, however late.
    paced.cue(2, 0, at);
    paced.checked(&2, Checked::Kept, at);
    paced.cue(2, 0, at);
    assert_eq!(paced.due(due + 2 * LAST_HOLD, 8, |_| Some(0)), [2]);
    paced.remove(&2);
    assert!(!paced.contains(&2));
}
