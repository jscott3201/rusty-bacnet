//! A Load Control's shed request (`RBNLCT01`), as
//! `FileLoadControlPersistence` (with std) writes its file (#2479).
//!
//! After the magic tag and the four-octet object identifier, the body, each
//! field big-endian:
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
//!   each;
//! - Enable, one octet, 0 or 1;
//! - a presence octet, 0 or 1, for a written Description, and after a 1
//!   the Description: its length in four octets, then at most 16 KiB of
//!   UTF-8.
//!
//! The body ends where its last field does. A state outside the four shed
//! states, a choice past 2, a percent over 100, an amount that is negative
//! or not finite, a Start_Time that is neither a moment nor wholly
//! unspecified, an Enable or presence octet other than 0 or 1, octets past
//! the end, and a Description past its cap or not UTF-8 are what no save
//! writes, so they are damage.

use crate::prelude::*;
use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::ShedState;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::SnapshotError;
use super::framing;
use crate::clock::{UNSPECIFIED_DATETIME, local_moment};
use crate::load_control::LoadControlSnapshot;
use crate::write_config::description::MAX_OCTETS as MAX_DESCRIPTION_OCTETS;

/// The format's tag, version 1.
pub const MAGIC: [u8; 8] = *b"RBNLCT01";
/// The fixed fields' length: the state, the level, Start_Time, the three
/// durations, Enable and the Description's presence octet.
const FIXED_LEN: usize = 4 + 9 + 8 + 3 * 8 + 2;
/// The most octets a file may take: the header, the fixed fields and the
/// longest Description.
pub const MAX_LEN: usize = framing::HEADER_LEN + FIXED_LEN + 4 + MAX_DESCRIPTION_OCTETS;

/// The file octets for `snapshot`, saved for the Load Control `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) what
/// [`decode`] would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &LoadControlSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_body(snapshot)
        .map(|body| framing::seal(&MAGIC, object, &body))
        .map_err(|error| error.in_format("Load Control snapshot"))
}

fn encode_body(snapshot: &LoadControlSnapshot) -> Result<Vec<u8>, SnapshotError> {
    if !sound(snapshot) {
        return Err(SnapshotError::out_of_range("holds a value no write leaves"));
    }
    let mut body = Vec::with_capacity(FIXED_LEN);
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
    body.push(u8::from(snapshot.enable));
    body.push(u8::from(snapshot.description.is_some()));
    if let Some(text) = &snapshot.description {
        framing::put_description(&mut body, text)?;
    }
    Ok(body)
}

/// The snapshot a file's `octets` hold for the Load Control `object`.
///
/// Refuses octets past [`MAX_LEN`], without the tag, saved for another
/// object, cut short, going on past the body's end, and a value no save
/// writes.
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
    let Some((fixed, rest)) = body.split_first_chunk::<FIXED_LEN>() else {
        return Err(SnapshotError::truncated("is cut short"));
    };
    let word = |at: usize| u64::from_be_bytes(fixed[at..at + 8].try_into().expect("eight octets"));
    let flag = |at: usize| match fixed[at] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(SnapshotError::damaged("holds a flag other than 0 or 1")),
    };
    let state = u32::from_be_bytes(fixed[..4].try_into().expect("four octets"));
    let value = word(5);
    let requested_shed_level = match fixed[4] {
        0 => BACnetShedLevel::Percent(value),
        1 => BACnetShedLevel::Level(value),
        2 if value <= u64::from(u32::MAX) => BACnetShedLevel::Amount(f32::from_bits(value as u32)),
        _ => return Err(SnapshotError::damaged("holds no shed level")),
    };
    let start_time = (
        Date {
            year: fixed[13],
            month: fixed[14],
            day: fixed[15],
            day_of_week: fixed[16],
        },
        Time {
            hour: fixed[17],
            minute: fixed[18],
            second: fixed[19],
            hundredths: fixed[20],
        },
    );
    let enable = flag(45)?;
    let (description, rest) = if flag(46)? {
        let (text, rest) = framing::take_description(rest)?;
        (Some(text), rest)
    } else {
        (None, rest)
    };
    if !rest.is_empty() {
        return Err(SnapshotError::damaged("goes on past its body"));
    }
    let snapshot = LoadControlSnapshot {
        state: ShedState::from_raw(state),
        requested_shed_level,
        start_time,
        shed_duration: word(21),
        duty_window: word(29),
        agreed_duty_window: word(37),
        enable,
        description,
    };
    if !sound(&snapshot) {
        return Err(SnapshotError::damaged("holds a value no write leaves"));
    }
    Ok(snapshot)
}

/// Whether `snapshot` holds only what the object serves: a shed state, a
/// level a write takes, and a Start_Time that names a moment or none. A
/// Description past its cap is refused as it is written.
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
            enable: true,
            description: Some("Chillers".into()),
        }
    }

    const HEADER: usize = framing::HEADER_LEN;

    #[test]
    fn each_choice_round_trips() {
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
            assert_eq!(octets.len(), HEADER + FIXED_LEN + 4 + "Chillers".len());
            assert_eq!(octets[..8], MAGIC);
            assert_eq!(decode(oid(1), &octets).unwrap(), snapshot);
        }
        for snapshot in [
            LoadControlSnapshot::default(),
            LoadControlSnapshot {
                enable: false,
                description: Some(String::new()),
                ..LoadControlSnapshot::default()
            },
        ] {
            let octets = encode(oid(1), &snapshot).unwrap();
            assert_eq!(decode(oid(1), &octets).unwrap(), snapshot);
        }
        assert_eq!(
            encode(oid(1), &LoadControlSnapshot::default())
                .unwrap()
                .len(),
            HEADER + FIXED_LEN
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
        // Cut short in the fixed fields, and inside the Description.
        assert_eq!(kind(&octets[..HEADER + 10]), SnapshotErrorKind::Truncated);
        assert_eq!(
            kind(&octets[..octets.len() - 1]),
            SnapshotErrorKind::Truncated
        );
        // An octet past the end, after a Description and after none.
        let mut longer = octets.clone();
        longer.push(0);
        assert_eq!(kind(&longer), SnapshotErrorKind::Damaged);
        let mut plain = encode(oid(1), &LoadControlSnapshot::default()).unwrap();
        plain.push(0);
        assert_eq!(kind(&plain), SnapshotErrorKind::Damaged);
        let mut huge = octets.clone();
        huge.resize(MAX_LEN + 1, 0);
        assert_eq!(kind(&huge), SnapshotErrorKind::TooLarge);
        // A state past the four, a fourth choice, an hour unspecified
        // alone, an Enable of 2 and a presence octet of 2.
        for (at, octet) in [
            (HEADER + 3, 9),
            (HEADER + 4, 3),
            (HEADER + 17, 0xFF),
            (HEADER + 45, 2),
            (HEADER + 46, 2),
        ] {
            let mut damaged = octets.clone();
            damaged[at] = octet;
            assert_eq!(kind(&damaged), SnapshotErrorKind::Damaged, "octet {at}");
        }
        // A Description that isn't UTF-8.
        let mut text = octets.clone();
        *text.last_mut().unwrap() = 0xFF;
        assert_eq!(kind(&text), SnapshotErrorKind::Damaged);
        // A percent over 100, which a write refuses.
        let mut percent = encode(oid(1), &LoadControlSnapshot::default()).unwrap();
        percent[HEADER + 4] = 0;
        percent[HEADER + 12] = 101;
        assert_eq!(kind(&percent), SnapshotErrorKind::Damaged);
        for refused in [
            LoadControlSnapshot {
                requested_shed_level: BACnetShedLevel::Percent(101),
                ..shedding()
            },
            LoadControlSnapshot {
                requested_shed_level: BACnetShedLevel::Amount(-1.0),
                ..shedding()
            },
            LoadControlSnapshot {
                requested_shed_level: BACnetShedLevel::Amount(f32::NAN),
                ..shedding()
            },
            LoadControlSnapshot {
                description: Some("x".repeat(MAX_DESCRIPTION_OCTETS + 1)),
                ..shedding()
            },
        ] {
            assert_eq!(
                encode(oid(1), &refused).unwrap_err().kind(),
                SnapshotErrorKind::OutOfRange
            );
        }
    }
}
