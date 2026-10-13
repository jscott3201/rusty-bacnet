//! Criterion suite: the server's cost to answer one ReadRange of a full
//! Trend Log's Log_Buffer, by request kind and log size (#1536).
//!
//! Each request asks for 100 records from the middle of the log, so the
//! encoded page is the same size at every log size and any growth with the
//! log is the cost of finding the window. `by_time_clock_back` reads a log
//! whose clock stepped back once, which the time search can't bisect.
use bytes::BytesMut;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::trend::TrendLogObject;
use bacnet_server::handlers::handle_read_range;
use bacnet_services::read_range::{RangeSpec, ReadRangeRequest};
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{Date, Time};

const SIZES: [u32; 2] = [10_000, 100_000];
const PAGE: i32 = 100;

/// The timestamp of sample `second`: one record a second from midnight on
/// 1 August 2026.
fn timestamp(second: u32) -> (Date, Time) {
    let day = second / 86_400;
    let in_day = second % 86_400;
    (
        Date {
            year: 126,
            month: 8,
            day: 1 + day as u8,
            day_of_week: 1 + (day % 7) as u8,
        },
        Time {
            hour: (in_day / 3_600) as u8,
            minute: (in_day / 60 % 60) as u8,
            second: (in_day % 60) as u8,
            hundredths: 0,
        },
    )
}

/// A full Trend Log of `size` records. With `clock_back`, the clock steps
/// back an hour a quarter of the way in.
fn full_log(size: u32, clock_back: bool) -> (ObjectDatabase, ReadRangeRequest) {
    let mut log = TrendLogObject::new(1, "TL-1", size).unwrap();
    for sample in 0..size {
        let second = if clock_back && sample >= size / 4 {
            sample - 3_600.min(sample)
        } else {
            sample
        };
        let (date, time) = timestamp(second);
        log.add_record(BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::RealValue(sample as f32),
            status_flags: None,
        })
        .unwrap();
    }
    let request = ReadRangeRequest {
        object_identifier: bacnet_objects::traits::BACnetObject::object_identifier(&log),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: None,
    };
    let mut db = ObjectDatabase::new();
    db.add(Box::new(log)).unwrap();
    (db, request)
}

fn encoded(request: &ReadRangeRequest, range: RangeSpec) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    ReadRangeRequest {
        range: Some(range),
        ..request.clone()
    }
    .encode(&mut bytes)
    .unwrap();
    bytes.to_vec()
}

fn bench_read_range(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_range_trend_log");
    for size in SIZES {
        let (db, request) = full_log(size, false);
        let middle = size / 2;
        let cases = [
            (
                "by_position",
                RangeSpec::ByPosition {
                    reference_index: u64::from(middle),
                    count: PAGE,
                },
            ),
            (
                "by_sequence",
                RangeSpec::BySequenceNumber {
                    reference_seq: u64::from(middle),
                    count: PAGE,
                },
            ),
            (
                "by_time",
                RangeSpec::ByTime {
                    reference_time: timestamp(middle),
                    count: PAGE,
                },
            ),
        ];
        for (name, range) in cases {
            let service_data = encoded(&request, range);
            group.bench_with_input(BenchmarkId::new(name, size), &service_data, |b, data| {
                b.iter(|| {
                    let mut response = BytesMut::with_capacity(4_096);
                    handle_read_range(&db, data, &mut response).unwrap();
                    black_box(response);
                })
            });
        }

        let (db, request) = full_log(size, true);
        let service_data = encoded(
            &request,
            RangeSpec::ByTime {
                reference_time: timestamp(middle),
                count: PAGE,
            },
        );
        group.bench_with_input(
            BenchmarkId::new("by_time_clock_back", size),
            &service_data,
            |b, data| {
                b.iter(|| {
                    let mut response = BytesMut::with_capacity(4_096);
                    handle_read_range(&db, data, &mut response).unwrap();
                    black_box(response);
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_read_range);
criterion_main!(benches);
