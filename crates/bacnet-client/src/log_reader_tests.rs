use super::*;
use bacnet_encoding::constructed::encode_log_record;
use bacnet_services::read_range::ReadRangeAck;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bytes::BytesMut;

const TREND: SequenceSpace = SequenceSpace {
    max: u32::MAX as u64,
};
const AUDIT: SequenceSpace = SequenceSpace { max: u64::MAX };
const TOP: u64 = u32::MAX as u64;

fn trend_log() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

#[test]
fn sequence_numbers_wrap_to_one_and_never_zero() {
    assert_eq!(TREND.advance(1, 10), 11);
    assert_eq!(TREND.advance(TOP - 4, 5), 1);
    assert_eq!(TREND.advance(TOP - 4, 10), 6);
    assert_eq!(TREND.advance(TOP, 1), 1);
    assert_eq!(AUDIT.advance(u64::MAX, 2), 2);
    assert_eq!(AUDIT.advance(u64::MAX - 1, 1), u64::MAX);
    // A device that numbered the record after its wrap 0 counts on from it.
    assert_eq!(TREND.advance(0, 10), 10);
}

#[test]
fn the_window_runs_from_total_less_count_plus_one_to_total() {
    assert_eq!(TREND.window(0, 0).unwrap(), Window::Empty { next: 1 });
    assert_eq!(TREND.window(0, 41).unwrap(), Window::Empty { next: 42 });
    assert_eq!(TREND.window(0, TOP).unwrap(), Window::Empty { next: 1 });
    assert_eq!(
        TREND.window(10, 100).unwrap(),
        Window::Records {
            oldest: 91,
            newest: 100
        }
    );
    // Across the wrap: three records numbered after it, seven before.
    assert_eq!(
        TREND.window(10, 3).unwrap(),
        Window::Records {
            oldest: TOP - 6,
            newest: 3
        }
    );
    // A device that counts through 0 reads a Total_Record_Count of 0.
    assert_eq!(
        TREND.window(10, 0).unwrap(),
        Window::Records {
            oldest: TOP - 8,
            newest: 0
        }
    );
    assert_eq!(
        AUDIT.window(3, 1).unwrap(),
        Window::Records {
            oldest: u64::MAX - 1,
            newest: 1
        }
    );
    assert!(TREND.window(1, TOP + 1).is_err());
    assert!(TREND.window(TOP + 1, 1).is_err());
}

#[test]
fn distance_is_signed_across_the_wrap() {
    assert_eq!(TREND.distance(5, 9), 4);
    assert_eq!(TREND.distance(9, 5), -4);
    assert_eq!(TREND.distance(TOP - 1, 2), 3);
    assert_eq!(TREND.distance(2, TOP - 1), -3);
    assert_eq!(TREND.distance(1, 0), 0);
    assert_eq!(TREND.distance(5, 0), -4);
    assert_eq!(AUDIT.distance(u64::MAX, 1), 1);
}

#[test]
fn page_size_must_be_a_positive_integer16() {
    assert_eq!(page_count(1).unwrap(), 1);
    assert_eq!(page_count(32_767).unwrap(), 32_767);
    assert!(page_count(0).is_err());
    assert!(page_count(32_768).is_err());
}

#[test]
fn only_log_objects_have_a_sequence_space() {
    assert_eq!(SequenceSpace::of(ObjectType::AUDIT_LOG).unwrap(), AUDIT);
    assert_eq!(SequenceSpace::of(ObjectType::EVENT_LOG).unwrap(), TREND);
    assert!(SequenceSpace::of(ObjectType::ANALOG_INPUT).is_err());
}

fn reply(
    count: u32,
    first: Option<u64>,
    flags: (bool, bool, bool),
    violations: Vec<ReadRangeViolation>,
) -> ReadRangeReply {
    let mut item_data = BytesMut::new();
    for value in 0..u64::from(count) {
        let record = BACnetLogRecord {
            date: Date {
                year: 126,
                month: 10,
                day: 5,
                day_of_week: 1,
            },
            time: Time {
                hour: 9,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            log_datum: LogDatum::UnsignedValue(value),
            status_flags: None,
        };
        encode_log_record(&record, &mut item_data).unwrap();
    }
    ReadRangeReply {
        ack: ReadRangeAck {
            object_identifier: trend_log(),
            property_identifier: PropertyIdentifier::LOG_BUFFER,
            property_array_index: None,
            result_flags: flags,
            item_count: count,
            item_data: item_data.to_vec(),
            first_sequence_number: first,
        },
        violations,
    }
}

#[test]
fn a_page_advances_by_its_first_sequence_number_plus_its_records() {
    let page = page(
        TREND,
        LogCursor::Sequence(TOP - 2),
        reply(5, Some(TOP - 2), (false, false, true), vec![]),
        None,
    )
    .unwrap();
    assert_eq!(page.records.len(), 5);
    assert_eq!(page.first_sequence_number, Some(TOP - 2));
    assert_eq!(page.next, LogCursor::Sequence(3));
    assert!(!page.done);
    assert_eq!(page.gap, None);
}

#[test]
fn more_items_or_a_full_page_continues_last_item_or_an_empty_page_ends() {
    let at = LogCursor::Sequence(1);
    for (flags, done) in [
        ((true, false, true), false),
        // MORE_ITEMS only reports a cut to fit; LAST_ITEM clear means the
        // newest record is still ahead.
        ((true, false, false), false),
        ((true, true, false), true),
        // Contradictory, but LAST_ITEM still ends the read.
        ((true, true, true), true),
    ] {
        let page = page(TREND, at, reply(2, Some(1), flags, vec![]), None).unwrap();
        assert_eq!(page.done, done, "{flags:?}");
    }
    let empty = page(
        TREND,
        at,
        reply(0, None, (false, false, false), vec![]),
        None,
    )
    .unwrap();
    assert!(empty.done);
    assert_eq!(empty.next, at);
}

#[test]
fn a_first_record_past_the_one_asked_for_is_a_gap() {
    let page = page(
        TREND,
        LogCursor::Sequence(TOP - 1),
        reply(1, Some(3), (false, true, false), vec![]),
        None,
    )
    .unwrap();
    assert_eq!(
        page.gap,
        Some(LogGap {
            expected: TOP - 1,
            first: 3,
            skipped: Some(4),
        })
    );
    assert_eq!(page.next, LogCursor::Sequence(4));
}

#[test]
fn a_first_record_before_the_one_asked_for_is_not_advancing() {
    let error = page(
        TREND,
        LogCursor::Sequence(4),
        reply(10, Some(TOP - 5), (true, false, true), vec![]),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            Error::LogNotAdvancing {
                requested: 4,
                returned: Some(returned)
            } if returned == TOP - 5
        ),
        "{error:?}"
    );
}

#[test]
fn a_restarted_read_reports_the_gap_from_the_checkpoint() {
    let lost = page(
        TREND,
        LogCursor::Sequence(50),
        reply(2, Some(50), (true, true, false), vec![]),
        Some(20),
    )
    .unwrap();
    assert_eq!(
        lost.gap,
        Some(LogGap {
            expected: 20,
            first: 50,
            skipped: Some(30),
        })
    );
    // A checkpoint ahead of the log's numbering, as after a reset.
    let reset = page(
        TREND,
        LogCursor::Sequence(1),
        reply(2, Some(1), (true, true, false), vec![]),
        Some(900),
    )
    .unwrap();
    assert_eq!(reset.gap.unwrap().skipped, None);
}

#[test]
fn tolerated_rules_are_listed_and_others_fail() {
    let wrapped = page(
        TREND,
        LogCursor::Sequence(1),
        reply(
            3,
            Some(0),
            (false, false, true),
            vec![ReadRangeViolation::ZeroFirstSequenceNumber],
        ),
        None,
    )
    .unwrap();
    assert_eq!(
        wrapped.violations,
        [ReadRangeViolation::ZeroFirstSequenceNumber]
    );
    assert_eq!(wrapped.gap, None);
    assert_eq!(wrapped.next, LogCursor::Sequence(3));

    for fatal in [
        ReadRangeViolation::ObjectMismatch,
        ReadRangeViolation::PropertyMismatch,
        ReadRangeViolation::ArrayIndexMismatch,
        ReadRangeViolation::MissingFirstSequenceNumber,
    ] {
        let error = page(
            TREND,
            LogCursor::Sequence(1),
            reply(1, None, (true, true, false), vec![fatal]),
            None,
        )
        .unwrap_err();
        assert!(
            matches!(error, Error::ReadRangeViolation(rule) if rule == fatal),
            "{error:?}"
        );
    }
}

#[test]
fn by_position_advances_by_position_and_ignores_sequence_numbers() {
    let page = page(
        TREND,
        LogCursor::Position(11),
        reply(
            4,
            Some(7),
            (false, false, true),
            vec![ReadRangeViolation::UnexpectedFirstSequenceNumber],
        ),
        None,
    )
    .unwrap();
    assert_eq!(page.next, LogCursor::Position(15));
    assert_eq!(page.first_sequence_number, None);
    assert!(!page.done);
}

#[test]
fn by_time_goes_on_by_sequence_number() {
    let at = LogCursor::Time(
        Date {
            year: 126,
            month: 10,
            day: 5,
            day_of_week: 1,
        },
        Time {
            hour: 8,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
    );
    let page = page(
        TREND,
        at,
        reply(3, Some(40), (false, false, false), vec![]),
        None,
    )
    .unwrap();
    assert_eq!(page.next, LogCursor::Sequence(43));
    assert_eq!(page.gap, None);
    let nothing_newer = super::page(
        TREND,
        at,
        reply(0, None, (false, false, false), vec![]),
        None,
    )
    .unwrap();
    assert_eq!(nothing_newer.next, at);
    assert!(nothing_newer.done);
}

/// A requester whose ReadRange answers are canned, with counts of 10 and
/// 10: records 1 to 10.
struct Canned(std::sync::Mutex<Vec<ReadRangeReply>>);

impl LogRequester for Canned {
    async fn read_unsigned(
        &self,
        _mac: &[u8],
        _log: ObjectIdentifier,
        _property: PropertyIdentifier,
    ) -> Result<u64, Error> {
        Ok(10)
    }

    async fn read_range_lenient(
        &self,
        _mac: &[u8],
        _request: &ReadRangeRequest,
    ) -> Result<ReadRangeReply, Error> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

/// An empty page that breaks a rule the reader can't tolerate fails, even
/// though an empty page otherwise only sends the reader to the counts.
#[tokio::test]
async fn an_empty_page_that_breaks_an_echo_rule_fails() {
    let mut empty = reply(0, None, (false, false, false), vec![]);
    empty.ack.object_identifier = ObjectIdentifier::new(ObjectType::TREND_LOG, 9).unwrap();
    empty.violations = vec![ReadRangeViolation::ObjectMismatch];
    let requester = Canned(std::sync::Mutex::new(vec![empty]));
    let error = read_log_page(&requester, &[1], trend_log(), LogCursor::Sequence(3), 5)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::ReadRangeViolation(ReadRangeViolation::ObjectMismatch)
        ),
        "{error:?}"
    );
}
