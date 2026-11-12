//! A Load Control's shed request (`RBNLCT01`), as
//! `FileLoadControlPersistence` (with std) writes its file (#2479).
//!
//! After the magic tag and the four-octet object identifier, a body of 45
//! octets, each field big-endian:
//!
//! - the shed state (Present_Value), four octets;
//! - Requested_Shed_Level: one octet naming the choice (0 percent, 1 level,
//!   2 amount), then eight octets: the Unsigned value of a percent or a
//!   level, or the IEEE 754 single of an amount in the last four, after
//!   four zero octets;
//! - Start_Time: the date's year (past 1900), month, day and weekday, then
//!   the time's hour, minute, second and hundredths, an octet each, 255 for
//!   an unspecified field;
//! - Shed_Duration, Duty_Window and the agreed Duty_Window, eight octets
//!   each.
//!
//! The body must be exactly that long. A state outside the four shed
//! states, a choice past 2, a percent over 100, an amount that is negative
//! or not finite, and a Start_Time that is neither a moment nor wholly
//! unspecified are what no save writes, so they are damage.

use crate::prelude::*;
use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::ShedState;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::SnapshotError;
use super::framing;
use crate::clock::{UNSPECIFIED_DATETIME, local_moment};
use crate::load_control::LoadControlSnapshot;

/// The format's tag, version 1.
pub const MAGIC: [u8; 8] = *b"RBNLCT01";
/// The body's length.
const BODY_LEN: usize = 4 + 9 + 8 + 3 * 8;
/// The most octets a file may take: the header and the body.
pub const MAX_LEN: usize = framing::HEADER_LEN + BODY_LEN;

/// The file octets for `snapshot`, saved for the Load Control `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) what
/// [`decode`] would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &LoadControlSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    if !sound(snapshot) {
        return Err(SnapshotError::out_of_range("holds a value no write leaves")
            .in_format("Load Control snapshot"));
    }
    let mut body = Vec::with_capacity(BODY_LEN);
    body.extend_from_slice(&snapshot.state.to_raw().to_be_bytes());
    let (choice, value) = match snapshot.requested_shed_level {
        BACnetShedLevel::Percent(percent) => (0, percent),
        BACnetShedLevel::Level(level) => (1, level),
        BACnetShedLevel::Amount(amount) => (2, u64::from(amount.to_bits())),
    };
    body.push(choice);
    body.extend_from_slice(&value.to_be_bytes());
    let (date, time) = snapshot.start_time;
    body.extend_from_slice(&[date.year, date.month, date.day, date.day_of_week]);
    body.extend_from_slice(&[time.hour, time.minute, time.second, time.hundredths]);
    for minutes in [
        snapshot.shed_duration,
        snapshot.duty_window,
        snapshot.agreed_duty_window,
    ] {
        body.extend_from_slice(&minutes.to_be_bytes());
    }
    Ok(framing::seal(&MAGIC, object, &body))
}

/// The snapshot a file's `octets` hold for the Load Control `object`.
///
/// Refuses octets past [`MAX_LEN`], without the tag, saved for another
/// object, a body of another length, and a value no save writes.
pub fn decode(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<LoadControlSnapshot, SnapshotError> {
    decode_file(object, octets).map_err(|error| error.in_format("Load Control file"))
}

fn decode_file(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<LoadControlSnapshot, SnapshotError> {
    let (_, body) = framing::open(octets, &MAGIC, None, MAX_LEN, object)?;
    let body: &[u8; BODY_LEN] = match body.len() {
        BODY_LEN => body.try_into().expect("checked length"),
        len if len < BODY_LEN => return Err(SnapshotError::truncated("is cut short")),
        _ => return Err(SnapshotError::damaged("goes on past its body")),
    };
    let word = |at: usize| u64::from_be_bytes(body[at..at + 8].try_into().expect("eight octets"));
    let state = u32::from_be_bytes(body[..4].try_into().expect("four octets"));
    let value = word(5);
    let requested_shed_level = match body[4] {
        0 => BACnetShedLevel::Percent(value),
        1 => BACnetShedLevel::Level(value),
        2 if value <= u64::from(u32::MAX) => BACnetShedLevel::Amount(f32::from_bits(value as u32)),
        _ => return Err(SnapshotError::damaged("holds no shed level")),
    };
    let start_time = (
        Date {
            year: body[13],
            month: body[14],
            day: body[15],
            day_of_week: body[16],
        },
        Time {
            hour: body[17],
            minute: body[18],
            second: body[19],
            hundredths: body[20],
        },
    );
    let snapshot = LoadControlSnapshot {
        state: ShedState::from_raw(state),
        requested_shed_level,
        start_time,
        shed_duration: word(21),
        duty_window: word(29),
        agreed_duty_window: word(37),
    };
    if !sound(&snapshot) {
        return Err(SnapshotError::damaged("holds a value no write leaves"));
    }
    Ok(snapshot)
}

/// Whether `snapshot` holds only what the object serves: a shed state, a
/// level a write takes, and a Start_Time that names a moment or none.
fn sound(snapshot: &LoadControlSnapshot) -> bool {
    let state = snapshot.state.to_raw() <= ShedState::SHED_NON_COMPLIANT.to_raw();
    let level = match snapshot.requested_shed_level {
        BACnetShedLevel::Percent(percent) => percent <= 100,
        BACnetShedLevel::Level(_) => true,
        BACnetShedLevel::Amount(amount) => amount.is_finite() && amount >= 0.0,
    };
    let start =
        snapshot.start_time == UNSPECIFIED_DATETIME || local_moment(snapshot.start_time).is_some();
    state && level && start
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::SnapshotErrorKind;
    use bacnet_types::enums::ObjectType;

    fn oid(instance: u32) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::LOAD_CONTROL, instance).unwrap()
    }

    fn shedding() -> LoadControlSnapshot {
        LoadControlSnapshot {
            state: ShedState::SHED_COMPLIANT,
            requested_shed_level: BACnetShedLevel::Amount(12.5),
            start_time: (
                Date {
                    year: 126,
                    month: 11,
                    day: 11,
                    day_of_week: 3,
                },
                Time {
                    hour: 10,
                    minute: 5,
                    second: 0xFF,
                    hundredths: 0xFF,
                },
            ),
            shed_duration: 30,
            duty_window: 10,
            agreed_duty_window: 15,
        }
    }

    #[test]
    fn each_choice_round_trips_in_a_fixed_body() {
        for level in [
            BACnetShedLevel::Percent(80),
            BACnetShedLevel::Level(u64::MAX),
            BACnetShedLevel::Amount(12.5),
        ] {
            let snapshot = LoadControlSnapshot {
                requested_shed_level: level,
                ..shedding()
            };
            let octets = encode(oid(1), &snapshot).unwrap();
            assert_eq!(octets.len(), MAX_LEN);
            assert_eq!(octets[..8], MAGIC);
            assert_eq!(decode(oid(1), &octets).unwrap(), snapshot);
        }
        let idle = LoadControlSnapshot::default();
        assert_eq!(
            decode(oid(1), &encode(oid(1), &idle).unwrap()).unwrap(),
            idle
        );
    }

    #[test]
    fn damage_and_other_files_are_refused() {
        let octets = encode(oid(1), &shedding()).unwrap();
        let kind = |octets: &[u8]| decode(oid(1), octets).unwrap_err().kind();
        assert_eq!(
            decode(oid(2), &octets).unwrap_err().kind(),
            SnapshotErrorKind::OtherObject
        );
        assert_eq!(kind(&octets[..MAX_LEN - 1]), SnapshotErrorKind::Truncated);
        let mut longer = octets.clone();
        longer.push(0);
        assert_eq!(kind(&longer), SnapshotErrorKind::TooLarge);
        let header = framing::HEADER_LEN;
        // A state past the four, a fourth choice, and an hour unspecified
        // alone.
        for (at, octet) in [(header + 3, 9), (header + 4, 3), (header + 17, 0xFF)] {
            let mut damaged = octets.clone();
            damaged[at] = octet;
            assert_eq!(kind(&damaged), SnapshotErrorKind::Damaged, "octet {at}");
        }
        // A percent over 100, which a write refuses.
        let mut percent = encode(oid(1), &LoadControlSnapshot::default()).unwrap();
        percent[header + 4] = 0;
        percent[header + 12] = 101;
        assert_eq!(kind(&percent), SnapshotErrorKind::Damaged);
        for refused in [
            BACnetShedLevel::Percent(101),
            BACnetShedLevel::Amount(-1.0),
            BACnetShedLevel::Amount(f32::NAN),
        ] {
            let snapshot = LoadControlSnapshot {
                requested_shed_level: refused,
                ..shedding()
            };
            assert_eq!(
                encode(oid(1), &snapshot).unwrap_err().kind(),
                SnapshotErrorKind::OutOfRange
            );
        }
    }
}
