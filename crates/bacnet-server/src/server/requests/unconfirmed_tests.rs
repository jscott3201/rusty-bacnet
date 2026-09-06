use super::super::{ClockConfig, ServerClock, ServerConfig};
use super::unconfirmed::apply_time_sync_request;
use crate::server::TimeSyncData;
use bacnet_objects::clock::ClockReader;
use bacnet_services::device_mgmt::TimeSynchronizationRequest;
use bacnet_types::primitives::{Date, Time};
use bytes::{Bytes, BytesMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> Date {
    Date {
        year: (year - 1900) as u8,
        month,
        day,
        day_of_week,
    }
}

fn time(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn encoded(date: Date, time: Time) -> Bytes {
    let mut bytes = BytesMut::new();
    TimeSynchronizationRequest { date, time }.encode(&mut bytes);
    bytes.freeze()
}

fn config_with_counter(counter: Arc<AtomicUsize>) -> ServerConfig {
    ServerConfig {
        on_time_sync: Some(Arc::new(move |_: TimeSyncData| {
            counter.fetch_add(1, Ordering::SeqCst);
        })),
        ..ServerConfig::default()
    }
}

#[test]
fn accepted_local_and_utc_requests_update_then_notify() {
    let clock = ServerClock::new(ClockConfig::new(300, true).unwrap());
    let callbacks = Arc::new(AtomicUsize::new(0));
    let config = config_with_counter(Arc::clone(&callbacks));

    apply_time_sync_request(
        Some(&clock),
        &config,
        encoded(date(2024, 7, 4, 4), time(9, 15)),
        false,
    )
    .unwrap();
    let frame = clock.read_clock().unwrap();
    assert_eq!((frame.local_time.hour, frame.local_time.minute), (9, 15));
    assert_eq!(callbacks.load(Ordering::SeqCst), 1);

    apply_time_sync_request(
        Some(&clock),
        &config,
        encoded(date(2024, 7, 4, 4), time(13, 15)),
        true,
    )
    .unwrap();
    let frame = clock.read_clock().unwrap();
    assert_eq!((frame.local_time.hour, frame.local_time.minute), (9, 15));
    assert_eq!(callbacks.load(Ordering::SeqCst), 2);
}

#[test]
fn invalid_and_clockless_requests_do_not_notify() {
    let clock = ServerClock::new(ClockConfig::default());
    let callbacks = Arc::new(AtomicUsize::new(0));
    let config = config_with_counter(Arc::clone(&callbacks));

    assert!(
        apply_time_sync_request(Some(&clock), &config, Bytes::from_static(&[0xff]), false,)
            .is_err()
    );
    assert!(apply_time_sync_request(
        Some(&clock),
        &config,
        encoded(
            Date {
                year: Date::UNSPECIFIED,
                month: 1,
                day: 1,
                day_of_week: 1,
            },
            time(0, 0),
        ),
        false,
    )
    .is_err());
    assert!(apply_time_sync_request(
        None,
        &config,
        encoded(date(2024, 7, 4, 4), time(9, 15)),
        false,
    )
    .is_err());
    assert_eq!(callbacks.load(Ordering::SeqCst), 0);
}
