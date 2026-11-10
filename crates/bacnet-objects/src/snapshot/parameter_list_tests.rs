//! The list rows of the written-configuration format's kind 6 (#2331):
//! Alarm_Values on a multi-state object, an Access Door and an Access Zone,
//! and a door's Fault_Values and Masked_Alarm_Values, each a count and then
//! the states. Their octets, their cap, that no cut, lengthened or
//! miscounted row reads as a shorter valid list, and the rule each object
//! type holds a listed state to.

use bacnet_types::enums::ObjectType;

use super::parameters_tests::{encode, listed, oid, snapshot, with_tail};
use super::{SnapshotErrorKind, write_config};
use crate::write_config::AlgorithmWriteConfig;

/// One list row on one of the types that keep it: the type and the row's
/// bit of the presence mask.
#[derive(Clone, Copy, Debug)]
struct Row {
    kind: ObjectType,
    bit: u32,
}

/// Every list row on every type that keeps it.
const ROWS: [Row; 6] = [
    Row::new(ObjectType::MULTI_STATE_INPUT, 7),
    Row::new(ObjectType::MULTI_STATE_VALUE, 7),
    Row::new(ObjectType::ACCESS_DOOR, 7),
    Row::new(ObjectType::ACCESS_DOOR, 8),
    Row::new(ObjectType::ACCESS_DOOR, 9),
    Row::new(ObjectType::ACCESS_ZONE, 7),
];

/// The most states a row holds: what a write of the list may.
const CAP: u32 = 1024;

impl Row {
    const fn new(kind: ObjectType, bit: u32) -> Self {
        Self {
            kind,
            bit: 1 << bit,
        }
    }

    /// A snapshot holding `values` in this row and nothing else.
    fn list(self, values: Vec<u32>) -> AlgorithmWriteConfig {
        let mut config = AlgorithmWriteConfig::default();
        match self.bit {
            0x80 => config.alarm_values = Some(values),
            0x100 => config.fault_values = Some(values),
            _ => config.masked_alarm_values = Some(values),
        }
        config
    }

    /// The tail of a file whose row states `count` and holds `states`.
    fn tail(self, count: u32, states: &[u32]) -> Vec<u8> {
        let mut tail = self.bit.to_be_bytes().to_vec();
        tail.extend_from_slice(&count.to_be_bytes());
        for state in states {
            tail.extend_from_slice(&state.to_be_bytes());
        }
        tail
    }

    fn refusal(self, tail: &[u8]) -> SnapshotErrorKind {
        write_config::decode(oid(self.kind), &with_tail(self.kind, tail))
            .unwrap_err()
            .kind()
    }

    /// `count` different states the type's lists take: a multi-state
    /// object's from 1, and the proprietary states of the two enumerations
    /// from where each begins.
    fn states(self, count: u32) -> Vec<u32> {
        let first = match self.kind {
            ObjectType::ACCESS_DOOR => 256,
            ObjectType::ACCESS_ZONE => 64,
            _ => 1,
        };
        (first..first + count).collect()
    }
}

/// The rows here are the list rows of the codec's own table: bits 7, 8 and
/// 9, on the types that keep each.
#[test]
fn the_rows_are_the_tables_list_rows() {
    use bacnet_types::enums::PropertyIdentifier as P;
    let lists = [
        (P::ALARM_VALUES, 0x80, 4),
        (P::FAULT_VALUES, 0x100, 1),
        (P::MASKED_ALARM_VALUES, 0x200, 1),
    ];
    for (property, bit, keepers) in lists {
        assert_eq!(write_config::parameters::bit_of(property), Some(bit));
        let rows = ROWS.iter().filter(|row| row.bit == bit);
        assert_eq!(rows.clone().count(), keepers, "{property:?}");
        for row in rows {
            assert!(write_config::keeps_parameter(row.kind, property), "{row:?}");
        }
    }
}

/// The row is its count in four octets and each state in four, in list
/// order, repeats kept. An empty list, which a write may leave, is a count
/// of zero and nothing more, and comes back as an empty list, not as no
/// list.
#[test]
fn the_row_is_a_count_and_then_each_state() {
    for row in ROWS {
        let plain = encode(row.kind, AlgorithmWriteConfig::default()).unwrap();
        let some = row.states(3);
        for values in [
            vec![],
            vec![some[2]],
            vec![some[1], some[1], some[2], some[0]],
        ] {
            let octets = encode(row.kind, row.list(values.clone())).unwrap();
            assert_eq!(
                octets[plain.len()..],
                row.tail(values.len() as u32, &values),
                "{row:?}"
            );
            assert_eq!(
                write_config::decode(oid(row.kind), &octets),
                Ok(snapshot(row.list(values)))
            );
        }
    }
}

/// The cap is the 1,024 entries a write of such a list may hold, on every
/// type: a list of that many is kept, one more is refused as it is encoded,
/// and a file stating more is damage.
#[test]
fn the_cap_is_what_a_write_may_hold() {
    assert_eq!(CAP as usize, crate::multistate::MAX_ALARM_VALUES);
    for row in ROWS {
        let full = row.states(CAP);
        let octets = encode(row.kind, row.list(full.clone())).unwrap();
        assert_eq!(
            write_config::decode(oid(row.kind), &octets),
            Ok(snapshot(row.list(full)))
        );
        let over = row.states(CAP + 1);
        let error = encode(row.kind, row.list(over.clone())).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{row:?}");
        assert_eq!(
            row.refusal(&row.tail(CAP + 1, &over)),
            SnapshotErrorKind::Damaged,
            "{row:?}"
        );
    }
}

/// A count past the cap is refused as damage at the count, whatever
/// follows it: with no state at all after it the file is not read as cut
/// short, so nothing was sized by the count first.
#[test]
fn a_count_past_the_cap_is_refused_before_a_state_is_read() {
    for row in ROWS {
        let some = row.states(2);
        for count in [CAP + 1, 0x4000_0000, 0x7FFF_FFFF, u32::MAX] {
            for states in [&[][..], &some] {
                assert_eq!(
                    row.refusal(&row.tail(count, states)),
                    SnapshotErrorKind::Damaged,
                    "{row:?} count {count:#x}"
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
    for row in ROWS {
        let some = row.states(3);
        for (count, states) in [(1, &[][..]), (3, &some[..2]), (CAP, &some[..1])] {
            assert_eq!(
                row.refusal(&row.tail(count, states)),
                SnapshotErrorKind::Truncated,
                "{row:?} count {count}"
            );
        }
        for (count, states) in [(0, &some[..1]), (2, &some[..])] {
            assert_eq!(
                row.refusal(&row.tail(count, states)),
                SnapshotErrorKind::Damaged,
                "{row:?} count {count}"
            );
        }
        // Part of a state is neither.
        let mut ragged = row.tail(2, &some[..2]);
        ragged.pop();
        assert_eq!(row.refusal(&ragged), SnapshotErrorKind::Truncated);
        ragged.extend_from_slice(&[2, 0]);
        assert_eq!(row.refusal(&ragged), SnapshotErrorKind::Damaged);
    }
}

/// Every cut of a file with a list is refused, and so is the same file
/// going on past the list's last state.
#[test]
fn no_cut_reads_as_the_list_it_was_cut_from() {
    for row in ROWS {
        let values = row.states(4);
        let octets = encode(row.kind, row.list(values.clone())).unwrap();
        for cut in 12..octets.len() {
            let error = write_config::decode(oid(row.kind), &octets[..cut]).unwrap_err();
            assert!(error.is_damage(), "{row:?} cut at {cut}");
        }
        let mut long = octets.clone();
        long.extend_from_slice(&[0, 0, 0, 5]);
        let error = write_config::decode(oid(row.kind), &long).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Damaged, "{row:?}");
    }
}

/// The states a list of `kind` takes and refuses, at the edges of its
/// rule. A multi-state object's state is any number but zero. An Access
/// Door's is a BACnetDoorAlarmState other than NORMAL, the eight named
/// after it or a proprietary one from 256 to 65535; an Access Zone's a
/// BACnetAccessZoneOccupancyState other than NORMAL, the six named after it
/// or a proprietary one from 64 to 65535. NORMAL is zero in both
/// enumerations, and a write of either object's lists refuses it.
fn edges(kind: ObjectType) -> (&'static [u32], &'static [u32]) {
    match kind {
        ObjectType::ACCESS_DOOR => (
            &[1, 8, 256, 65_535],
            &[0, 9, 64, 255, 65_536, 0x0102_0304, u32::MAX],
        ),
        ObjectType::ACCESS_ZONE => (
            &[1, 6, 64, 255, 65_535],
            &[0, 7, 8, 63, 65_536, 0x0102_0304, u32::MAX],
        ),
        _ => (&[1, 9, 65_536, u32::MAX], &[0]),
    }
}

/// A state the type's lists don't take is refused as it is encoded and as
/// damage in a file, wherever it sits in the list, and every state at the
/// edge of what they take is kept. Zero is refused on every type: it names
/// no state of a multi-state object, and it is NORMAL on a door and a zone.
#[test]
fn a_state_the_types_lists_do_not_take_is_refused() {
    for row in ROWS {
        let (taken, refused) = edges(row.kind);
        assert!(refused.contains(&0), "{row:?}");
        let good = taken[0];
        for &bad in refused {
            for values in [vec![bad], vec![good, bad], vec![bad, good, good]] {
                let error = encode(row.kind, row.list(values.clone())).unwrap_err();
                assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{values:?}");
                assert_eq!(
                    row.refusal(&row.tail(values.len() as u32, &values)),
                    SnapshotErrorKind::Damaged,
                    "{row:?} {values:?}"
                );
            }
        }
        let octets = encode(row.kind, row.list(taken.to_vec())).unwrap();
        assert_eq!(
            write_config::decode(oid(row.kind), &octets),
            Ok(snapshot(row.list(taken.to_vec())))
        );
    }
}

/// A list is read under the rule of the type its file is for, and no
/// other. An Access Door's sample list holds a state an Access Zone has
/// none for, an Access Zone's a state below a door's proprietary ones, and
/// a multi-state object's a state past both enumerations. So a door's file
/// refuses the other two lists as damage and a zone's does too, each
/// taking its own; a multi-state object's file, whose rule is any state
/// but zero, takes all three.
#[test]
fn a_list_is_read_under_its_own_types_rule() {
    let keepers = [
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::ACCESS_DOOR,
        ObjectType::ACCESS_ZONE,
    ];
    let row = |kind| ROWS.into_iter().find(|row| row.kind == kind).unwrap();
    let mut refused = 0;
    for written_for in keepers {
        let list = listed(written_for);
        for read_as in keepers {
            let reader = row(read_as);
            let tail = reader.tail(list.len() as u32, &list);
            let decoded = write_config::decode(oid(read_as), &with_tail(read_as, &tail));
            let encoded = encode(read_as, reader.list(list.clone()));
            if read_as == written_for || read_as == ObjectType::MULTI_STATE_INPUT {
                assert_eq!(decoded, Ok(snapshot(reader.list(list.clone()))));
                assert!(encoded.is_ok(), "{read_as}");
            } else {
                let error = decoded.unwrap_err();
                assert_eq!(
                    error.kind(),
                    SnapshotErrorKind::Damaged,
                    "{written_for}'s list read as {read_as}'s"
                );
                let error = encoded.unwrap_err();
                assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange, "{read_as}");
                refused += 1;
            }
        }
    }
    assert_eq!(refused, 4);
    // A door's two other lists are held to the door's rule too.
    for bit in [8, 9] {
        let reader = Row::new(ObjectType::ACCESS_DOOR, bit);
        for list in [
            listed(ObjectType::MULTI_STATE_INPUT),
            listed(ObjectType::ACCESS_ZONE),
        ] {
            let tail = reader.tail(list.len() as u32, &list);
            assert_eq!(reader.refusal(&tail), SnapshotErrorKind::Damaged, "{bit}");
        }
    }
}
