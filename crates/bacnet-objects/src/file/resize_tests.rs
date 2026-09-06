//! Conditional File_Size / Record_Count property resize behavior.

use super::*;
use crate::clock::{ClockFrame, ClockReader};
use crate::traits::WritePropertyRollback;
use std::sync::atomic::{AtomicUsize, Ordering};

fn protocol_pair(error: Error) -> (u32, u32) {
    match error {
        Error::Protocol { class, code } => (class, code),
        other => panic!("expected protocol error, got {other:?}"),
    }
}

fn write_denied_pair() -> (u32, u32) {
    (
        ErrorClass::PROPERTY.to_raw() as u32,
        ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
    )
}

fn invalid_type_pair() -> (u32, u32) {
    (
        ErrorClass::PROPERTY.to_raw() as u32,
        ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
    )
}

fn file_full_pair() -> (u32, u32) {
    (
        ErrorClass::OBJECT.to_raw() as u32,
        ErrorCode::FILE_FULL.to_raw() as u32,
    )
}

fn write_resize(
    file: &mut FileObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    file.write_property(property, None, value, None)
}

#[derive(Debug, PartialEq)]
struct FileState {
    data: Vec<u8>,
    records: Vec<Vec<u8>>,
    file_size: u64,
    record_count: Option<u64>,
    modification_date: (Date, Time),
    archive: bool,
}

fn state(file: &FileObject) -> FileState {
    FileState {
        data: file.data.clone(),
        records: file.records.clone(),
        file_size: file.file_size,
        record_count: file.record_count,
        modification_date: file.modification_date,
        archive: file.archive,
    }
}

fn dated_state(file: &mut FileObject) -> FileState {
    let frame = clock_frame(1);
    file.set_modification_date(frame.local_date, frame.local_time);
    file.set_archive(true);
    state(file)
}

fn stream_file(data: &[u8]) -> FileObject {
    let mut file = FileObject::new(1, "STREAM", "application/octet-stream").unwrap();
    file.set_data(data.to_vec());
    file
}

fn record_file(records: Vec<Vec<u8>>) -> FileObject {
    let mut file = FileObject::new(2, "RECORD", "application/octet-stream").unwrap();
    file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    file.set_records(records);
    file
}

fn clock_frame(hour: u8) -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 8,
            day: 31,
            day_of_week: 1,
        },
        local_time: Time {
            hour,
            minute: 2,
            second: 3,
            hundredths: 4,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

struct CountingClock {
    frame: ClockFrame,
    reads: Arc<AtomicUsize>,
}

impl ClockReader for CountingClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Some(self.frame)
    }
}

#[test]
fn stream_file_size_zero_shrink_and_expand_use_prefix_and_zero_fill() {
    let mut cleared = stream_file(&[1, 2, 3, 4]);
    write_resize(
        &mut cleared,
        PropertyIdentifier::FILE_SIZE,
        PropertyValue::Unsigned(0),
    )
    .unwrap();
    assert!(cleared.data().is_empty());
    assert_eq!(cleared.file_size(), 0);

    let mut shrunk = stream_file(&[1, 2, 3, 4]);
    write_resize(
        &mut shrunk,
        PropertyIdentifier::FILE_SIZE,
        PropertyValue::Unsigned(2),
    )
    .unwrap();
    assert_eq!(shrunk.data(), &[1, 2]);
    assert_eq!(shrunk.file_size(), 2);

    let mut expanded = stream_file(&[1, 2]);
    write_resize(
        &mut expanded,
        PropertyIdentifier::FILE_SIZE,
        PropertyValue::Unsigned(5),
    )
    .unwrap();
    assert_eq!(expanded.data(), &[1, 2, 0, 0, 0]);
    assert_eq!(expanded.file_size(), 5);
}

#[test]
fn stream_file_size_failures_are_atomic_and_preloaded_growth_only_cap_is_preserved() {
    let mut file = stream_file(&[1, 2, 3, 4, 5, 6, 7, 8]);
    file.set_max_file_size(4);
    let expected = dated_state(&mut file);
    assert_eq!(
        protocol_pair(
            write_resize(
                &mut file,
                PropertyIdentifier::FILE_SIZE,
                PropertyValue::Unsigned(u64::MAX),
            )
            .unwrap_err()
        ),
        file_full_pair()
    );
    assert_eq!(state(&file), expected);

    write_resize(
        &mut file,
        PropertyIdentifier::FILE_SIZE,
        PropertyValue::Unsigned(6),
    )
    .unwrap();
    assert_eq!(file.data(), &[1, 2, 3, 4, 5, 6]);
    let expected = dated_state(&mut file);
    assert_eq!(
        protocol_pair(
            write_resize(
                &mut file,
                PropertyIdentifier::FILE_SIZE,
                PropertyValue::Unsigned(7),
            )
            .unwrap_err()
        ),
        file_full_pair()
    );
    assert_eq!(state(&file), expected);
}

#[test]
fn record_count_zero_shrink_and_expand_keep_octet_accounting_coherent() {
    let records = vec![vec![1, 2], vec![3], vec![4, 5, 6]];
    let mut cleared = record_file(records.clone());
    write_resize(
        &mut cleared,
        PropertyIdentifier::RECORD_COUNT,
        PropertyValue::Unsigned(0),
    )
    .unwrap();
    assert!(cleared.records().is_empty());
    assert_eq!(cleared.record_count, Some(0));
    assert_eq!(cleared.file_size(), 0);

    let mut shrunk = record_file(records.clone());
    write_resize(
        &mut shrunk,
        PropertyIdentifier::RECORD_COUNT,
        PropertyValue::Unsigned(2),
    )
    .unwrap();
    assert_eq!(shrunk.records(), &records[..2]);
    assert_eq!(shrunk.record_count, Some(2));
    assert_eq!(shrunk.file_size(), 3);

    let mut expanded = record_file(records);
    write_resize(
        &mut expanded,
        PropertyIdentifier::RECORD_COUNT,
        PropertyValue::Unsigned(5),
    )
    .unwrap();
    assert_eq!(
        expanded.records(),
        &[vec![1, 2], vec![3], vec![4, 5, 6], vec![], vec![]]
    );
    assert_eq!(expanded.record_count, Some(5));
    assert_eq!(expanded.file_size(), 6);
}

#[test]
fn record_count_cap_failure_is_atomic_and_preloaded_shrink_is_allowed() {
    let mut file = record_file(vec![vec![1], vec![2], vec![3], vec![4]]);
    file.set_max_record_count(2);
    let expected = dated_state(&mut file);
    assert_eq!(
        protocol_pair(
            write_resize(
                &mut file,
                PropertyIdentifier::RECORD_COUNT,
                PropertyValue::Unsigned(u64::MAX),
            )
            .unwrap_err()
        ),
        file_full_pair()
    );
    assert_eq!(state(&file), expected);

    write_resize(
        &mut file,
        PropertyIdentifier::RECORD_COUNT,
        PropertyValue::Unsigned(3),
    )
    .unwrap();
    assert_eq!(file.records(), &[vec![1], vec![2], vec![3]]);
    let expected = dated_state(&mut file);
    assert_eq!(
        protocol_pair(
            write_resize(
                &mut file,
                PropertyIdentifier::RECORD_COUNT,
                PropertyValue::Unsigned(4),
            )
            .unwrap_err()
        ),
        file_full_pair()
    );
    assert_eq!(state(&file), expected);
}

#[test]
fn resize_eligibility_precedes_type_validation_and_every_denial_is_atomic() {
    let mut record = record_file(vec![vec![1], vec![2]]);
    let expected = dated_state(&mut record);
    for value in [PropertyValue::Unsigned(1), PropertyValue::Boolean(false)] {
        assert_eq!(
            protocol_pair(
                write_resize(&mut record, PropertyIdentifier::FILE_SIZE, value).unwrap_err()
            ),
            write_denied_pair()
        );
        assert_eq!(state(&record), expected);
    }

    let mut stream = stream_file(&[1, 2]);
    let expected = dated_state(&mut stream);
    for value in [PropertyValue::Unsigned(1), PropertyValue::Boolean(false)] {
        assert_eq!(
            protocol_pair(
                write_resize(&mut stream, PropertyIdentifier::RECORD_COUNT, value).unwrap_err()
            ),
            write_denied_pair()
        );
        assert_eq!(state(&stream), expected);
    }

    let mut unknown = stream_file(&[1, 2]);
    unknown.set_file_access_method(99);
    let expected = dated_state(&mut unknown);
    for property in [
        PropertyIdentifier::FILE_SIZE,
        PropertyIdentifier::RECORD_COUNT,
    ] {
        assert_eq!(
            protocol_pair(
                write_resize(&mut unknown, property, PropertyValue::Boolean(false)).unwrap_err()
            ),
            write_denied_pair()
        );
        assert_eq!(state(&unknown), expected);
    }

    for mut file in [stream_file(&[1, 2]), record_file(vec![vec![1]])] {
        file.set_read_only(true);
        let property = if file.record_count.is_some() {
            PropertyIdentifier::RECORD_COUNT
        } else {
            PropertyIdentifier::FILE_SIZE
        };
        let expected = dated_state(&mut file);
        assert_eq!(
            protocol_pair(
                write_resize(&mut file, property, PropertyValue::Boolean(false)).unwrap_err()
            ),
            write_denied_pair()
        );
        assert_eq!(state(&file), expected);
    }

    let mut stream = stream_file(&[1, 2]);
    let expected = dated_state(&mut stream);
    assert_eq!(
        protocol_pair(
            write_resize(
                &mut stream,
                PropertyIdentifier::FILE_SIZE,
                PropertyValue::Boolean(false),
            )
            .unwrap_err()
        ),
        invalid_type_pair()
    );
    assert_eq!(state(&stream), expected);

    let mut record = record_file(vec![vec![1]]);
    let expected = dated_state(&mut record);
    assert_eq!(
        protocol_pair(
            write_resize(
                &mut record,
                PropertyIdentifier::RECORD_COUNT,
                PropertyValue::Boolean(false),
            )
            .unwrap_err()
        ),
        invalid_type_pair()
    );
    assert_eq!(state(&record), expected);
}

#[test]
fn resize_noop_is_clock_and_metadata_neutral_but_actual_change_samples_once() {
    let old = clock_frame(1);
    let changed = clock_frame(7);
    let reads = Arc::new(AtomicUsize::new(0));
    let mut file = stream_file(&[1, 2, 3]);
    file.set_modification_date(old.local_date, old.local_time);
    file.set_archive(true);
    file.bind_clock_internal(Some(Arc::new(CountingClock {
        frame: changed,
        reads: reads.clone(),
    })));

    let expected = state(&file);
    write_resize(
        &mut file,
        PropertyIdentifier::FILE_SIZE,
        PropertyValue::Unsigned(3),
    )
    .unwrap();
    assert_eq!(state(&file), expected);
    assert_eq!(reads.load(Ordering::SeqCst), 0);

    write_resize(
        &mut file,
        PropertyIdentifier::FILE_SIZE,
        PropertyValue::Unsigned(2),
    )
    .unwrap();
    assert_eq!(file.data(), &[1, 2]);
    assert_eq!(
        file.modification_date,
        (changed.local_date, changed.local_time)
    );
    assert!(!file.archive());
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn empty_record_growth_is_a_metadata_change_even_when_file_size_is_unchanged() {
    let old = clock_frame(1);
    let changed = clock_frame(8);
    let reads = Arc::new(AtomicUsize::new(0));
    let mut file = record_file(vec![vec![], vec![]]);
    file.set_modification_date(old.local_date, old.local_time);
    file.set_archive(true);
    file.bind_clock_internal(Some(Arc::new(CountingClock {
        frame: changed,
        reads: reads.clone(),
    })));

    write_resize(
        &mut file,
        PropertyIdentifier::RECORD_COUNT,
        PropertyValue::Unsigned(3),
    )
    .unwrap();
    assert_eq!(file.records(), &[vec![], vec![], vec![]]);
    assert_eq!(file.record_count, Some(3));
    assert_eq!(file.file_size(), 0);
    assert_eq!(
        file.modification_date,
        (changed.local_date, changed.local_time)
    );
    assert!(!file.archive());
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn resize_writability_and_rollback_capture_follow_runtime_eligibility() {
    let mut stream = stream_file(&[1, 2, 3]);
    for property in [
        PropertyIdentifier::DESCRIPTION,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyIdentifier::ARCHIVE,
        PropertyIdentifier::FILE_TYPE,
        PropertyIdentifier::FILE_SIZE,
    ] {
        assert!(stream.is_writable_property(property), "{property:?}");
    }
    assert!(!stream.is_writable_property(PropertyIdentifier::RECORD_COUNT));
    assert!(
        stream
            .capture_write_property_rollback(
                PropertyIdentifier::FILE_SIZE,
                &PropertyValue::Unsigned(2),
            )
            .is_some()
    );
    assert!(
        stream
            .capture_write_property_rollback(
                PropertyIdentifier::FILE_SIZE,
                &PropertyValue::Unsigned(3),
            )
            .is_none()
    );
    assert!(stream
        .capture_write_property_rollback(
            PropertyIdentifier::FILE_SIZE,
            &PropertyValue::Boolean(false),
        )
        .is_none());

    let mut record = record_file(vec![vec![1], vec![2]]);
    assert!(!record.is_writable_property(PropertyIdentifier::FILE_SIZE));
    assert!(record.is_writable_property(PropertyIdentifier::RECORD_COUNT));
    record.set_read_only(true);
    assert!(!record.is_writable_property(PropertyIdentifier::FILE_SIZE));
    assert!(!record.is_writable_property(PropertyIdentifier::RECORD_COUNT));
    assert!(record
        .capture_write_property_rollback(
            PropertyIdentifier::RECORD_COUNT,
            &PropertyValue::Unsigned(1),
        )
        .is_none());

    let expected = state(&stream);
    assert!(stream
        .restore_write_property_rollback(WritePropertyRollback::new("wrong token"))
        .is_err());
    assert_eq!(state(&stream), expected);
}
