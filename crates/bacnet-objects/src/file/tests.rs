use super::*;
use crate::clock::{ClockFrame, ClockReader};
use bacnet_types::enums::ErrorCode;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

struct SequenceClock(Mutex<VecDeque<ClockFrame>>);

impl SequenceClock {
    fn new(frames: impl IntoIterator<Item = ClockFrame>) -> Self {
        Self(Mutex::new(frames.into_iter().collect()))
    }
}

impl ClockReader for SequenceClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.0.lock().ok()?.pop_front()
    }
}

fn clock_frame(hour: u8) -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 124,
            month: 2,
            day: 29,
            day_of_week: 4,
        },
        local_time: Time {
            hour,
            minute: 15,
            second: 30,
            hundredths: 25,
        },
        utc_offset: 300,
        daylight_savings_status: false,
    }
}

fn unspecified_datetime() -> (Date, Time) {
    (
        Date {
            year: Date::UNSPECIFIED,
            month: Date::UNSPECIFIED,
            day: Date::UNSPECIFIED,
            day_of_week: Date::UNSPECIFIED,
        },
        Time {
            hour: Time::UNSPECIFIED,
            minute: Time::UNSPECIFIED,
            second: Time::UNSPECIFIED,
            hundredths: Time::UNSPECIFIED,
        },
    )
}

#[test]
fn file_object_creation() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    assert_eq!(file.object_name(), "FILE-1");
    assert_eq!(file.object_identifier().instance_number(), 1);
}

#[test]
fn file_read_object_type() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(ObjectType::FILE.to_raw()));
}

#[test]
fn file_read_object_identifier() {
    let file = FileObject::new(42, "FILE-42", "application/octet-stream").unwrap();
    let val = file
        .read_property(PropertyIdentifier::OBJECT_IDENTIFIER, None)
        .unwrap();
    if let PropertyValue::ObjectIdentifier(oid) = val {
        assert_eq!(oid.instance_number(), 42);
    } else {
        panic!("expected ObjectIdentifier");
    }
}

#[test]
fn file_read_object_name() {
    let file = FileObject::new(1, "MY-FILE", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::OBJECT_NAME, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("MY-FILE".into()));
}

#[test]
fn file_read_file_type() {
    let file = FileObject::new(1, "FILE-1", "text/csv").unwrap();
    let val = file
        .read_property(PropertyIdentifier::FILE_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("text/csv".into()));
}

#[test]
fn file_read_file_size_default_zero() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(0));
}

#[test]
fn file_set_data_updates_file_size() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_data(vec![0x48, 0x65, 0x6C, 0x6C, 0x6F]); // "Hello"
    let val = file
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(5));
    assert_eq!(file.data(), &[0x48, 0x65, 0x6C, 0x6C, 0x6F]);
}

#[test]
fn file_read_archive_default_false() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::ARCHIVE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
}

#[test]
fn file_set_and_read_archive() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_archive(true);
    assert!(file.archive());
    let val = file
        .read_property(PropertyIdentifier::ARCHIVE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn file_read_read_only_default_false() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::READ_ONLY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
}

#[test]
fn file_set_and_read_read_only() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_read_only(true);
    assert!(file.read_only());
    let val = file
        .read_property(PropertyIdentifier::READ_ONLY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn file_read_modification_date_default_unspecified() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::MODIFICATION_DATE, None)
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert_eq!(items.len(), 2);
        let unspec_date = Date {
            year: 0xFF,
            month: 0xFF,
            day: 0xFF,
            day_of_week: 0xFF,
        };
        let unspec_time = Time {
            hour: 0xFF,
            minute: 0xFF,
            second: 0xFF,
            hundredths: 0xFF,
        };
        assert_eq!(items[0], PropertyValue::Date(unspec_date));
        assert_eq!(items[1], PropertyValue::Time(unspec_time));
    } else {
        panic!("expected PropertyValue::List");
    }
}

#[test]
fn file_set_and_read_modification_date() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let d = Date {
        year: 126,
        month: 3,
        day: 1,
        day_of_week: 7,
    };
    let t = Time {
        hour: 14,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    file.set_modification_date(d, t);
    let val = file
        .read_property(PropertyIdentifier::MODIFICATION_DATE, None)
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert_eq!(items[0], PropertyValue::Date(d));
        assert_eq!(items[1], PropertyValue::Time(t));
    } else {
        panic!("expected PropertyValue::List");
    }
}

#[test]
fn file_metadata_direct_payload_mutations_use_bound_clock() {
    let old = clock_frame(1);

    let stream_frame = clock_frame(8);
    let mut stream = FileObject::new(1, "STREAM", "text/plain").unwrap();
    stream.set_modification_date(old.local_date, old.local_time);
    stream.set_archive(true);
    stream.bind_clock_internal(Some(Arc::new(SequenceClock::new([stream_frame]))));
    assert_eq!(
        stream.modification_date,
        (old.local_date, old.local_time),
        "clock binding must not timestamp the object"
    );
    assert!(stream.archive());
    stream.set_data(vec![1, 2, 3]);
    assert_eq!(
        stream.modification_date,
        (stream_frame.local_date, stream_frame.local_time)
    );
    assert!(!stream.archive());

    let record_frame = clock_frame(9);
    let mut record = FileObject::new(2, "RECORD", "text/plain").unwrap();
    record.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    record.set_modification_date(old.local_date, old.local_time);
    record.set_archive(true);
    record.bind_clock_internal(Some(Arc::new(SequenceClock::new([record_frame]))));
    record.set_records(vec![vec![0xAA], vec![0xBB, 0xCC]]);
    assert_eq!(
        record.modification_date,
        (record_frame.local_date, record_frame.local_time)
    );
    assert!(!record.archive());
}

#[test]
fn file_metadata_clockless_and_invalid_frames_are_fully_unspecified() {
    let old = clock_frame(1);

    let mut clockless = FileObject::new(1, "CLOCKLESS", "text/plain").unwrap();
    clockless.set_modification_date(old.local_date, old.local_time);
    clockless.set_archive(true);
    clockless.set_data(vec![1]);
    assert_eq!(clockless.modification_date, unspecified_datetime());
    assert!(!clockless.archive());

    let mut invalid_frame = clock_frame(2);
    invalid_frame.local_time.hour = 24;
    let mut invalid = FileObject::new(2, "INVALID", "text/plain").unwrap();
    invalid.set_modification_date(old.local_date, old.local_time);
    invalid.set_archive(true);
    invalid.bind_clock_internal(Some(Arc::new(SequenceClock::new([invalid_frame]))));
    invalid.set_data(vec![2]);
    assert_eq!(invalid.modification_date, unspecified_datetime());
    assert!(!invalid.archive());
}

#[test]
fn file_metadata_equal_and_empty_mutations_sample_the_next_frame() {
    let first = clock_frame(10);
    let second = clock_frame(11);
    let mut file = FileObject::new(1, "FILE", "text/plain").unwrap();
    file.set_data(vec![1, 2]);
    file.set_archive(true);
    file.bind_clock_internal(Some(Arc::new(SequenceClock::new([first, second]))));

    file.set_data(vec![1, 2]);
    assert_eq!(
        file.modification_date,
        (first.local_date, first.local_time),
        "an equal-content mutation still selects a new Modification_Date"
    );
    assert!(!file.archive());

    file.set_archive(true);
    file.set_data(Vec::new());
    assert_eq!(
        file.modification_date,
        (second.local_date, second.local_time),
        "an empty successful mutation samples the clock again"
    );
    assert!(!file.archive());
}

#[test]
fn file_metadata_manual_assignment_clears_archive_only_when_changed() {
    let first = clock_frame(4);
    let second = clock_frame(5);
    let mut file = FileObject::new(1, "FILE", "text/plain").unwrap();
    file.set_modification_date(first.local_date, first.local_time);
    file.set_archive(true);

    file.set_modification_date(first.local_date, first.local_time);
    assert!(file.archive(), "an identical assignment is not a change");

    file.set_modification_date(second.local_date, second.local_time);
    assert_eq!(
        file.modification_date,
        (second.local_date, second.local_time)
    );
    assert!(!file.archive());
}

#[test]
fn file_metadata_access_method_marks_only_effective_file_size_changes() {
    let old = clock_frame(1);
    let next = clock_frame(6);
    let mut equal_size = FileObject::new(1, "EQUAL", "text/plain").unwrap();
    equal_size.set_data(vec![1, 2]);
    equal_size.set_records(vec![vec![3], vec![4]]);
    equal_size.set_modification_date(old.local_date, old.local_time);
    equal_size.set_archive(true);
    equal_size.bind_clock_internal(Some(Arc::new(SequenceClock::new([next]))));

    equal_size.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    assert_eq!(equal_size.file_size(), 2);
    assert_eq!(
        equal_size.modification_date,
        (old.local_date, old.local_time)
    );
    assert!(
        equal_size.archive(),
        "method and Record_Count changes with equal File_Size are not modification events"
    );

    equal_size.set_data(vec![1, 2]);
    assert_eq!(
        equal_size.modification_date,
        (next.local_date, next.local_time),
        "the equal-size method switch must not consume the clock sample"
    );
    assert!(!equal_size.archive());

    let changed = clock_frame(7);
    let mut changed_size = FileObject::new(2, "CHANGED", "text/plain").unwrap();
    changed_size.set_data(vec![1, 2, 3]);
    changed_size.set_records(vec![vec![4], vec![5]]);
    changed_size.set_modification_date(old.local_date, old.local_time);
    changed_size.set_archive(true);
    changed_size.bind_clock_internal(Some(Arc::new(SequenceClock::new([changed]))));
    changed_size.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    assert_eq!(changed_size.file_size(), 2);
    assert_eq!(
        changed_size.modification_date,
        (changed.local_date, changed.local_time)
    );
    assert!(!changed_size.archive());
}

#[test]
fn file_read_file_access_method_default_stream() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::FILE_ACCESS_METHOD, None)
        .unwrap();
    // stream-access is 1 per the Clause 21 production (#273).
    assert_eq!(
        val,
        PropertyValue::Enumerated(FileAccessMethod::STREAM_ACCESS.to_raw())
    );
    assert_eq!(
        val,
        PropertyValue::Enumerated(1),
        "stream-access enumeration value"
    );
}

#[test]
fn file_read_file_access_method_record() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    let val = file
        .read_property(PropertyIdentifier::FILE_ACCESS_METHOD, None)
        .unwrap();
    // record-access is 0 per the Clause 21 production (#273).
    assert_eq!(
        val,
        PropertyValue::Enumerated(FileAccessMethod::RECORD_ACCESS.to_raw())
    );
    assert_eq!(
        val,
        PropertyValue::Enumerated(0),
        "record-access enumeration value"
    );
}

#[test]
fn file_record_count_unavailable_for_stream() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let result = file.read_property(PropertyIdentifier::RECORD_COUNT, None);
    assert!(result.is_err());
}

#[test]
fn file_set_records_updates_record_count_and_size() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    file.set_records(vec![vec![0x01, 0x02], vec![0x03, 0x04, 0x05]]);
    let count = file
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .unwrap();
    assert_eq!(count, PropertyValue::Unsigned(2));
    let size = file
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .unwrap();
    assert_eq!(size, PropertyValue::Unsigned(5)); // 2 + 3 bytes
    assert_eq!(file.records().len(), 2);
}

#[test]
fn file_read_status_flags_default() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::STATUS_FLAGS, None)
        .unwrap();
    if let PropertyValue::BitString { unused_bits, data } = val {
        assert_eq!(unused_bits, 4);
        assert_eq!(data, vec![0x00]);
    } else {
        panic!("expected BitString");
    }
}

#[test]
fn file_read_out_of_service_default_false() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
}

#[test]
fn file_read_reliability_default() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0));
}

#[test]
fn file_read_description_default_empty() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let val = file
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString(String::new()));
}

#[test]
fn file_write_description() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("A test file".into()),
        None,
    )
    .unwrap();
    let val = file
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("A test file".into()));
}

#[test]
fn file_write_archive() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.write_property(
        PropertyIdentifier::ARCHIVE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let val = file
        .read_property(PropertyIdentifier::ARCHIVE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn file_write_archive_invalid_type() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let result = file.write_property(
        PropertyIdentifier::ARCHIVE,
        None,
        PropertyValue::Unsigned(1),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn file_write_file_type() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.write_property(
        PropertyIdentifier::FILE_TYPE,
        None,
        PropertyValue::CharacterString("application/json".into()),
        None,
    )
    .unwrap();
    let val = file
        .read_property(PropertyIdentifier::FILE_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::CharacterString("application/json".into())
    );
}

#[test]
fn file_write_out_of_service() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let val = file
        .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
}

#[test]
fn file_write_read_only_denied() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let result = file.write_property(
        PropertyIdentifier::READ_ONLY,
        None,
        PropertyValue::Boolean(true),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn file_write_file_size_denied() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let result = file.write_property(
        PropertyIdentifier::FILE_SIZE,
        None,
        PropertyValue::Unsigned(100),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn file_property_list_stream() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let props = file.property_list();
    assert!(props.contains(&PropertyIdentifier::OBJECT_IDENTIFIER));
    assert!(props.contains(&PropertyIdentifier::OBJECT_NAME));
    assert!(props.contains(&PropertyIdentifier::OBJECT_TYPE));
    assert!(props.contains(&PropertyIdentifier::FILE_TYPE));
    assert!(props.contains(&PropertyIdentifier::FILE_SIZE));
    assert!(props.contains(&PropertyIdentifier::MODIFICATION_DATE));
    assert!(props.contains(&PropertyIdentifier::ARCHIVE));
    assert!(props.contains(&PropertyIdentifier::READ_ONLY));
    assert!(props.contains(&PropertyIdentifier::FILE_ACCESS_METHOD));
    assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
    assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
    assert!(props.contains(&PropertyIdentifier::RELIABILITY));
    // RECORD_COUNT should NOT be in property list for stream-access files
    assert!(!props.contains(&PropertyIdentifier::RECORD_COUNT));
}

#[test]
fn file_property_list_record_access() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    let props = file.property_list();
    assert!(props.contains(&PropertyIdentifier::RECORD_COUNT));
}

#[test]
fn file_unknown_property_error() {
    let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    let result = file.read_property(PropertyIdentifier::PRESENT_VALUE, None);
    assert!(result.is_err());
    if let Err(Error::Protocol { code, .. }) = result {
        assert_eq!(code, ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32);
    } else {
        panic!("expected Protocol error");
    }
}
