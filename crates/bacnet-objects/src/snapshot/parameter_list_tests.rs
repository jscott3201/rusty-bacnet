//! The list row of the written-configuration format's kind 6 (#2331): a
//! multi-state object's Alarm_Values, a count and then the states. Its
//! octets, its cap, and that no cut, lengthened or miscounted row reads as
//! a shorter valid list.

use bacnet_types::enums::ObjectType;

use super::parameters_tests::{encode, oid, snapshot, with_tail};
use super::{SnapshotErrorKind, write_config};
use crate::write_config::AlgorithmWriteConfig;

/// The two types that keep the row.
const KEEPERS: [ObjectType; 2] = [ObjectType::MULTI_STATE_INPUT, ObjectType::MULTI_STATE_VALUE];

/// The list's bit of the presence mask.
const MASK: [u8; 4] = [0, 0, 0, 0x80];

/// The most states the row holds: what a write of Alarm_Values may.
const CAP: u32 = 1024;

fn list(values: Vec<u32>) -> AlgorithmWriteConfig {
    AlgorithmWriteConfig {
        alarm_values: Some(values),
        ..Default::default()
    }
}

/// The tail of a file whose list row states `count` and holds `states`.
fn row(count: u32, states: &[u32]) -> Vec<u8> {
    let mut tail = MASK.to_vec();
    tail.extend_from_slice(&count.to_be_bytes());
    for state in states {
        tail.extend_from_slice(&state.to_be_bytes());
    }
    tail
}

fn refusal(kind: ObjectType, tail: &[u8]) -> SnapshotErrorKind {
    write_config::decode(oid(kind), &with_tail(kind, tail))
        .unwrap_err()
        .kind()
}

/// The row is its count in four octets and each state in four, in list
/// order, repeats kept. An empty list, which a write may leave, is a count
/// of zero and nothing more, and comes back as an empty list, not as no
/// list.
#[test]
fn the_row_is_a_count_and_then_each_state() {
    for kind in KEEPERS {
        let plain = encode(kind, AlgorithmWriteConfig::default()).unwrap();
        for values in [vec![], vec![7], vec![2, 2, 0x0102_0304, 1]] {
            let octets = encode(kind, list(values.clone())).unwrap();
            assert_eq!(
                octets[plain.len()..],
                row(values.len() as u32, &values),
                "{kind}"
            );
            assert_eq!(
                write_config::decode(oid(kind), &octets),
                Ok(snapshot(list(values)))
            );
        }
    }
}

/// The cap is the 1,024 entries a write of Alarm_Values may hold: a list of
/// that many is kept, one more is refused as it is encoded, and a file
/// stating more is damage.
#[test]
fn the_cap_is_what_a_write_may_hold() {
    assert_eq!(CAP as usize, crate::multistate::MAX_ALARM_VALUES);
    for kind in KEEPERS {
        let full: Vec<u32> = (1..=CAP).collect();
        let octets = encode(kind, list(full.clone())).unwrap();
        assert_eq!(
            write_config::decode(oid(kind), &octets),
            Ok(snapshot(list(full)))
        );
        let over: Vec<u32> = (1..=CAP + 1).collect();
        let error = encode(kind, list(over.clone())).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{kind}");
        assert_eq!(
            refusal(kind, &row(CAP + 1, &over)),
            SnapshotErrorKind::Damaged,
            "{kind}"
        );
    }
}

/// A count past the cap is refused as damage at the count, whatever
/// follows it: with no state at all after it the file is not read as cut
/// short, so nothing was sized by the count first.
#[test]
fn a_count_past_the_cap_is_refused_before_a_state_is_read() {
    for kind in KEEPERS {
        for count in [CAP + 1, 0x4000_0000, 0x7FFF_FFFF, u32::MAX] {
            for states in [&[][..], &[1, 2]] {
                assert_eq!(
                    refusal(kind, &row(count, states)),
                    SnapshotErrorKind::Damaged,
                    "{kind} count {count:#x}"
                );
            }
        }
    }
}

/// A count within the cap that the file doesn't hold is a file cut short,
/// and one the file goes on past leaves octets after the body's end: no
/// miscounted row reads as a list.
#[test]
fn a_count_the_file_does_not_match_is_refused() {
    for kind in KEEPERS {
        for (count, states) in [(1, &[][..]), (3, &[1, 2]), (CAP, &[1])] {
            assert_eq!(
                refusal(kind, &row(count, states)),
                SnapshotErrorKind::Truncated,
                "{kind} count {count}"
            );
        }
        for (count, states) in [(0, &[1][..]), (2, &[1, 2, 3])] {
            assert_eq!(
                refusal(kind, &row(count, states)),
                SnapshotErrorKind::Damaged,
                "{kind} count {count}"
            );
        }
        // Part of a state is neither.
        let mut ragged = row(2, &[1, 2]);
        ragged.pop();
        assert_eq!(refusal(kind, &ragged), SnapshotErrorKind::Truncated);
        ragged.extend_from_slice(&[2, 0]);
        assert_eq!(refusal(kind, &ragged), SnapshotErrorKind::Damaged);
    }
}

/// Every cut of a file with a list is refused, and so is the same file
/// going on past the list's last state.
#[test]
fn no_cut_reads_as_the_list_it_was_cut_from() {
    for kind in KEEPERS {
        let values = vec![4, 9, 2, 6];
        let octets = encode(kind, list(values.clone())).unwrap();
        for cut in 12..octets.len() {
            let error = write_config::decode(oid(kind), &octets[..cut]).unwrap_err();
            assert!(error.is_damage(), "{kind} cut at {cut}");
        }
        let mut long = octets.clone();
        long.extend_from_slice(&[0, 0, 0, 5]);
        let error = write_config::decode(oid(kind), &long).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{kind}");
    }
}

/// State zero names no state: a write refuses it, so it is refused as it
/// is encoded and as damage in a file, wherever it sits in the list. Any
/// other state is the object's to check against its Number_Of_States.
#[test]
fn a_state_of_zero_is_refused() {
    for kind in KEEPERS {
        for values in [vec![0], vec![1, 0], vec![0, 3, 3], vec![5, 0, 7]] {
            let error = encode(kind, list(values.clone())).unwrap_err();
            assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{values:?}");
            assert_eq!(
                refusal(kind, &row(values.len() as u32, &values)),
                SnapshotErrorKind::Damaged,
                "{kind} {values:?}"
            );
        }
        let edge = vec![1, u32::MAX];
        let octets = encode(kind, list(edge.clone())).unwrap();
        assert_eq!(
            write_config::decode(oid(kind), &octets),
            Ok(snapshot(list(edge)))
        );
    }
}
