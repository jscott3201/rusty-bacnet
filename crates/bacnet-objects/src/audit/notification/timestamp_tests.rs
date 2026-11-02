//! The complementary-timestamp window on its own: which timestamps two
//! reports of one operation may carry and still merge (#2183).

use super::*;

fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Time {
    Time {
        hour,
        minute,
        second,
        hundredths,
    }
}

/// `year` is the calendar year; `day_of_week` runs Monday 1 to Sunday 7.
fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> Date {
    Date {
        year: (year - 1900) as u8,
        month,
        day,
        day_of_week,
    }
}

fn at(date: Date, time: Time) -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime { date, time }
}

#[test]
fn times_of_day_compare_round_midnight_and_refuse_invalid_fields() {
    let late = BACnetTimeStamp::Time(time(23, 59, 59, 90));
    let early = BACnetTimeStamp::Time(time(0, 0, 0, 5));
    // 15 hundredths apart across midnight: inside 200 ms, outside 100 ms.
    assert!(timestamps_within(&late, &early, 200));
    assert!(!timestamps_within(&late, &early, 100));
    for invalid in [
        time(24, 0, 0, 0),
        time(0, 60, 0, 0),
        time(0, 0, 60, 0),
        time(0, 0, 0, 100),
    ] {
        assert!(!timestamps_within(
            &BACnetTimeStamp::Time(invalid),
            &early,
            u64::MAX
        ));
    }
}

#[test]
fn dates_compare_across_a_day_and_refuse_impossible_ones() {
    let noon = time(12, 0, 0, 0);
    // Thursday 29 February 2024 and Friday 1 March 2024.
    let leap_day = at(date(2024, 2, 29, 4), noon);
    let next_day = at(date(2024, 3, 1, 5), noon);
    let day_ms = 24 * 60 * 60 * 1000;
    assert!(timestamps_within(&leap_day, &next_day, day_ms));
    assert!(!timestamps_within(&leap_day, &next_day, day_ms - 10));

    let valid = at(date(2024, 3, 1, 5), noon);
    for invalid in [
        date(2023, 2, 29, 3), // 2023 is no leap year
        date(1900, 2, 29, 4), // nor is 1900
        date(2024, 4, 31, 2), // April has 30 days
        date(2024, 3, 0, 5),
        date(2024, 13, 1, 1),
        date(2024, 0, 1, 1),
        date(2024, 3, 1, 0),
        date(2024, 3, 1, 8),
        date(2024, 3, 1, 4), // 1 March 2024 was a Friday
    ] {
        assert!(!timestamps_within(&at(invalid, noon), &valid, u64::MAX));
        assert!(!timestamps_within(&valid, &at(invalid, noon), u64::MAX));
    }
    // An unspecified year has no place on the calendar.
    let mut unspecified = date(2024, 3, 1, 5);
    unspecified.year = 255;
    assert!(!timestamps_within(&at(unspecified, noon), &valid, u64::MAX));
    // A valid date with an impossible time is refused too.
    assert!(!timestamps_within(
        &at(date(2024, 3, 1, 5), time(25, 0, 0, 0)),
        &valid,
        u64::MAX
    ));
    // 2000 is a leap year: Tuesday 29 February 2000.
    assert!(timestamps_within(
        &at(date(2000, 2, 29, 2), noon),
        &at(date(2000, 2, 29, 2), noon),
        0
    ));
}

#[test]
fn month_lengths_follow_the_gregorian_calendar() {
    let lengths = |year| {
        (1..=12)
            .map(|month| days_in_month(year, month))
            .collect::<Vec<_>>()
    };
    let common = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut leap = common;
    leap[1] = 29;
    assert_eq!(lengths(2023), common);
    assert_eq!(lengths(2024), leap);
    assert_eq!(lengths(1900), common);
    assert_eq!(lengths(2000), leap);
    assert_eq!(days_in_month(2024, 0), 0);
    assert_eq!(days_in_month(2024, 13), 0);
}

#[test]
fn only_like_variants_compare_and_sequence_numbers_must_be_equal() {
    let noon = time(12, 0, 0, 0);
    let time_only = BACnetTimeStamp::Time(noon);
    let dated = at(date(2024, 3, 1, 5), noon);
    let sequence = BACnetTimeStamp::SequenceNumber(7);
    assert!(!timestamps_within(&time_only, &dated, u64::MAX));
    assert!(!timestamps_within(&dated, &sequence, u64::MAX));
    assert!(!timestamps_within(&sequence, &time_only, u64::MAX));
    assert!(timestamps_within(&sequence, &sequence, 0));
    assert!(!timestamps_within(
        &sequence,
        &BACnetTimeStamp::SequenceNumber(8),
        u64::MAX
    ));
}

#[test]
fn a_stored_target_report_matches_an_incoming_source_report() {
    let blank = || BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: bacnet_types::constructed::BACnetRecipient::Device(
            ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 1).unwrap(),
        ),
        source_object: None,
        operation: bacnet_types::enums::AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: bacnet_types::constructed::BACnetRecipient::Device(
            ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 2).unwrap(),
        ),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: None,
        current_value: None,
        result: None,
    };
    let mut stored = blank();
    stored.target_timestamp = Some(BACnetTimeStamp::Time(time(12, 0, 0, 0)));
    let mut incoming = blank();
    incoming.source_timestamp = Some(BACnetTimeStamp::Time(time(12, 0, 1, 0)));
    assert!(complementary_timestamps_match(&stored, &incoming, 1_000));
    assert!(!complementary_timestamps_match(&stored, &incoming, 990));
    // Two target reports never complement each other.
    incoming.target_timestamp = incoming.source_timestamp.take();
    assert!(!complementary_timestamps_match(
        &stored,
        &incoming,
        u64::MAX
    ));
}
