//! `FileStorage` (#397): the Clause 14 service data behind the File
//! object — read windows and End Of File, extension and append, the growth
//! caps, access-method refusal, and the `BACnetObject` storage hooks.

use super::*;
use bacnet_types::enums::ErrorCode;

fn protocol_pair(err: Error) -> (u32, u32) {
    match err {
        Error::Protocol { class, code } => (class, code),
        other => panic!("expected protocol error, got {other:?}"),
    }
}

fn file_full_pair() -> (u32, u32) {
    (
        ErrorClass::OBJECT.to_raw() as u32,
        ErrorCode::FILE_FULL.to_raw() as u32,
    )
}

fn invalid_method_pair() -> (u32, u32) {
    (
        ErrorClass::SERVICES.to_raw() as u32,
        ErrorCode::INVALID_FILE_ACCESS_METHOD.to_raw() as u32,
    )
}

fn invalid_start_pair() -> (u32, u32) {
    (
        ErrorClass::SERVICES.to_raw() as u32,
        ErrorCode::INVALID_FILE_START_POSITION.to_raw() as u32,
    )
}

fn access_denied_pair() -> (u32, u32) {
    (
        ErrorClass::SERVICES.to_raw() as u32,
        ErrorCode::FILE_ACCESS_DENIED.to_raw() as u32,
    )
}

fn stream_file() -> FileObject {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_data(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    file
}

fn record_file() -> FileObject {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    file.set_records(vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD], vec![0xEE]]);
    file
}

fn retained_datetime() -> (Date, Time) {
    (
        Date {
            year: 124,
            month: 2,
            day: 29,
            day_of_week: 4,
        },
        Time {
            hour: 12,
            minute: 34,
            second: 56,
            hundredths: 78,
        },
    )
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

#[derive(Debug, PartialEq)]
struct StreamState {
    data: Vec<u8>,
    file_size: u64,
    modification_date: (Date, Time),
    archive: bool,
}

fn stream_state(file: &FileObject) -> StreamState {
    StreamState {
        data: file.data.clone(),
        file_size: file.file_size,
        modification_date: file.modification_date,
        archive: file.archive,
    }
}

#[derive(Debug, PartialEq)]
struct RecordState {
    records: Vec<Vec<u8>>,
    file_size: u64,
    record_count: Option<u64>,
    modification_date: (Date, Time),
    archive: bool,
}

fn record_state(file: &FileObject) -> RecordState {
    RecordState {
        records: file.records.clone(),
        file_size: file.file_size,
        record_count: file.record_count,
        modification_date: file.modification_date,
        archive: file.archive,
    }
}

#[test]
fn storage_stream_write_then_read_round_trips() {
    let mut file = stream_file();
    assert_eq!(
        file.write_stream(FileWriteStart::At(2), &[0xAA, 0xBB])
            .unwrap(),
        2
    );
    let read = file.read_stream(2, 2).unwrap();
    assert_eq!(read.data, vec![0xAA, 0xBB]);
    assert!(!read.end_of_file);
    assert_eq!(file.file_size(), 8);
    assert_eq!(
        file.write_stream(FileWriteStart::Append, &[0x99]).unwrap(),
        8
    );
    assert_eq!(file.data(), &[1, 2, 0xAA, 0xBB, 5, 6, 7, 8, 0x99]);
    assert_eq!(file.file_size(), 9);
}

/// End Of File remains false for an empty window in a non-empty file, while
/// the built-in empty-file correction reports the only valid position as EOF.
#[test]
fn storage_stream_read_boundaries() {
    let file = stream_file();
    let at_end = file.read_stream(8, 4).unwrap();
    assert!(at_end.data.is_empty());
    assert!(
        !at_end.end_of_file,
        "an empty window includes no last octet"
    );
    let none = file.read_stream(2, 0).unwrap();
    assert!(none.data.is_empty());
    assert!(!none.end_of_file);
    let empty = FileObject::new(2, "EMPTY", "text/plain").unwrap();
    let nothing = empty.read_stream(0, 4).unwrap();
    assert!(nothing.data.is_empty());
    assert!(nothing.end_of_file);
    assert_eq!(
        protocol_pair(file.read_stream(9, 1).unwrap_err()),
        invalid_start_pair()
    );
    let short = file.read_stream(6, u64::MAX).unwrap();
    assert_eq!(short.data, vec![7, 8]);
    assert!(short.end_of_file);
    let last = file.read_stream(7, 1).unwrap();
    assert_eq!(last.data, vec![8]);
    assert!(last.end_of_file);
}

#[test]
fn storage_empty_record_file_reports_end_of_file() {
    let mut empty = FileObject::new(2, "EMPTY-RECORDS", "application/octet-stream").unwrap();
    empty.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());

    let nothing = empty.read_records(0, 4).unwrap();
    assert!(nothing.records.is_empty());
    assert!(nothing.end_of_file);
    assert_eq!(
        protocol_pair(empty.read_records(1, 1).unwrap_err()),
        invalid_start_pair()
    );
}

#[test]
fn storage_stream_growth_cap_is_file_full_and_leaves_data_unchanged() {
    let mut file = stream_file();
    file.set_max_file_size(4);
    assert_eq!(
        protocol_pair(
            file.write_stream(FileWriteStart::At(8), &[0x01])
                .unwrap_err()
        ),
        file_full_pair()
    );
    assert_eq!(file.data(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(file.file_size(), 8);
    // Preloaded contents past the cap are still writable in place: both
    // windows end beyond the 4-octet cap but within the 8 stored octets.
    file.write_stream(FileWriteStart::At(0), &[0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5])
        .unwrap();
    assert_eq!(file.data(), &[0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 7, 8]);
    file.write_stream(FileWriteStart::At(6), &[0xF6, 0xF7])
        .unwrap();
    assert_eq!(
        file.data(),
        &[0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7]
    );
    assert_eq!(file.file_size(), 8);
    assert_eq!(
        protocol_pair(
            file.write_stream(FileWriteStart::At(u64::MAX), &[0x01])
                .unwrap_err()
        ),
        file_full_pair(),
        "offset overflow is reported as FILE_FULL"
    );
}

#[test]
fn storage_record_write_then_read_round_trips() {
    let mut file = record_file();
    assert_eq!(
        file.write_records(FileWriteStart::At(1), &[vec![0x01, 0x02, 0x03]])
            .unwrap(),
        1
    );
    assert_eq!(file.records()[1], vec![0x01, 0x02, 0x03]);
    assert_eq!(file.file_size(), 6);
    assert_eq!(
        file.read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        file.write_records(FileWriteStart::Append, &[vec![0x77]])
            .unwrap(),
        3
    );
    assert_eq!(file.records().len(), 4);
    assert_eq!(file.file_size(), 7);
    let read = file.read_records(3, 5).unwrap();
    assert_eq!(read.records, vec![vec![0x77]]);
    assert!(read.end_of_file);
    assert_eq!(
        protocol_pair(file.read_records(5, 1).unwrap_err()),
        invalid_start_pair()
    );
}

#[test]
fn storage_record_caps_are_file_full_and_leave_records_unchanged() {
    let mut file = record_file();
    file.set_max_record_count(2);
    assert_eq!(
        protocol_pair(
            file.write_records(FileWriteStart::At(3), &[vec![0x01]])
                .unwrap_err()
        ),
        file_full_pair()
    );
    assert_eq!(file.records().len(), 3);
    assert_eq!(file.file_size(), 5);
    // In-place replacement of preloaded records is not growth.
    file.write_records(FileWriteStart::At(2), &[vec![0x0E, 0x0F]])
        .unwrap();
    assert_eq!(file.records()[2], vec![0x0E, 0x0F]);

    // Octet cap below the 5 preloaded octets: growth is refused, an
    // in-place replacement that keeps the total at 5 is not.
    let mut file = record_file();
    file.set_max_file_size(2);
    assert_eq!(
        protocol_pair(
            file.write_records(FileWriteStart::Append, &[vec![0x01]])
                .unwrap_err()
        ),
        file_full_pair(),
        "octet cap applies to record payloads"
    );
    assert_eq!(file.records().len(), 3);
    file.write_records(FileWriteStart::At(0), &[vec![0x10, 0x11]])
        .unwrap();
    assert_eq!(file.records()[0], vec![0x10, 0x11]);
    assert_eq!(file.file_size(), 5);
}

#[test]
fn file_metadata_storage_failures_preserve_payload_accounting_and_metadata() {
    let retained = retained_datetime();

    let mut stream = stream_file();
    stream.set_modification_date(retained.0, retained.1);
    stream.set_archive(true);
    let expected = stream_state(&stream);
    stream
        .write_records(FileWriteStart::At(0), &[vec![0x01]])
        .unwrap_err();
    assert_eq!(stream_state(&stream), expected, "stream access failure");

    let expected = stream_state(&stream);
    stream
        .write_stream(FileWriteStart::At(u64::MAX), &[0x01])
        .unwrap_err();
    assert_eq!(stream_state(&stream), expected, "stream start overflow");

    stream.set_max_file_size(7);
    let expected = stream_state(&stream);
    stream
        .write_stream(FileWriteStart::Append, &[0x01])
        .unwrap_err();
    assert_eq!(stream_state(&stream), expected, "stream growth cap");

    let mut record = record_file();
    record.set_modification_date(retained.0, retained.1);
    record.set_archive(true);
    let expected = record_state(&record);
    record
        .write_stream(FileWriteStart::At(0), &[0x01])
        .unwrap_err();
    assert_eq!(record_state(&record), expected, "record access failure");

    let expected = record_state(&record);
    record
        .write_records(FileWriteStart::At(u64::MAX), &[vec![0x01]])
        .unwrap_err();
    assert_eq!(record_state(&record), expected, "record start overflow");

    record.set_max_record_count(2);
    let expected = record_state(&record);
    record
        .write_records(FileWriteStart::Append, &[vec![0x01]])
        .unwrap_err();
    assert_eq!(record_state(&record), expected, "record count cap");

    let mut record = record_file();
    record.set_modification_date(retained.0, retained.1);
    record.set_archive(true);
    record.set_max_file_size(4);
    let expected = record_state(&record);
    record
        .write_records(FileWriteStart::Append, &[vec![0x01]])
        .unwrap_err();
    assert_eq!(record_state(&record), expected, "record octet cap");
}

#[test]
fn storage_refuses_the_other_access_method() {
    let mut stream = stream_file();
    assert_eq!(
        protocol_pair(stream.read_records(0, 1).unwrap_err()),
        invalid_method_pair()
    );
    assert_eq!(
        protocol_pair(
            stream
                .write_records(FileWriteStart::At(0), &[vec![1]])
                .unwrap_err()
        ),
        invalid_method_pair()
    );
    let mut record = record_file();
    assert_eq!(
        protocol_pair(record.read_stream(0, 1).unwrap_err()),
        invalid_method_pair()
    );
    assert_eq!(
        protocol_pair(
            record
                .write_stream(FileWriteStart::At(0), &[1])
                .unwrap_err()
        ),
        invalid_method_pair()
    );
    assert_eq!(stream.data(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(record.records().len(), 3);
}

#[test]
fn storage_caps_default_and_clamp_to_integer_range() {
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    assert_eq!(file.max_file_size(), DEFAULT_MAX_FILE_SIZE);
    assert_eq!(file.max_record_count(), DEFAULT_MAX_RECORD_COUNT);
    file.set_max_file_size(u64::MAX);
    file.set_max_record_count(u64::MAX);
    assert_eq!(file.max_file_size(), i32::MAX as u64);
    assert_eq!(
        file.max_record_count(),
        DEFAULT_MAX_RECORD_COUNT,
        "the record cap never exceeds the decoder ceiling"
    );
    file.set_max_record_count(5);
    assert_eq!(file.max_record_count(), 5);
}

/// A storage that implements only the stream write: the other methods
/// report the file as not accessible, never as a method mismatch.
struct WriteOnlyStream;

impl FileStorage for WriteOnlyStream {
    fn write_stream(&mut self, _start: FileWriteStart, data: &[u8]) -> Result<u64, Error> {
        Ok(data.len() as u64)
    }
}

#[test]
fn storage_default_methods_are_file_access_denied() {
    let mut storage = WriteOnlyStream;
    assert_eq!(
        storage
            .write_stream(FileWriteStart::At(0), &[1, 2])
            .unwrap(),
        2
    );
    assert_eq!(
        protocol_pair(storage.read_stream(0, 1).unwrap_err()),
        access_denied_pair()
    );
    assert_eq!(
        protocol_pair(storage.read_records(0, 1).unwrap_err()),
        access_denied_pair()
    );
    assert_eq!(
        protocol_pair(
            storage
                .write_records(FileWriteStart::Append, &[vec![1]])
                .unwrap_err()
        ),
        access_denied_pair()
    );
}

/// Table 12-16 footnote 2: Record_Count "shall be present only if
/// File_Access_Method is RECORD_ACCESS", and File_Size counts the octets of
/// the channel in use — whichever order the payload and the method are set.
#[test]
fn payload_setters_follow_the_access_method() {
    let mut file = stream_file();
    file.set_records(vec![vec![1, 2, 3], vec![4, 5]]);
    assert!(!file
        .property_list()
        .contains(&PropertyIdentifier::RECORD_COUNT));
    assert!(file
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .is_err());
    assert_eq!(file.file_size(), 8, "File_Size stays with the stream data");
    assert_eq!(
        file.read_stream(0, 8).unwrap().data,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );

    file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    assert_eq!(
        file.read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(file.file_size(), 5, "File_Size now counts the records");

    file.set_data(vec![9; 20]);
    assert_eq!(
        file.file_size(),
        5,
        "stream data is inert under RECORD_ACCESS"
    );
    file.set_file_access_method(FileAccessMethod::STREAM_ACCESS.to_raw());
    assert_eq!(file.file_size(), 20);
    assert!(file
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .is_err());

    // An unrecognised raw value is the stream channel for both setters, as
    // it is for set_file_access_method's own recomputation.
    file.set_file_access_method(99);
    file.set_data(vec![7; 3]);
    assert_eq!(
        file.file_size(),
        3,
        "File_Size tracks set_data under raw 99"
    );
    file.set_records(vec![vec![1], vec![2]]);
    assert_eq!(file.file_size(), 3, "records are inert under raw 99");
    assert!(file
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .is_err());
}

struct NoStorageFile;

impl BACnetObject for NoStorageFile {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::FILE, 7).unwrap()
    }

    fn object_name(&self) -> &str {
        "NO-STORAGE"
    }

    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Err(common::unknown_property_error())
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

#[test]
fn storage_hooks_default_to_none_and_file_object_opts_in() {
    let mut none = NoStorageFile;
    assert!(none.file_configuration_internal().is_none());
    assert!(none.file_configuration_internal_mut().is_none());
    assert!(none.file_storage_internal().is_none());
    assert!(none.file_storage_internal_mut().is_none());
    let mut file = stream_file();
    assert!(file.file_configuration_internal().is_some());
    assert!(file.file_configuration_internal_mut().is_some());
    assert!(file.file_storage_internal().is_some());
    assert!(file.file_storage_internal_mut().is_some());
    // 65 was the fabricated "File Data" property the old handler read (it
    // is Max_Pres_Value); Table 12-16 defines no such property.
    assert!(!file
        .property_list()
        .iter()
        .any(|p| *p == PropertyIdentifier::from_raw(65)));
}

#[test]
fn configuration_capability_enforces_shape_and_routes_payload_metadata() {
    let retained = retained_datetime();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_read_only(true);
    file.set_modification_date(retained.0, retained.1);
    file.set_archive(true);

    {
        let configuration = file.file_configuration_internal_mut().unwrap();
        configuration.set_stream_data(vec![1, 2, 3]).unwrap();
        assert_eq!(configuration.stream_data().unwrap(), &[1, 2, 3]);
    }
    assert_eq!(file.file_size(), 3);
    assert!(file.read_only(), "trusted preload does not clear Read_Only");
    assert_eq!(file.modification_date, unspecified_datetime());
    assert!(!file.archive());

    file.set_modification_date(retained.0, retained.1);
    file.set_archive(true);
    let expected = stream_state(&file);
    let err = file
        .file_configuration_internal_mut()
        .unwrap()
        .set_record_data(vec![vec![9]])
        .unwrap_err();
    assert_eq!(protocol_pair(err), invalid_method_pair());
    assert_eq!(stream_state(&file), expected);

    file.file_configuration_internal_mut()
        .unwrap()
        .set_access_method(FileAccessMethod::RECORD_ACCESS);
    file.set_modification_date(retained.0, retained.1);
    file.set_archive(true);
    {
        let configuration = file.file_configuration_internal_mut().unwrap();
        configuration
            .set_record_data(vec![vec![0xAA], vec![0xBB, 0xCC]])
            .unwrap();
        assert_eq!(
            configuration.record_data().unwrap(),
            &[vec![0xAA], vec![0xBB, 0xCC]]
        );
    }
    assert_eq!(file.file_size(), 3);
    assert_eq!(
        file.read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(file.modification_date, unspecified_datetime());
    assert!(!file.archive());

    file.set_modification_date(retained.0, retained.1);
    file.set_archive(true);
    let expected = record_state(&file);
    let err = file
        .file_configuration_internal_mut()
        .unwrap()
        .set_stream_data(vec![7, 8])
        .unwrap_err();
    assert_eq!(protocol_pair(err), invalid_method_pair());
    assert_eq!(record_state(&file), expected);
}

#[test]
fn configuration_capability_caps_are_effective_growth_limits_not_preload_limits() {
    let mut stream = stream_file();
    {
        let configuration = stream.file_configuration_internal_mut().unwrap();
        assert_eq!(configuration.set_max_file_size(u64::MAX), i32::MAX as u64);
        assert_eq!(configuration.set_max_file_size(4), 4);
    }
    let expected = stream_state(&stream);
    assert_eq!(
        protocol_pair(
            stream
                .write_stream(FileWriteStart::Append, &[0x99])
                .unwrap_err()
        ),
        file_full_pair()
    );
    assert_eq!(stream_state(&stream), expected);

    let mut record = record_file();
    {
        let configuration = record.file_configuration_internal_mut().unwrap();
        assert_eq!(
            configuration.set_max_record_count(u64::MAX),
            DEFAULT_MAX_RECORD_COUNT
        );
        assert_eq!(configuration.set_max_record_count(2), 2);
    }
    let expected = record_state(&record);
    assert_eq!(
        protocol_pair(
            record
                .write_records(FileWriteStart::Append, &[vec![0x99]])
                .unwrap_err()
        ),
        file_full_pair()
    );
    assert_eq!(record_state(&record), expected);
}
