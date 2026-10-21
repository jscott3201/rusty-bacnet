//! Digital resolution never repairs physical fragments or explicit faults.
use super::*;
use crate::{ContentionPolicy, SimRng};

fn us(value: u64) -> Instant {
    Instant::from_micros(value)
}

fn tx(line: &mut Line, station: u8, at: u64, corruption: Option<Corruption>) -> TxId {
    let tx = line.begin(
        line.timing,
        Driver::Station(station),
        vec![0x55],
        us(at),
        corruption.map(|c| (0, c)),
        Duration::ZERO,
        Duration::ZERO,
    );
    line.start_octet(tx, 0, us(at));
    tx
}

fn resolve(outcome: &WindowOutcome) -> Character {
    crate::contention::resolve(
        ContentionPolicy::Capture(Driver::Station(1)),
        outcome,
        &mut SimRng::new(1),
    )
    .0
}

#[test]
fn misaligned_contributors_are_errors_on_the_unchanged_grid() {
    let mut line = Line::new(WireTiming::new(10_000));
    tx(&mut line, 1, 0, None);
    tx(&mut line, 2, 500, None);
    let first = line.end_window(us(1_000)).unwrap();
    assert_eq!(first.start, us(0));
    assert_eq!(first.reopened, Some(us(2_000)));
    assert_eq!(resolve(&first), Character::Garbled);
    let second = line.end_window(us(2_000)).unwrap();
    assert_eq!(second.start, us(1_000));
    assert_eq!(resolve(&second), Character::Garbled);
    assert_eq!(line.last_activity(), Some(us(2_000)));
}

#[test]
fn explicit_errors_truncation_and_xor_faults_are_not_healed() {
    for error in [Corruption::FramingError, Corruption::Xor(1)] {
        let mut line = Line::new(WireTiming::new(10_000));
        tx(&mut line, 1, 0, Some(error));
        tx(&mut line, 2, 0, None);
        let outcome = line.end_window(us(1_000)).unwrap();
        let expected = if error == Corruption::FramingError {
            Character::Garbled
        } else {
            Character::Clean(0x54)
        };
        assert_eq!(resolve(&outcome), expected);
        // The same XOR applied by the synthetic policy must not undo the
        // independently injected physical corruption.
        let policy = ContentionPolicy::Xor {
            driver: Driver::Station(1),
            offset: 0,
            mask: 1,
        };
        assert_eq!(
            crate::contention::resolve(policy, &outcome, &mut SimRng::new(1)).0,
            expected
        );
    }
    let mut line = Line::new(WireTiming::new(10_000));
    tx(&mut line, 1, 0, None);
    let other = tx(&mut line, 2, 0, None);
    line.truncate(other, us(500));
    let outcome = line.end_window(us(1_000)).unwrap();
    assert_eq!(resolve(&outcome), Character::Garbled);
}
