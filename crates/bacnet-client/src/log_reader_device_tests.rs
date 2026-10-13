//! Paged log reads end to end, against a scripted Trend Log (#1530).
use std::sync::{Arc, Mutex as StdMutex};

use bacnet_encoding::constructed::encode_log_record;
use bacnet_encoding::primitives::encode_app_unsigned;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::read_range::{LogRecords, ReadRangeAck};
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bytes::BytesMut;

use super::*;
use crate::client::fake_device::{client_with_device, Answer, FakeDevice, DEVICE_MAC};

const TOP: u64 = u32::MAX as u64;

fn trend_log() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

/// The time of the `index`-th record ever logged: one a second from 09:00.
fn time_of(index: u64) -> Time {
    Time {
        hour: 9 + (index / 3600) as u8,
        minute: ((index / 60) % 60) as u8,
        second: (index % 60) as u8,
        hundredths: 0,
    }
}

const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 5,
    day_of_week: 1,
};

/// A record that carries its own sequence number as its value.
fn record(sequence: u64, index: u64) -> BACnetLogRecord {
    BACnetLogRecord {
        date: DATE,
        time: time_of(index),
        log_datum: LogDatum::UnsignedValue(sequence),
        status_flags: None,
    }
}

fn time_key(time: Time) -> (u8, u8, u8, u8) {
    (time.hour, time.minute, time.second, time.hundredths)
}

/// A Trend Log whose records answer ReadRange the way a device does.
struct FakeLog {
    /// Resident records, oldest first: sequence number, then the record.
    records: Vec<(u64, BACnetLogRecord)>,
    total: u64,
    /// The log's capacity; the oldest record goes when a new one won't fit.
    capacity: usize,
    /// Most records one answer holds, as an APDU size would cut it.
    answer_cap: usize,
    logged: u64,
    /// bacnet-stack 1.6.1 compares a sequence reference with its oldest
    /// and newest numbers as plain integers. Once its records span the wrap
    /// the oldest number is above the newest, so every reference, before the
    /// wrap or after it, reads from the oldest record. Before the wrap it
    /// reads like any other log.
    clamp_across_wrap: bool,
    /// Numbers the record after the top of the range 0, not 1.
    zero_after_wrap: bool,
    /// Records logged just after the next Record_Count read is answered, as
    /// a log that grows between two requests does.
    log_on_count_read: u64,
    /// Records logged just before each coming ReadRange is answered, one
    /// entry a ReadRange.
    log_before_range: std::collections::VecDeque<u64>,
}

impl FakeLog {
    fn new(total: u64, capacity: usize, answer_cap: usize) -> Self {
        Self {
            records: Vec::new(),
            total,
            capacity,
            answer_cap,
            logged: 0,
            clamp_across_wrap: false,
            zero_after_wrap: false,
            log_on_count_read: 0,
            log_before_range: Default::default(),
        }
    }

    fn log(&mut self, count: u64) {
        for _ in 0..count {
            self.total = match self.total {
                TOP if self.zero_after_wrap => 0,
                TOP => 1,
                total => total + 1,
            };
            if self.records.len() == self.capacity {
                self.records.remove(0);
            }
            self.records
                .push((self.total, record(self.total, self.logged)));
            self.logged += 1;
        }
    }

    fn answer(&mut self, service: ConfirmedServiceChoice, data: &[u8]) -> Answer {
        match service {
            ConfirmedServiceChoice::READ_PROPERTY => {
                let request = ReadPropertyRequest::decode(data).unwrap();
                let value = match request.property_identifier {
                    PropertyIdentifier::RECORD_COUNT => self.records.len() as u64,
                    PropertyIdentifier::TOTAL_RECORD_COUNT => self.total,
                    _ => return Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
                };
                if request.property_identifier == PropertyIdentifier::RECORD_COUNT {
                    let grown = std::mem::take(&mut self.log_on_count_read);
                    self.log(grown);
                }
                let mut property_value = BytesMut::new();
                encode_app_unsigned(&mut property_value, value);
                let mut buf = BytesMut::new();
                ReadPropertyACK {
                    object_identifier: request.object_identifier,
                    property_identifier: request.property_identifier,
                    property_array_index: None,
                    property_value: property_value.to_vec(),
                }
                .encode(&mut buf);
                Ok(buf.to_vec())
            }
            ConfirmedServiceChoice::READ_RANGE => {
                let grown = self.log_before_range.pop_front().unwrap_or(0);
                self.log(grown);
                Ok(self.read_range(data))
            }
            _ => Err((ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)),
        }
    }

    fn read_range(&self, data: &[u8]) -> Vec<u8> {
        let request = ReadRangeRequest::decode(data).unwrap();
        let (start, count, sequenced) = match request.range.unwrap() {
            RangeSpec::ByPosition {
                reference_index,
                count,
            } => (
                usize::try_from(reference_index)
                    .ok()
                    .and_then(|index| index.checked_sub(1)),
                count,
                false,
            ),
            RangeSpec::BySequenceNumber {
                reference_seq,
                count,
            } => {
                let found = self.records.iter().position(|(s, _)| *s == reference_seq);
                // Before the wrap a reference past the newest matches nothing,
                // as the standard has it; once the records span the wrap,
                // no reference is between the oldest and newest numbers.
                let clamped = self.clamp_across_wrap
                    && match (self.records.first(), self.records.last()) {
                        (Some((oldest, _)), Some((newest, _))) if oldest > newest => {
                            reference_seq < *oldest || reference_seq > *newest
                        }
                        _ => false,
                    };
                (if clamped { Some(0) } else { found }, count, true)
            }
            RangeSpec::ByTime {
                reference_time: (_, time),
                count,
            } => (
                self.records
                    .iter()
                    .position(|(_, r)| time_key(r.time) > time_key(time)),
                count,
                true,
            ),
        };
        assert!(count > 0, "the reader reads forward");
        let start = start.filter(|start| *start < self.records.len());
        let (start, matched) = match start {
            Some(start) => (start, (self.records.len() - start).min(count as usize)),
            None => (0, 0),
        };
        let returned = matched.min(self.answer_cap);
        let mut item_data = BytesMut::new();
        for (_, record) in &self.records[start..start + returned] {
            encode_log_record(record, &mut item_data).unwrap();
        }
        let ack = ReadRangeAck {
            object_identifier: request.object_identifier,
            property_identifier: request.property_identifier,
            property_array_index: None,
            result_flags: if returned == 0 {
                (false, false, false)
            } else {
                (
                    start == 0,
                    start + returned == self.records.len(),
                    returned < matched,
                )
            },
            item_count: returned as u32,
            item_data: item_data.to_vec(),
            first_sequence_number: (sequenced && returned > 0).then(|| self.records[start].0),
        };
        let mut buf = BytesMut::new();
        ack.encode(&mut buf);
        buf.to_vec()
    }
}

async fn device(
    log: FakeLog,
) -> (
    BACnetClient<bacnet_transport::loopback::LoopbackTransport>,
    FakeDevice,
    Arc<StdMutex<FakeLog>>,
) {
    let log = Arc::new(StdMutex::new(log));
    let shared = Arc::clone(&log);
    let (client, device) =
        client_with_device(move |service, data| shared.lock().unwrap().answer(service, data)).await;
    (client, device, log)
}

/// The sequence numbers the records carry, in order.
fn sequences(records: &LogRecords) -> Vec<u64> {
    let LogRecords::TrendLog(records) = records else {
        panic!("a Trend Log reads Trend Log records")
    };
    records
        .iter()
        .map(|record| match record.log_datum {
            LogDatum::UnsignedValue(sequence) => sequence,
            ref other => panic!("unexpected datum {other:?}"),
        })
        .collect()
}

/// Read pages from `cursor` until one is done, failing past `limit` pages.
async fn read_all(
    client: &BACnetClient<bacnet_transport::loopback::LoopbackTransport>,
    mut cursor: LogCursor,
    limit: usize,
) -> Result<(Vec<u64>, Vec<LogPage>), Error> {
    let mut read = Vec::new();
    let mut pages = Vec::new();
    for _ in 0..limit {
        let page = client
            .read_log_page(&DEVICE_MAC, trend_log(), cursor, 7)
            .await?;
        read.extend(sequences(&page.records));
        cursor = page.next;
        let done = page.done;
        pages.push(page);
        if done {
            return Ok((read, pages));
        }
    }
    panic!("the read did not finish within {limit} pages")
}

#[tokio::test]
async fn reads_a_wrapped_log_from_the_oldest_record_and_resumes_from_its_checkpoint() {
    let mut log = FakeLog::new(TOP - 12, 20, 5);
    log.log(20);
    let (mut client, device, log) = device(log).await;

    let (read, pages) = read_all(&client, LogCursor::Oldest, 20).await.unwrap();
    let expected: Vec<u64> = (TOP - 11..=TOP).chain(1..=8).collect();
    assert_eq!(read, expected);
    assert!(pages.iter().all(|page| page.gap.is_none()));
    // The device cut every 7-record page to 5 with MORE_ITEMS.
    assert!(pages[0].result_flags.2);
    let checkpoint = pages.last().unwrap().next;
    assert_eq!(checkpoint, LogCursor::Sequence(9));
    // Only the third page, TOP - 1 to 3, reaches the top of the range.
    assert_eq!(
        pages.iter().map(|page| page.wrapped).collect::<Vec<_>>(),
        [false, false, true, false]
    );
    // The counts once (total, count, total) for the oldest record, then one
    // ReadRange a page.
    assert_eq!(device.count(ConfirmedServiceChoice::READ_PROPERTY), 3);
    assert_eq!(
        device.count(ConfirmedServiceChoice::READ_RANGE),
        pages.len()
    );

    log.lock().unwrap().log(3);
    let (read, _) = read_all(&client, checkpoint, 5).await.unwrap();
    assert_eq!(read, [9, 10, 11]);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn an_empty_log_reads_as_one_done_page_resuming_at_the_next_number() {
    let (mut client, _device, log) = device(FakeLog::new(41, 10, 5)).await;
    let page = client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Oldest, 10)
        .await
        .unwrap();
    assert!(page.done && page.records.is_empty());
    assert_eq!(page.next, LogCursor::Sequence(42));
    log.lock().unwrap().log(2);
    let (read, _) = read_all(&client, page.next, 3).await.unwrap();
    assert_eq!(read, [42, 43]);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn a_checkpoint_the_log_no_longer_holds_restarts_from_the_oldest_with_a_gap() {
    let mut log = FakeLog::new(0, 10, 50);
    log.log(30);
    let (mut client, _device, _log) = device(log).await;
    let page = client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Sequence(12), 50)
        .await
        .unwrap();
    assert_eq!(sequences(&page.records), (21..=30).collect::<Vec<_>>());
    assert_eq!(
        page.gap,
        Some(LogGap {
            expected: 12,
            first: 21,
            skipped: Some(9),
        })
    );
    assert!(page.done);
    assert_eq!(page.next, LogCursor::Sequence(31));

    // Caught up: an empty page that stays where it is, with no gap.
    let caught_up = client
        .read_log_page(&DEVICE_MAC, trend_log(), page.next, 50)
        .await
        .unwrap();
    assert!(caught_up.done && caught_up.records.is_empty() && caught_up.gap.is_none());
    assert_eq!(caught_up.next, LogCursor::Sequence(31));
    client.stop().await.unwrap();
}

/// bacnet-stack 1.6.1 clamps a reference past its wrap back to the oldest
/// record, so every page after the wrap would be the first page again.
/// The same device reads like any other log until its records reach the
/// wrap; resuming from the checkpoint after that fails instead of looping.
#[tokio::test]
async fn a_clamping_device_reads_correctly_up_to_its_wrap_and_then_fails() {
    let mut log = FakeLog::new(TOP - 30, 20, 5);
    log.clamp_across_wrap = true;
    log.log(20);
    let (mut client, _device, log) = device(log).await;
    let (read, pages) = read_all(&client, LogCursor::Oldest, 20).await.unwrap();
    assert_eq!(read, (TOP - 29..=TOP - 10).collect::<Vec<_>>());
    let checkpoint = pages.last().unwrap().next;
    assert_eq!(checkpoint, LogCursor::Sequence(TOP - 9));
    // Caught up: nothing past the newest, no clamp before the wrap.
    let (read, _) = read_all(&client, checkpoint, 5).await.unwrap();
    assert!(read.is_empty());

    // Fifteen more records take it past the top of the range.
    log.lock().unwrap().log(15);
    let error = read_all(&client, checkpoint, 20).await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::LogNotAdvancing { requested, returned: Some(returned) }
                if requested == TOP - 9 && returned == TOP - 14
        ),
        "{error:?}"
    );
    client.stop().await.unwrap();
}

#[tokio::test]
async fn a_device_that_clamps_past_its_wrap_fails_as_not_advancing_and_reads_by_position() {
    let mut log = FakeLog::new(TOP - 6, 20, 5);
    log.clamp_across_wrap = true;
    log.log(20);
    let (mut client, device, _log) = device(log).await;

    let error = read_all(&client, LogCursor::Oldest, 50).await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::LogNotAdvancing { requested: TOP, returned: Some(returned) } if returned == TOP - 5
        ),
        "{error:?}"
    );
    // The first page read from the oldest record either way; the second
    // came back as the first again.
    assert_eq!(device.count(ConfirmedServiceChoice::READ_RANGE), 2);

    let (read, _) = read_all(&client, LogCursor::Position(1), 20).await.unwrap();
    let expected: Vec<u64> = (TOP - 5..=TOP).chain(1..=14).collect();
    assert_eq!(read, expected);
    client.stop().await.unwrap();
}

/// A record logged between the two count reads would make the oldest look
/// one newer than it is; the total read on both sides catches it.
#[tokio::test]
async fn a_record_logged_between_the_count_reads_does_not_hide_the_oldest() {
    let mut log = FakeLog::new(0, 50, 50);
    log.log(5);
    log.log_on_count_read = 1;
    let (mut client, device, _log) = device(log).await;
    let page = client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Oldest, 50)
        .await
        .unwrap();
    assert_eq!(sequences(&page.records), [1, 2, 3, 4, 5, 6]);
    assert_eq!(page.gap, None);
    // Total, count, total; then count and total again once it moved.
    assert_eq!(device.count(ConfirmedServiceChoice::READ_PROPERTY), 5);
    client.stop().await.unwrap();
}

/// A full log that drops its oldest record between the count read and the
/// ReadRange, once or twice, still reads from the oldest it holds.
#[tokio::test]
async fn a_full_log_that_drops_the_oldest_while_read_restarts_with_a_gap() {
    for (drops, first) in [(1u64, 22u64), (2, 23)] {
        let mut log = FakeLog::new(0, 10, 50);
        log.log(30);
        log.log_before_range = std::iter::repeat_n(1, drops as usize).collect();
        let (mut client, _device, _log) = device(log).await;
        let page = client
            .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Oldest, 50)
            .await
            .unwrap();
        assert_eq!(
            sequences(&page.records),
            (first..=30 + drops).collect::<Vec<_>>(),
            "{drops} drops"
        );
        assert_eq!(
            page.gap,
            Some(LogGap {
                expected: 21,
                first,
                skipped: Some(drops),
            })
        );
        client.stop().await.unwrap();
    }

    // A log that drops its oldest faster than the restarts catch it fails.
    let mut log = FakeLog::new(0, 10, 50);
    log.log(30);
    log.log_before_range = std::iter::repeat_n(1, 10).collect();
    let (mut client, device, _log) = device(log).await;
    let error = client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Oldest, 50)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::LogNotAdvancing { returned: None, .. }),
        "{error:?}"
    );
    assert_eq!(device.count(ConfirmedServiceChoice::READ_RANGE), 4);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn records_logged_after_an_empty_page_are_read_in_the_same_call() {
    let mut log = FakeLog::new(0, 50, 50);
    log.log(5);
    log.log_on_count_read = 2;
    let (mut client, device, _log) = device(log).await;
    let page = client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Sequence(6), 50)
        .await
        .unwrap();
    assert_eq!(sequences(&page.records), [6, 7]);
    assert_eq!(page.gap, None);
    assert_eq!(page.next, LogCursor::Sequence(8));
    assert_eq!(device.count(ConfirmedServiceChoice::READ_RANGE), 2);
    client.stop().await.unwrap();
}

/// A device that numbers through 0 holds one number more per cycle than
/// the counts imply, so the oldest record computed from them isn't there.
#[tokio::test]
async fn a_device_numbering_through_zero_fails_from_the_oldest_and_reads_by_position() {
    let mut log = FakeLog::new(TOP - 2, 20, 50);
    log.zero_after_wrap = true;
    log.log(8);
    let (mut client, _device, _log) = device(log).await;
    let error = client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Oldest, 50)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::LogNotAdvancing { requested, returned: None } if requested == TOP - 2
        ),
        "{error:?}"
    );
    let (read, _) = read_all(&client, LogCursor::Position(1), 5).await.unwrap();
    assert_eq!(read, [TOP - 1, TOP, 0, 1, 2, 3, 4, 5]);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn a_page_numbered_from_zero_after_the_wrap_is_kept_and_listed() {
    let mut log = FakeLog::new(TOP - 2, 20, 50);
    log.zero_after_wrap = true;
    log.log(8);
    let (mut client, _device, _log) = device(log).await;
    // The standard's successor of the top is 1; this device calls it 0.
    let page = client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Sequence(1), 50)
        .await;
    // Sequence 1 is a record of this device (the second after its wrap).
    let page = page.unwrap();
    assert_eq!(sequences(&page.records), [1, 2, 3, 4, 5]);
    let from_zero = client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Sequence(0), 50)
        .await
        .unwrap();
    assert_eq!(sequences(&from_zero.records), [0, 1, 2, 3, 4, 5]);
    assert_eq!(
        from_zero.violations,
        [ReadRangeViolation::ZeroFirstSequenceNumber]
    );
    assert_eq!(from_zero.next, LogCursor::Sequence(6));
    client.stop().await.unwrap();
}

#[tokio::test]
async fn by_time_reads_records_newer_than_the_reference_then_by_sequence() {
    let mut log = FakeLog::new(100, 50, 3);
    log.log(10);
    let (mut client, device, _log) = device(log).await;
    // Records are a second apart from 09:00:00; ask for those after :05.
    let (read, pages) = read_all(&client, LogCursor::Time(DATE, time_of(5)), 10)
        .await
        .unwrap();
    assert_eq!(read, [107, 108, 109, 110]);
    assert_eq!(pages[0].first_sequence_number, Some(107));
    let second = device.requests.lock().unwrap()[1].1.clone();
    let by_sequence = ReadRangeRequest::decode(&second).unwrap();
    assert!(matches!(
        by_sequence.range,
        Some(RangeSpec::BySequenceNumber {
            reference_seq: 110,
            ..
        })
    ));
    client.stop().await.unwrap();
}

#[tokio::test]
async fn a_log_reader_refuses_a_non_log_object_and_a_bad_page_size_before_sending() {
    let (mut client, device, _log) = device(FakeLog::new(0, 1, 1)).await;
    let analog = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert!(client
        .read_log_page(&DEVICE_MAC, analog, LogCursor::Oldest, 10)
        .await
        .is_err());
    assert!(client
        .read_log_page(&DEVICE_MAC, trend_log(), LogCursor::Oldest, 0)
        .await
        .is_err());
    assert!(device.requests.lock().unwrap().is_empty());
    client.stop().await.unwrap();
}
