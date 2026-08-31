//! File Modification_Date and Archive behavior for built-in AtomicWriteFile.

use super::file_access_method::{file_oid, record_file_db, stream_file_db};
use super::file_persistence::{read_records, read_stream, write_records, write_stream};
use super::*;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::file::FileObject;
use bacnet_types::primitives::{Date, Time};
use std::sync::Arc;

struct FixedClock(ClockFrame);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(self.0)
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

fn file_metadata(
    db: &ObjectDatabase,
) -> (
    PropertyValue,
    PropertyValue,
    PropertyValue,
    Option<PropertyValue>,
) {
    let object = db.get(&file_oid()).unwrap();
    (
        object
            .read_property(PropertyIdentifier::MODIFICATION_DATE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::ARCHIVE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::FILE_SIZE, None)
            .unwrap(),
        object
            .read_property(PropertyIdentifier::RECORD_COUNT, None)
            .ok(),
    )
}

fn arm_refusal_detection(db: &mut ObjectDatabase, frame: ClockFrame) {
    db.set_clock_reader(Some(Arc::new(FixedClock(frame))));
    db.get_mut(&file_oid())
        .unwrap()
        .write_property(
            PropertyIdentifier::ARCHIVE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
}

#[test]
fn atomic_write_file_metadata_uses_clock_bound_before_object_add() {
    let old = clock_frame(1);
    let written = clock_frame(8);
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(Arc::new(FixedClock(written))));

    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_data(vec![1, 2, 3]);
    file.set_modification_date(old.local_date, old.local_time);
    file.set_archive(true);
    db.add(Box::new(file)).unwrap();
    assert_eq!(
        file_metadata(&db).0,
        PropertyValue::List(vec![
            PropertyValue::Date(old.local_date),
            PropertyValue::Time(old.local_time),
        ]),
        "adding an object to a clocked database must not timestamp it"
    );

    write_stream(&mut db, 0, &[0xAA]).unwrap();
    let metadata = file_metadata(&db);
    assert_eq!(
        metadata.0,
        PropertyValue::List(vec![
            PropertyValue::Date(written.local_date),
            PropertyValue::Time(written.local_time),
        ])
    );
    assert_eq!(metadata.1, PropertyValue::Boolean(false));
}

#[test]
fn atomic_write_file_metadata_uses_clock_rebound_after_object_add() {
    let old = clock_frame(1);
    let first = clock_frame(8);
    let rebound = clock_frame(9);
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw());
    file.set_records(vec![vec![0xAA], vec![0xBB]]);
    file.set_modification_date(old.local_date, old.local_time);
    file.set_archive(true);
    db.add(Box::new(file)).unwrap();

    let retained = file_metadata(&db);
    db.set_clock_reader(Some(Arc::new(FixedClock(first))));
    assert_eq!(file_metadata(&db), retained);
    db.set_clock_reader(Some(Arc::new(FixedClock(rebound))));
    assert_eq!(
        file_metadata(&db),
        retained,
        "clock rebinding must not mutate File metadata"
    );

    write_records(&mut db, 0, &[vec![0xCC]]).unwrap();
    let metadata = file_metadata(&db);
    assert_eq!(
        metadata.0,
        PropertyValue::List(vec![
            PropertyValue::Date(rebound.local_date),
            PropertyValue::Time(rebound.local_time),
        ])
    );
    assert_eq!(metadata.1, PropertyValue::Boolean(false));
}

#[test]
fn atomic_write_file_metadata_refusals_are_neutral() {
    let frame = clock_frame(10);

    let mut read_only = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_data(vec![1, 2, 3]);
    file.set_read_only(true);
    read_only.add(Box::new(file)).unwrap();
    arm_refusal_detection(&mut read_only, frame);
    let retained = file_metadata(&read_only);
    assert!(write_stream(&mut read_only, 0, &[0xAA]).is_err());
    assert_eq!(file_metadata(&read_only), retained, "read-only refusal");
    assert_eq!(read_stream(&read_only, 0, 3).unwrap().0, vec![1, 2, 3]);

    let mut stream = stream_file_db();
    arm_refusal_detection(&mut stream, frame);
    let retained = file_metadata(&stream);
    assert!(write_records(&mut stream, 0, &[vec![0xAA]]).is_err());
    assert_eq!(file_metadata(&stream), retained, "access-method refusal");
    assert!(write_stream(&mut stream, -5, &[0xAA]).is_err());
    assert_eq!(file_metadata(&stream), retained, "invalid stream start");
    assert!(write_stream(&mut stream, i32::MAX, &[0xAA]).is_err());
    assert_eq!(file_metadata(&stream), retained, "stream growth cap");
    assert_eq!(
        read_stream(&stream, 0, 8).unwrap().0,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );

    let mut record = record_file_db();
    arm_refusal_detection(&mut record, frame);
    let retained = file_metadata(&record);
    assert!(write_stream(&mut record, 0, &[0xAA]).is_err());
    assert_eq!(file_metadata(&record), retained, "record access refusal");
    assert!(write_records(&mut record, -5, &[vec![0xAA]]).is_err());
    assert_eq!(file_metadata(&record), retained, "invalid record start");
    assert_eq!(
        read_records(&record, 0, 3).unwrap().0,
        vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD], vec![0xEE]]
    );

    let mut capped = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw());
    file.set_records(vec![vec![0xAA], vec![0xBB], vec![0xCC]]);
    file.set_max_record_count(3);
    capped.add(Box::new(file)).unwrap();
    arm_refusal_detection(&mut capped, frame);
    let retained = file_metadata(&capped);
    assert!(write_records(&mut capped, -1, &[vec![0xDD]]).is_err());
    assert_eq!(file_metadata(&capped), retained, "record growth cap");
    assert_eq!(
        read_records(&capped, 0, 3).unwrap().0,
        vec![vec![0xAA], vec![0xBB], vec![0xCC]]
    );
}
