//! Schedule codecs (Clause 21 `BACnetTimeValue`, `BACnetDailySchedule` and
//! `BACnetSpecialEvent`): golden vectors, round trips and rejections. The
//! vectors follow the tag rules summarised in `tests/calendar.rs`.

use super::*;
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDateRange, BACnetSpecialEvent, BACnetTimeValue, BACnetWeekNDay,
    SpecialEventPeriod,
};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, PropertyValue, Time};

fn d(year: u8, month: u8, day: u8, day_of_week: u8) -> Date {
    Date {
        year,
        month,
        day,
        day_of_week,
    }
}

fn t(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn tv(hour: u8, minute: u8, value: PropertyValue) -> BACnetTimeValue {
    BACnetTimeValue {
        time: t(hour, minute),
        value,
    }
}

fn tv_real(hour: u8, minute: u8, value: f32) -> BACnetTimeValue {
    tv(hour, minute, PropertyValue::Real(value))
}

fn unsigned_42() -> PropertyValue {
    PropertyValue::Unsigned(42)
}

fn calendar(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CALENDAR, instance).unwrap()
}

fn encode_event(event: &BACnetSpecialEvent) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_special_event(&mut buf, event).unwrap();
    buf.to_vec()
}

/// 08:30 -> application Unsigned 42: Time is application tag 11 with length
/// 4 (0xB4); Unsigned is application tag 2 with length 1 (0x21).
const TV_0830_42: &[u8] = &[0xB4, 8, 30, 0, 0, 0x21, 42];

// --- BACnetTimeValue --------------------------------------------------------

/// One time-value per primitive datatype, each value under its own
/// application tag (Clause 20.2.1.4): the time-value carries the scheduled
/// value typed (#1028), never as an Octet String of its encoding.
#[test]
fn time_value_golden_vectors_carry_every_primitive_datatype() {
    let at = |value: &[u8]| [&[0xB4, 6, 0, 0, 0][..], value].concat();
    for (time_value, wire) in [
        (tv(8, 30, unsigned_42()), TV_0830_42.to_vec()),
        // Null (0x00) and Boolean TRUE (0x11) carry no content octets.
        (tv(6, 0, PropertyValue::Null), at(&[0x00])),
        (tv(6, 0, PropertyValue::Boolean(true)), at(&[0x11])),
        (tv(6, 0, PropertyValue::Signed(-2)), at(&[0x31, 0xFE])),
        (tv_real(6, 0, 65.0), at(&[0x44, 0x42, 0x82, 0, 0])),
        (
            tv(6, 0, PropertyValue::Double(1.0)),
            at(&[0x55, 8, 0x3F, 0xF0, 0, 0, 0, 0, 0, 0]),
        ),
        (
            tv(6, 0, PropertyValue::OctetString(vec![0xAB])),
            at(&[0x61, 0xAB]),
        ),
        (
            tv(6, 0, PropertyValue::CharacterString("on".into())),
            at(&[0x73, 0, b'o', b'n']),
        ),
        (
            tv(
                6,
                0,
                PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![0xA0],
                },
            ),
            at(&[0x82, 4, 0xA0]),
        ),
        (tv(6, 0, PropertyValue::Enumerated(3)), at(&[0x91, 3])),
        (
            tv(6, 0, PropertyValue::Date(d(126, 12, 25, 5))),
            at(&[0xA4, 126, 12, 25, 5]),
        ),
        (
            tv(6, 0, PropertyValue::Time(t(17, 30))),
            at(&[0xB4, 17, 30, 0, 0]),
        ),
        (
            tv(6, 0, PropertyValue::ObjectIdentifier(calendar(7))),
            at(&[0xC4, 0x01, 0x80, 0x00, 0x07]),
        ),
    ] {
        let mut buf = BytesMut::new();
        encode_time_value(&mut buf, &time_value).unwrap();
        assert_eq!(&buf[..], &wire[..], "{time_value:?}");
        assert_eq!(
            decode_time_value(&wire, 0).unwrap(),
            (time_value, wire.len())
        );
    }
}

#[test]
fn time_value_refuses_to_encode_a_constructed_value() {
    for value in [
        PropertyValue::List(vec![PropertyValue::Unsigned(1)]),
        PropertyValue::ApplicationData(vec![0x09, 1]),
    ] {
        let mut buf = BytesMut::new();
        let err = encode_time_value(&mut buf, &tv(6, 0, value)).unwrap_err();
        assert!(format!("{err}").contains("primitive"), "{err}");
        assert!(buf.is_empty());
    }
}

#[test]
fn time_value_rejects_a_context_tagged_value_and_a_missing_time() {
    let mut buf = BytesMut::new();
    primitives::encode_app_time(&mut buf, &t(8, 0));
    primitives::encode_ctx_unsigned(&mut buf, 0, 1);
    let err = decode_time_value(&buf, 0).unwrap_err();
    assert!(format!("{err}").contains("expected an application-tagged value"));
    // A context-tagged time, and a value with no time before it.
    assert!(decode_time_value(&[0x0C, 8, 0, 0, 0, 0x21, 1], 0).is_err());
    assert!(decode_time_value(&[0x21, 42], 0).is_err());
    for cut in 1..TV_0830_42.len() {
        assert!(decode_time_value(&TV_0830_42[..cut], 0).is_err(), "{cut}");
    }
}

// --- BACnetSpecialEvent -----------------------------------------------------

/// Special-event golden vectors: the period, then the `[2]` frame of
/// time-values (0x2E ... 0x2F), then event-priority `[3]` as a one-octet
/// context primitive (0x39).
fn special_event_vectors() -> Vec<(BACnetSpecialEvent, Vec<u8>)> {
    vec![
        // Inline weekNDay entry: the calendar-entry `[0]` frame (0x0E ...
        // 0x0F) wraps the entry's own weekNDay `[2]` tag (0x2B).
        (
            BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                    BACnetWeekNDay {
                        month: 11,
                        week_of_month: 4,
                        day_of_week: 4,
                    },
                )),
                list_of_time_values: vec![tv(8, 30, unsigned_42())],
                event_priority: 3,
            },
            [
                &[0x0E, 0x2B, 11, 4, 4, 0x0F, 0x2E][..],
                TV_0830_42,
                &[0x2F, 0x39, 3],
            ]
            .concat(),
        ),
        // Inline date entry for Friday 25 December 2026, no time-values.
        (
            BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(d(
                    126, 12, 25, 5,
                ))),
                list_of_time_values: vec![],
                event_priority: 1,
            },
            vec![0x0E, 0x0C, 126, 12, 25, 5, 0x0F, 0x2E, 0x2F, 0x39, 1],
        ),
        // Inline date-range entry: the `[1]` frame nests inside the `[0]` one.
        // Wednesday 1 July to Monday 31 August 2026, Null at midnight.
        (
            BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::DateRange(
                    BACnetDateRange {
                        start_date: d(126, 7, 1, 3),
                        end_date: d(126, 8, 31, 1),
                    },
                )),
                list_of_time_values: vec![tv(0, 0, PropertyValue::Null)],
                event_priority: 16,
            },
            vec![
                0x0E, 0x1E, 0xA4, 126, 7, 1, 3, 0xA4, 126, 8, 31, 1, 0x1F, 0x0F, 0x2E, 0xB4, 0, 0,
                0, 0, 0x00, 0x2F, 0x39, 16,
            ],
        ),
        // calendar-reference `[1]` is a primitive four-octet object
        // identifier (0x1C): Calendar is type 6, so instance 7 is 0x01800007.
        (
            BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarReference(calendar(7)),
                list_of_time_values: vec![tv(8, 30, unsigned_42())],
                event_priority: 16,
            },
            [
                &[0x1C, 0x01, 0x80, 0x00, 0x07, 0x2E][..],
                TV_0830_42,
                &[0x2F, 0x39, 16],
            ]
            .concat(),
        ),
    ]
}

#[test]
fn special_event_golden_vectors_for_each_period() {
    for (event, wire) in special_event_vectors() {
        assert_eq!(encode_event(&event), wire, "{event:?}");
        assert_eq!(decode_special_event(&wire, 0).unwrap(), (event, wire.len()));
    }
}

#[test]
fn special_event_round_trips_several_time_values() {
    let event = BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarReference(calendar(1)),
        list_of_time_values: vec![
            tv_real(8, 0, 70.0),
            tv(12, 0, PropertyValue::Null),
            tv_real(18, 0, 60.0),
        ],
        event_priority: 8,
    };
    let wire = encode_event(&event);
    assert_eq!(decode_special_event(&wire, 0).unwrap(), (event, wire.len()));
}

#[test]
fn special_event_rejects_malformed_periods() {
    let tail: &[u8] = &[0x2E, 0x2F, 0x39, 1];
    for (what, period) in [
        // The calendar entry inside `[0]` must use its own CHOICE tags.
        (
            "application Date in the [0] frame",
            &[0x0E, 0xA4, 126, 12, 25, 5, 0x0F][..],
        ),
        ("unframed date entry", &[0x0C, 126, 12, 25, 5]),
        ("[0] frame never closed", &[0x0E, 0x0C, 126, 12, 25, 5]),
        (
            "[0] frame closed by [1]",
            &[0x0E, 0x0C, 126, 12, 25, 5, 0x1F],
        ),
        ("empty [0] frame", &[0x0E, 0x0F]),
        (
            "calendar-reference with three octets",
            &[0x1B, 0x01, 0x80, 0x00],
        ),
        (
            "calendar-reference as an opening tag",
            &[0x1E, 0xC4, 0x01, 0x80, 0, 7, 0x1F],
        ),
        ("unknown period [4]", &[0x4C, 0x01, 0x80, 0x00, 0x07]),
    ] {
        let wire = [period, tail].concat();
        assert!(decode_special_event(&wire, 0).is_err(), "{what} decoded");
    }
}

#[test]
fn special_event_decodes_any_unsigned_priority_for_the_consumer_to_check() {
    // The range is the consumer's to check (#1087): out-of-range priorities,
    // up to the widest Unsigned, decode and encode back unchanged.
    let period: &[u8] = &[0x1C, 0x01, 0x80, 0x00, 0x01];
    for (priority, encoded) in [
        (0, &[0x39, 0][..]),
        (17, &[0x39, 17]),
        (255, &[0x39, 0xFF]),
        (300, &[0x3A, 0x01, 0x2C]),
        (
            u64::MAX,
            &[0x3D, 8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        ),
    ] {
        let wire = [period, &[0x2E, 0x2F], encoded].concat();
        let event = BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarReference(calendar(1)),
            list_of_time_values: vec![],
            event_priority: priority,
        };
        assert_eq!(
            decode_special_event(&wire, 0).unwrap(),
            (event.clone(), wire.len()),
            "priority {priority}"
        );
        assert_eq!(encode_event(&event), wire, "priority {priority}");
    }
}

#[test]
fn special_event_rejects_malformed_priorities_and_missing_members() {
    let period: &[u8] = &[0x1C, 0x01, 0x80, 0x00, 0x01];
    for (what, rest) in [
        ("no priority", &[0x2E, 0x2F][..]),
        ("application-tagged priority", &[0x2E, 0x2F, 0x21, 3]),
        // No Unsigned is wider than eight octets.
        (
            "nine-octet priority",
            &[0x2E, 0x2F, 0x3D, 9, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        ),
        ("empty priority", &[0x2E, 0x2F, 0x38]),
        ("no time-value frame", &[0x39, 3]),
        (
            "time-value frame never closed",
            &[0x2E, 0xB4, 8, 30, 0, 0, 0x21, 42, 0x39, 3],
        ),
        ("time-value frame closed by [0]", &[0x2E, 0x0F, 0x39, 3]),
    ] {
        let wire = [period, rest].concat();
        decode_special_event(&wire, 0).expect_err(what);
    }
}

#[test]
fn special_event_rejects_truncated_data() {
    for (event, wire) in special_event_vectors() {
        for cut in 0..wire.len() {
            assert!(
                decode_special_event(&wire[..cut], 0).is_err(),
                "{event:?} cut to {cut} octets decoded"
            );
        }
    }
}

// --- Weekly_Schedule ---------------------------------------------------------

#[test]
fn daily_schedule_golden_vector_is_a_zero_frame() {
    let mut buf = BytesMut::new();
    encode_daily_schedule(&mut buf, &[tv(8, 30, unsigned_42())]).unwrap();
    let wire = [&[0x0E][..], TV_0830_42, &[0x0F]].concat();
    assert_eq!(&buf[..], &wire[..]);
    assert_eq!(
        decode_daily_schedule(&wire, 0).unwrap(),
        (vec![tv(8, 30, unsigned_42())], wire.len())
    );
    for bad in [
        &[0x0E][..],
        &[0x1E, 0x1F],
        &[0x0E, 0x21, 42, 0x0F],
        &[0x0E, 0x1F],
    ] {
        assert!(decode_daily_schedule(bad, 0).is_err(), "{bad:02x?}");
    }
}

#[test]
fn weekly_schedule_golden_vector_is_seven_daily_schedules() {
    let mut days: [Vec<BACnetTimeValue>; 7] = Default::default();
    days[0] = vec![tv(8, 30, unsigned_42())];
    let mut buf = BytesMut::new();
    encode_weekly_schedule(&mut buf, &days).unwrap();
    let mut wire = [&[0x0E][..], TV_0830_42, &[0x0F]].concat();
    for _ in 1..7 {
        wire.extend_from_slice(&[0x0E, 0x0F]);
    }
    assert_eq!(&buf[..], &wire[..]);
    assert_eq!(decode_weekly_schedule(&wire).unwrap(), days);
}

#[test]
fn weekly_schedule_round_trips_and_rejects_a_wrong_day_count() {
    let mut days: [Vec<BACnetTimeValue>; 7] = Default::default();
    days[0] = vec![tv_real(6, 0, 70.0), tv_real(22, 0, 65.0)];
    days[2] = vec![tv_real(8, 0, 72.0)];
    let mut buf = BytesMut::new();
    encode_weekly_schedule(&mut buf, &days).unwrap();
    assert_eq!(decode_weekly_schedule(&buf).unwrap(), days);

    let six = [0x0E, 0x0F].repeat(6);
    let err = decode_weekly_schedule(&six).unwrap_err();
    assert!(
        format!("{err}").contains("weekly schedule day[6]: "),
        "{err}"
    );
    let mut trailing = [0x0E, 0x0F].repeat(7);
    trailing.extend_from_slice(&[0xAA, 0xBB]);
    let err = decode_weekly_schedule(&trailing).unwrap_err();
    assert!(format!("{err}").contains("trailing byte"), "{err}");
}

// --- Exception_Schedule ------------------------------------------------------

#[test]
fn exception_schedule_is_the_concatenation_of_its_events() {
    assert!(decode_exception_schedule(&[]).unwrap().is_empty());
    let (events, wires): (Vec<_>, Vec<_>) = special_event_vectors().into_iter().unzip();
    let wire = wires.concat();
    let mut buf = BytesMut::new();
    encode_exception_schedule(&mut buf, &events).unwrap();
    assert_eq!(&buf[..], &wire[..]);
    assert_eq!(decode_exception_schedule(&wire).unwrap(), events);
    // A truncated last event fails the whole property.
    assert!(decode_exception_schedule(&wire[..wire.len() - 1]).is_err());
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    let mut framed = 0;
    for (_, wire) in special_event_vectors() {
        framed += super::assert_members_cut_short("BACnetSpecialEvent", &wire, |data| {
            decode_special_event(data, 0)
        });
    }
    assert!(framed > 0);
}
