//! A Load Control's shed request (`RBNLCT01`, and `RBNLCT02` with written
//! Shed_Levels elements), as `FileLoadControlPersistence` (with std) writes
//! its file (#2479, #2502).
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
//! Version 2 ([`MAGIC_WITH_SHED_LEVELS`]) goes on with the Shed_Levels
//! elements a write set (#2502): their count in four octets, then for each
//! its array index in four octets and its level in eight. [`encode`] writes
//! it only for a snapshot holding one, so a snapshot without keeps version
//! 1's octets, and a build that reads only version 1 refuses version 2 as
//! an unknown version.
//!
//! The body ends where its last field does. A state outside the four shed
//! states, a choice past 2, a percent over 100, an amount that is negative
//! or not finite, a Start_Time that is neither a moment nor wholly
//! unspecified, an Enable or presence octet other than 0 or 1, octets past
//! the end, a Description past its cap or not UTF-8, and in version 2 no
//! written level, more than [`MAX_WRITTEN_SHED_LEVELS`], an index of 0 or
//! one not past the one before, or a level of 0, are what no save writes, so
//! they are damage.

use crate::prelude::*;
use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::ShedState;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::SnapshotError;
use super::framing;
use crate::clock::{UNSPECIFIED_DATETIME, local_moment};
use crate::load_control::{LoadControlSnapshot, MAX_WRITTEN_SHED_LEVELS};
use crate::write_config::description::MAX_OCTETS as MAX_DESCRIPTION_OCTETS;

/// The format's tag, version 1.
pub const MAGIC: [u8; 8] = *b"RBNLCT01";
/// The format's tag, version 2, for a snapshot with written Shed_Levels
/// elements (#2502).
pub const MAGIC_WITH_SHED_LEVELS: [u8; 8] = *b"RBNLCT02";
/// The fixed fields' length: the state, the level, Start_Time, the three
/// durations, Enable and the Description's presence octet.
const FIXED_LEN: usize = 4 + 9 + 8 + 3 * 8 + 2;
/// One written Shed_Levels element: its index and its level.
const LEVEL_LEN: usize = 4 + 8;
/// The most octets a file may take: the header, the fixed fields, the
/// longest Description and the most written levels.
pub const MAX_LEN: usize = framing::HEADER_LEN
    + FIXED_LEN
    + 4
    + MAX_DESCRIPTION_OCTETS
    + 4
    + MAX_WRITTEN_SHED_LEVELS * LEVEL_LEN;

/// The file octets for `snapshot`, saved for the Load Control `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) what
/// [`decode`] would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &LoadControlSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_body(snapshot)
        .map(|body| {
            let magic = if snapshot.shed_levels.is_empty() {
                &MAGIC
            } else {
                &MAGIC_WITH_SHED_LEVELS
            };
            framing::seal(magic, object, &body)
        })
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
    if !snapshot.shed_levels.is_empty() {
        let count = u32::try_from(snapshot.shed_levels.len()).expect("checked by `sound`");
        body.extend_from_slice(&count.to_be_bytes());
        for &(index, level) in &snapshot.shed_levels {
            body.extend_from_slice(&index.to_be_bytes());
            body.extend_from_slice(&level.to_be_bytes());
        }
    }
    Ok(body)
}

/// The snapshot a file's `octets` hold for the Load Control `object`.
///
/// Reads both versions. Refuses octets past [`MAX_LEN`], without either
/// tag, saved for another object, cut short, going on past the body's end,
/// and a value no save writes.
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
    let (version_1, body) = framing::open(
        octets,
        &MAGIC_WITH_SHED_LEVELS,
        Some(&MAGIC),
        MAX_LEN,
        object,
    )?;
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
    let (shed_levels, rest) = if version_1 {
        (Vec::new(), rest)
    } else {
        take_levels(rest)?
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
        shed_levels,
    };
    if !sound(&snapshot) {
        return Err(SnapshotError::damaged("holds a value no write leaves"));
    }
    Ok(snapshot)
}

/// Written Shed_Levels elements: (array index, level) pairs.
type Levels = Vec<(u32, u64)>;

/// The written levels version 2 holds at the start of `rest`, and what
/// follows them. [`sound`] checks them.
fn take_levels(rest: &[u8]) -> Result<(Levels, &[u8]), SnapshotError> {
    let Some((count, rest)) = rest.split_first_chunk::<4>() else {
        return Err(SnapshotError::truncated("ends before its written levels"));
    };
    let count = usize::try_from(u32::from_be_bytes(*count))
        .ok()
        .filter(|count| (1..=MAX_WRITTEN_SHED_LEVELS).contains(count))
        .ok_or(SnapshotError::damaged(
            "holds no written levels or too many",
        ))?;
    let Some((levels, rest)) = rest.split_at_checked(count * LEVEL_LEN) else {
        return Err(SnapshotError::truncated("ends inside its written levels"));
    };
    let levels = levels
        .as_chunks::<LEVEL_LEN>()
        .0
        .iter()
        .map(|entry| {
            let (index, level) = entry.split_at(4);
            (
                u32::from_be_bytes(index.try_into().expect("four octets")),
                u64::from_be_bytes(level.try_into().expect("eight octets")),
            )
        })
        .collect();
    Ok((levels, rest))
}

/// Whether `snapshot` holds only what the object serves: a shed state, a
/// level a write takes, a Start_Time that names a moment or none, and
/// written levels a write leaves. A Description past its cap is refused as
/// it is written.
fn sound(snapshot: &LoadControlSnapshot) -> bool {
    let state = snapshot.state.to_raw() <= ShedState::SHED_NON_COMPLIANT.to_raw();
    let level = match snapshot.requested_shed_level {
        BACnetShedLevel::Percent(percent) => percent <= 100,
        BACnetShedLevel::Level(_) => true,
        BACnetShedLevel::Amount(amount) => amount.is_finite() && amount >= 0.0,
    };
    let start =
        snapshot.start_time == UNSPECIFIED_DATETIME || local_moment(snapshot.start_time).is_some();
    state && level && start && crate::load_control::written_levels_sound(&snapshot.shed_levels)
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
            shed_levels: Vec::new(),
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

    /// [`shedding`] with two written Shed_Levels elements.
    fn with_levels() -> LoadControlSnapshot {
        LoadControlSnapshot {
            shed_levels: vec![(1, 3), (4, u64::MAX)],
            ..shedding()
        }
    }

    #[test]
    fn written_levels_take_version_2_and_a_file_without_keeps_version_1() {
        let octets = encode(oid(1), &with_levels()).unwrap();
        assert_eq!(octets[..8], MAGIC_WITH_SHED_LEVELS);
        let levels_at = HEADER + FIXED_LEN + 4 + "Chillers".len();
        assert_eq!(octets.len(), levels_at + 4 + 2 * LEVEL_LEN);
        assert_eq!(octets[levels_at..levels_at + 4], [0, 0, 0, 2]);
        assert_eq!(decode(oid(1), &octets).unwrap(), with_levels());
        // Without a Description too.
        let plain = LoadControlSnapshot {
            description: None,
            ..with_levels()
        };
        assert_eq!(
            decode(oid(1), &encode(oid(1), &plain).unwrap()).unwrap(),
            plain
        );
        // No written level: version 1's octets, which still load.
        let old = encode(oid(1), &shedding()).unwrap();
        assert_eq!(old[..8], MAGIC);
        assert_eq!(decode(oid(1), &old).unwrap(), shedding());
        // A later version is refused as one, not as damage.
        let mut later = octets.clone();
        later[..8].copy_from_slice(b"RBNLCT03");
        assert_eq!(
            decode(oid(1), &later).unwrap_err().kind(),
            SnapshotErrorKind::UnknownVersion(3)
        );
    }

    #[test]
    fn written_levels_no_save_writes_are_refused() {
        let octets = encode(oid(1), &with_levels()).unwrap();
        let kind = |octets: &[u8]| decode(oid(1), octets).unwrap_err().kind();
        let levels_at = octets.len() - 4 - 2 * LEVEL_LEN;
        // Cut short in the count and in the last element; an octet past
        // the end; version 1's tag on a body with levels.
        assert_eq!(kind(&octets[..levels_at + 2]), SnapshotErrorKind::Truncated);
        assert_eq!(
            kind(&octets[..octets.len() - 1]),
            SnapshotErrorKind::Truncated
        );
        let mut longer = octets.clone();
        longer.push(0);
        assert_eq!(kind(&longer), SnapshotErrorKind::Damaged);
        let mut v1 = octets.clone();
        v1[..8].copy_from_slice(&MAGIC);
        assert_eq!(kind(&v1), SnapshotErrorKind::Damaged);
        // A count of 0 or past the cap, an index of 0, an index not past
        // the one before, and a level of 0.
        let first = levels_at + 4;
        let second = first + LEVEL_LEN;
        let past_cap = (MAX_WRITTEN_SHED_LEVELS as u32 + 1).to_be_bytes();
        for (at, written) in [
            (levels_at, &[0u8, 0, 0, 0][..]),
            (levels_at, &past_cap[..]),
            (first, &[0, 0, 0, 0][..]),
            (second, &[0, 0, 0, 1][..]),
            (second + 4, &[0; 8][..]),
        ] {
            let mut damaged = octets.clone();
            damaged[at..at + written.len()].copy_from_slice(written);
            assert_eq!(kind(&damaged), SnapshotErrorKind::Damaged, "octet {at}");
        }
        for refused in [
            vec![(0, 1)],
            vec![(1, 0)],
            vec![(2, 1), (1, 1)],
            vec![(1, 1), (1, 2)],
            (1..=MAX_WRITTEN_SHED_LEVELS as u32 + 1)
                .map(|index| (index, 1))
                .collect(),
        ] {
            let snapshot = LoadControlSnapshot {
                shed_levels: refused,
                ..shedding()
            };
            assert_eq!(
                encode(oid(1), &snapshot).unwrap_err().kind(),
                SnapshotErrorKind::OutOfRange
            );
        }
        // The cap itself saves, and the file fits.
        let most = LoadControlSnapshot {
            description: Some("x".repeat(MAX_DESCRIPTION_OCTETS)),
            shed_levels: (1..=MAX_WRITTEN_SHED_LEVELS as u32)
                .map(|index| (index, 1))
                .collect(),
            ..shedding()
        };
        let octets = encode(oid(1), &most).unwrap();
        assert_eq!(octets.len(), MAX_LEN);
        assert_eq!(decode(oid(1), &octets).unwrap(), most);
    }
}
