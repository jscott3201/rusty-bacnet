//! A log slot's length checks, and the two-slot choice [`log::newest`]
//! makes for the file backend and any other.

use bacnet_types::constructed::BACnetLogRecord;

use super::tests::{reseal, trend};
use super::{SnapshotErrorKind, log};

fn slot(generation: u64) -> Vec<u8> {
    let mut snapshot = trend();
    snapshot.generation = generation;
    log::encode(&snapshot).unwrap()
}

/// `slot` with its declared payload length set to `length`.
fn declaring(length: u32) -> Vec<u8> {
    let mut octets = slot(3);
    octets[22..26].copy_from_slice(&length.to_be_bytes());
    reseal(&mut octets);
    octets
}

/// Erased flash reads as all ones: a length field near `u32::MAX` must be
/// refused as cut short, never overflow (a 32-bit `usize` would).
#[test]
fn a_huge_declared_length_is_truncation_not_a_panic() {
    let object = trend().object_identifier;
    for length in [u32::MAX, 0xFFFF_FFE2, 0xFFFF_FFE1, u32::MAX / 2] {
        let error = log::decode::<BACnetLogRecord>(object, &declaring(length)).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Truncated, "{length:#x}");
    }
    let erased = vec![0xFF; slot(3).len()];
    let error = log::decode::<BACnetLogRecord>(object, &erased).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::NotThisFormat);
}

#[test]
fn a_slot_past_the_cap_is_too_large() {
    let mut octets = slot(3);
    // Zeroed pages: nothing is touched before the length is refused.
    octets.resize(log::MAX_LEN + 1, 0);
    let error = log::decode::<BACnetLogRecord>(trend().object_identifier, &octets).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::TooLarge);
    assert!(error.is_damage());
}

fn newest(
    slots: [Option<&[u8]>; 2],
) -> Result<Option<log::NewestSlot<BACnetLogRecord>>, super::SnapshotError> {
    log::newest(trend().object_identifier, slots)
}

#[test]
fn the_newer_sound_slot_wins_and_damage_is_passed_over() {
    assert_eq!(newest([None, None]), Ok(None));
    let (three, four) = (slot(3), slot(4));
    let chosen = newest([Some(&three), Some(&four)]).unwrap().unwrap();
    assert_eq!((chosen.slot, chosen.generation), (1, 4));
    assert_eq!(chosen.snapshot.generation, 4);
    assert_eq!(chosen.passed_over, [None, None]);

    let cut = &four[..four.len() - 1];
    let chosen = newest([Some(&three), Some(cut)]).unwrap().unwrap();
    assert_eq!((chosen.slot, chosen.generation), (0, 3));
    assert_eq!(
        chosen.passed_over[1].map(|error| error.kind()),
        Some(SnapshotErrorKind::Truncated)
    );
}

/// With no slot to load, damage is an error, not an empty store.
#[test]
fn damage_with_nothing_sound_is_an_error() {
    let four = slot(4);
    let cut = &four[..four.len() - 1];
    for slots in [[Some(cut), None], [None, Some(cut)], [Some(cut), Some(cut)]] {
        let error = newest(slots).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::Truncated);
    }
}

#[test]
fn one_generation_twice_must_hold_one_snapshot() {
    let three = slot(3);
    let chosen = newest([Some(&three), Some(&three)]).unwrap().unwrap();
    assert_eq!((chosen.slot, chosen.generation), (0, 3));

    let mut other = trend();
    other.generation = 3;
    other.notification_threshold += 1;
    let other = log::encode(&other).unwrap();
    let error = newest([Some(&three), Some(&other)]).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
    assert!(error.to_string().contains("one generation"), "{error}");
}

/// A sound slot this build must not take stops the load: no falling back
/// to the older slot.
#[test]
fn a_refused_slot_stops_the_choice() {
    let mut newer = slot(4);
    newer[8..10].copy_from_slice(&9u16.to_be_bytes());
    reseal(&mut newer);
    let three = slot(3);
    let error = newest([Some(&three), Some(&newer)]).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::UnknownVersion(9));
}
