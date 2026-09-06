//! AtomicReadFile / AtomicWriteFile persistence (#397): reads return the
//! File object's stored octets and records, writes persist them, and the
//! ACK reports where the data actually landed.
//!
//! Clause 14.1 Service Procedure: a start position or record that is less
//! than 0 or exceeds the file size is refused with SERVICES /
//! INVALID_FILE_START_POSITION; a short read returns what remains, and End
//! Of File is TRUE only when the window includes the last octet or record
//! (Clause 14.1.3.1: "TRUE if this response includes the last octet of the
//! file and FALSE otherwise"). Clause 14.2 Service Procedure: a start beyond
//! the file extends it (intervening contents are a local matter — zero
//! octets or empty records here), -1 appends, and the ACK carries the
//! resolved position (Annex F shows 14, not -1). A write the object cannot
//! hold is OBJECT / FILE_FULL (Clause 18: "filled to a designed limit").
//!
//! Every test here uses only the object surface that exists before the
//! storage hook landed (`set_data`, `set_read_only`, `read_property`, the
//! two handlers) plus
//! the `pub(super)` helpers of `file_access_method`, so the module compiles
//! against the pre-fix tree, where every test that pins new behaviour
//! fails on an assertion rather than a compile error. One test is a
//! regression pin that already held there:
//! `non_file_identifier_and_unknown_object_precede_start_position_gate`
//! (gate order from #398).

use super::file_access_method::{
    assert_file_access_denied, assert_invalid_access, file_oid, read_wire_for, record_file_db,
    stream_file_db, write_wire_for, SENTINEL,
};
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::file::FileObject;
use bacnet_services::file::{
    AtomicReadFileAck, AtomicWriteFileAck, FileAccessMethod, FileReadAckMethod,
    FileWriteAccessMethod, FileWriteAckMethod,
};

pub(super) fn assert_protocol_error(
    result: Result<(), Error>,
    class: ErrorClass,
    code: ErrorCode,
    context: &str,
) {
    match result.expect_err(&format!("{context}: request must be refused")) {
        Error::Protocol {
            class: got_class,
            code: got_code,
        } => {
            assert_eq!(
                got_class,
                class.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                got_code,
                code.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected {class:?}/{code:?}, got {other:?}"),
    }
}

fn assert_invalid_start(result: Result<(), Error>, context: &str) {
    assert_protocol_error(
        result,
        ErrorClass::SERVICES,
        ErrorCode::INVALID_FILE_START_POSITION,
        context,
    );
}

pub(super) fn read_stream(
    db: &ObjectDatabase,
    start: i32,
    count: u32,
) -> Result<(Vec<u8>, bool), Error> {
    let mut buf = BytesMut::new();
    handle_atomic_read_file(
        db,
        &read_wire_for(
            file_oid(),
            FileAccessMethod::Stream {
                file_start_position: start,
                requested_octet_count: count,
            },
        ),
        &mut buf,
    )?;
    let ack = AtomicReadFileAck::decode(&buf).unwrap();
    match ack.access {
        FileReadAckMethod::Stream {
            file_start_position,
            file_data,
        } => {
            assert_eq!(file_start_position, start, "read ACK must echo the start");
            Ok((file_data, ack.end_of_file))
        }
        other => panic!("expected stream ACK, got {other:?}"),
    }
}

pub(super) fn read_records(
    db: &ObjectDatabase,
    start: i32,
    count: u32,
) -> Result<(Vec<Vec<u8>>, bool), Error> {
    let mut buf = BytesMut::new();
    handle_atomic_read_file(
        db,
        &read_wire_for(
            file_oid(),
            FileAccessMethod::Record {
                file_start_record: start,
                requested_record_count: count,
            },
        ),
        &mut buf,
    )?;
    let ack = AtomicReadFileAck::decode(&buf).unwrap();
    match ack.access {
        FileReadAckMethod::Record {
            file_start_record,
            returned_record_count,
            file_record_data,
        } => {
            assert_eq!(file_start_record, start, "read ACK must echo the start");
            assert_eq!(
                returned_record_count as usize,
                file_record_data.len(),
                "Returned Record Count must match the payload list"
            );
            Ok((file_record_data, ack.end_of_file))
        }
        other => panic!("expected record ACK, got {other:?}"),
    }
}

/// Stream write through the handler; returns the ACK's resolved position.
pub(super) fn write_stream(db: &mut ObjectDatabase, start: i32, data: &[u8]) -> Result<i32, Error> {
    let mut buf = BytesMut::new();
    handle_atomic_write_file(
        db,
        &write_wire_for(
            file_oid(),
            FileWriteAccessMethod::Stream {
                file_start_position: start,
                file_data: data.to_vec(),
            },
        ),
        &mut buf,
    )?;
    match AtomicWriteFileAck::decode(&buf).unwrap().access {
        FileWriteAckMethod::Stream {
            file_start_position,
        } => Ok(file_start_position),
        other => panic!("expected stream write ACK, got {other:?}"),
    }
}

/// Record write through the handler; returns the ACK's resolved record.
pub(super) fn write_records(
    db: &mut ObjectDatabase,
    start: i32,
    records: &[Vec<u8>],
) -> Result<i32, Error> {
    let mut buf = BytesMut::new();
    handle_atomic_write_file(
        db,
        &write_wire_for(
            file_oid(),
            FileWriteAccessMethod::Record {
                file_start_record: start,
                record_count: records.len() as u32,
                file_record_data: records.to_vec(),
            },
        ),
        &mut buf,
    )?;
    match AtomicWriteFileAck::decode(&buf).unwrap().access {
        FileWriteAckMethod::Record { file_start_record } => Ok(file_start_record),
        other => panic!("expected record write ACK, got {other:?}"),
    }
}

fn unsigned_property(db: &ObjectDatabase, property: PropertyIdentifier) -> u64 {
    match db.get(&file_oid()).unwrap().read_property(property, None) {
        Ok(PropertyValue::Unsigned(n)) => n,
        other => panic!("expected Unsigned {property:?}, got {other:?}"),
    }
}

pub(super) fn file_size(db: &ObjectDatabase) -> u64 {
    unsigned_property(db, PropertyIdentifier::FILE_SIZE)
}

pub(super) fn record_count(db: &ObjectDatabase) -> u64 {
    unsigned_property(db, PropertyIdentifier::RECORD_COUNT)
}

// ──────────────────────────────────────────────────────────────────────────
// Stream access
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn stream_write_then_read_returns_the_written_octets() {
    let mut db = stream_file_db();
    let pos = write_stream(&mut db, 2, &[0xAA, 0xBB, 0xCC]).expect("stream write must succeed");
    assert_eq!(pos, 2);
    let (data, _) = read_stream(&db, 2, 3).unwrap();
    assert_eq!(data, vec![0xAA, 0xBB, 0xCC]);
    let (whole, eof) = read_stream(&db, 0, 8).unwrap();
    assert_eq!(whole, vec![1, 2, 0xAA, 0xBB, 0xCC, 6, 7, 8]);
    assert!(eof);
}

#[test]
fn preloaded_stream_read_returns_stored_octets() {
    let db = stream_file_db();
    let (data, eof) = read_stream(&db, 2, 4).unwrap();
    assert_eq!(data, vec![3, 4, 5, 6]);
    assert!(!eof, "octets 7 and 8 remain");
    let (data, eof) = read_stream(&db, 4, 4).unwrap();
    assert_eq!(data, vec![5, 6, 7, 8]);
    assert!(eof, "the window ends at the last octet");
    let (data, eof) = read_stream(&db, 6, 10).unwrap();
    assert_eq!(data, vec![7, 8], "short read returns what remains");
    assert!(eof);
}

#[test]
fn stream_write_extending_zero_fills_and_updates_file_size() {
    let mut db = stream_file_db();
    write_stream(&mut db, 12, &[0x77]).expect("extending write must succeed");
    assert_eq!(file_size(&db), 13);
    let (data, eof) = read_stream(&db, 8, 5).unwrap();
    assert_eq!(data, vec![0, 0, 0, 0, 0x77]);
    assert!(eof);
}

#[test]
fn stream_write_in_place_keeps_file_size() {
    let mut db = stream_file_db();
    write_stream(&mut db, 0, &[0xF0, 0xF1]).expect("in-place write must succeed");
    assert_eq!(file_size(&db), 8);
    let (data, eof) = read_stream(&db, 0, 8).unwrap();
    assert_eq!(data, vec![0xF0, 0xF1, 3, 4, 5, 6, 7, 8]);
    assert!(eof);
}

#[test]
fn stream_append_writes_at_end_and_ack_reports_resolved_position() {
    let mut db = stream_file_db();
    let pos = write_stream(&mut db, -1, &[0x99]).expect("append must succeed");
    assert_eq!(pos, 8, "ACK must carry the resolved position, not -1");
    assert_eq!(file_size(&db), 9);
    let (data, eof) = read_stream(&db, 8, 1).unwrap();
    assert_eq!(data, vec![0x99]);
    assert!(eof);
    let (head, _) = read_stream(&db, 0, 1).unwrap();
    assert_eq!(head, vec![1], "append must not touch the head of the file");
}

/// The pre-fix handler answered TRUE here from `start + count >= size`; the
/// window includes no octet, so Clause 14.1.3.1 gives FALSE.
#[test]
fn stream_read_at_end_returns_empty_without_end_of_file() {
    let db = stream_file_db();
    let (data, eof) = read_stream(&db, 8, 4).unwrap();
    assert!(data.is_empty());
    assert!(!eof, "an empty window includes no last octet");
    let (data, eof) = read_stream(&db, 2, 0).unwrap();
    assert!(data.is_empty());
    assert!(!eof, "a zero-count read mid-file is not at end of file");
}

#[test]
fn stream_read_beyond_end_is_invalid_file_start_position() {
    let db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            file_oid(),
            FileAccessMethod::Stream {
                file_start_position: 9,
                requested_octet_count: 1,
            },
        ),
        &mut buf,
    );
    assert_invalid_start(result, "stream read at 9 of an 8-octet file");
    assert_eq!(&buf[..], SENTINEL, "refused read must not touch the buffer");
}

#[test]
fn stream_read_negative_start_is_invalid_file_start_position() {
    let db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            file_oid(),
            FileAccessMethod::Stream {
                file_start_position: -1,
                requested_octet_count: 4,
            },
        ),
        &mut buf,
    );
    assert_invalid_start(result, "stream read at -1");
    assert_eq!(&buf[..], SENTINEL, "refused read must not touch the buffer");
}

#[test]
fn stream_write_negative_start_other_than_minus_one_is_refused() {
    let mut db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            file_oid(),
            FileWriteAccessMethod::Stream {
                file_start_position: -5,
                file_data: vec![0x01],
            },
        ),
        &mut buf,
    );
    assert_invalid_start(result, "stream write at -5");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the buffer"
    );
    assert_eq!(file_size(&db), 8);
    assert_eq!(
        read_stream(&db, 0, 8).unwrap().0,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[test]
fn huge_stream_start_position_is_file_full_not_a_giant_allocation() {
    let mut db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            file_oid(),
            FileWriteAccessMethod::Stream {
                file_start_position: i32::MAX,
                file_data: vec![0x01],
            },
        ),
        &mut buf,
    );
    assert_protocol_error(
        result,
        ErrorClass::OBJECT,
        ErrorCode::FILE_FULL,
        "stream write at i32::MAX",
    );
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the buffer"
    );
    assert_eq!(file_size(&db), 8);
}

// ──────────────────────────────────────────────────────────────────────────
// Record access
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn record_write_then_read_returns_the_written_records() {
    let mut db = record_file_db();
    let pos = write_records(&mut db, 0, &[vec![0x11], vec![0x22, 0x33]])
        .expect("record write must succeed");
    assert_eq!(pos, 0);
    let (records, eof) = read_records(&db, 0, 2).unwrap();
    assert_eq!(records, vec![vec![0x11], vec![0x22, 0x33]]);
    assert!(!eof, "record 2 remains");
    assert_eq!(record_count(&db), 3);
}

#[test]
fn preloaded_record_read_returns_stored_records() {
    let db = record_file_db();
    let (records, eof) = read_records(&db, 1, 2).unwrap();
    assert_eq!(records, vec![vec![0xCC, 0xDD], vec![0xEE]]);
    assert!(eof);
    let (records, eof) = read_records(&db, 2, 5).unwrap();
    assert_eq!(records, vec![vec![0xEE]], "short read returns what remains");
    assert!(eof);
}

#[test]
fn record_write_replacing_in_place_keeps_record_count_and_updates_file_size() {
    let mut db = record_file_db();
    write_records(&mut db, 1, &[vec![0x01, 0x02, 0x03]]).expect("record write must succeed");
    assert_eq!(record_count(&db), 3);
    assert_eq!(file_size(&db), 2 + 3 + 1, "File_Size is the octet total");
    let (records, eof) = read_records(&db, 0, 3).unwrap();
    assert_eq!(
        records,
        vec![vec![0xAA, 0xBB], vec![0x01, 0x02, 0x03], vec![0xEE]]
    );
    assert!(eof);
}

#[test]
fn record_write_beyond_end_extends_with_empty_records() {
    let mut db = record_file_db();
    let pos = write_records(&mut db, 5, &[vec![0x55, 0x66]]).expect("extending write must succeed");
    assert_eq!(pos, 5);
    assert_eq!(record_count(&db), 6);
    assert_eq!(file_size(&db), 5 + 2);
    let (records, eof) = read_records(&db, 3, 2).unwrap();
    assert_eq!(records, vec![Vec::<u8>::new(), Vec::<u8>::new()]);
    assert!(!eof);
    let (records, eof) = read_records(&db, 5, 1).unwrap();
    assert_eq!(records, vec![vec![0x55, 0x66]]);
    assert!(eof);
}

#[test]
fn record_append_writes_at_end_and_ack_reports_resolved_position() {
    let mut db = record_file_db();
    let pos = write_records(&mut db, -1, &[vec![0x77]]).expect("append must succeed");
    assert_eq!(pos, 3, "ACK must carry the resolved record, not -1");
    assert_eq!(record_count(&db), 4);
    let (records, eof) = read_records(&db, 3, 1).unwrap();
    assert_eq!(records, vec![vec![0x77]]);
    assert!(eof);
    assert_eq!(
        read_records(&db, 0, 1).unwrap().0,
        vec![vec![0xAA, 0xBB]],
        "append must not touch record 0"
    );
}

#[test]
fn record_read_beyond_end_is_invalid_file_start_position() {
    let db = record_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            file_oid(),
            FileAccessMethod::Record {
                file_start_record: 4,
                requested_record_count: 1,
            },
        ),
        &mut buf,
    );
    assert_invalid_start(result, "record read at 4 of a 3-record file");
    assert_eq!(&buf[..], SENTINEL, "refused read must not touch the buffer");

    let (records, eof) = read_records(&db, 3, 1).unwrap();
    assert!(records.is_empty(), "reading at the end is legal and empty");
    assert!(!eof, "an empty window includes no last record");
}

#[test]
fn record_read_negative_start_is_invalid_file_start_position() {
    let db = record_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            file_oid(),
            FileAccessMethod::Record {
                file_start_record: -1,
                requested_record_count: 4,
            },
        ),
        &mut buf,
    );
    assert_invalid_start(result, "record read at -1");
    assert_eq!(&buf[..], SENTINEL, "refused read must not touch the buffer");
}

#[test]
fn record_write_negative_start_other_than_minus_one_is_refused() {
    let mut db = record_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            file_oid(),
            FileWriteAccessMethod::Record {
                file_start_record: -5,
                record_count: 1,
                file_record_data: vec![vec![0x01]],
            },
        ),
        &mut buf,
    );
    assert_invalid_start(result, "record write at -5");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the buffer"
    );
    assert_eq!(record_count(&db), 3);
    assert_eq!(
        read_records(&db, 0, 3).unwrap().0,
        vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD], vec![0xEE]]
    );
}

/// Pins decoder rejection through the handler before lookup or mutation.
#[test]
fn record_write_with_short_payload_list_is_rejected_without_mutation() {
    use bacnet_encoding::{primitives, tags};

    let mut db = record_file_db();
    let mut wire = BytesMut::new();
    primitives::encode_app_object_id(&mut wire, &file_oid());
    tags::encode_opening_tag(&mut wire, 1);
    primitives::encode_app_signed(&mut wire, 0);
    primitives::encode_app_unsigned(&mut wire, 2);
    primitives::encode_app_octet_string(&mut wire, &[0x01]);
    tags::encode_closing_tag(&mut wire, 1);

    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(&mut db, &wire, &mut buf);
    match result.expect_err("short record list must be refused") {
        Error::Reject { reason } => assert_eq!(
            reason,
            RejectReason::MISSING_REQUIRED_PARAMETER.to_raw(),
            "wrong reject reason"
        ),
        other => panic!("expected Reject/MISSING_REQUIRED_PARAMETER, got {other:?}"),
    }
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the buffer"
    );
    assert_eq!(record_count(&db), 3);
    assert_eq!(
        read_records(&db, 0, 3).unwrap().0,
        vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD], vec![0xEE]]
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Gate order — the existing refusals still run before any storage access.
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn read_only_and_access_method_gates_still_precede_mutation() {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_data(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    file.set_read_only(true);
    db.add(Box::new(file)).unwrap();

    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            file_oid(),
            FileWriteAccessMethod::Stream {
                file_start_position: 0,
                file_data: vec![0xAA, 0xBB],
            },
        ),
        &mut buf,
    );
    assert_file_access_denied(result, "valid write to a read-only file");
    assert_eq!(&buf[..], SENTINEL);
    assert_eq!(
        read_stream(&db, 0, 8).unwrap().0,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );

    let mut db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            file_oid(),
            FileWriteAccessMethod::Record {
                file_start_record: 0,
                record_count: 1,
                file_record_data: vec![vec![0xAA]],
            },
        ),
        &mut buf,
    );
    assert_invalid_access(result, "record write to a STREAM_ACCESS file");
    assert_eq!(&buf[..], SENTINEL);
    assert_eq!(file_size(&db), 8);
    assert_eq!(
        read_stream(&db, 0, 8).unwrap().0,
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );

    let mut db = record_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            file_oid(),
            FileWriteAccessMethod::Stream {
                file_start_position: 0,
                file_data: vec![0xAA],
            },
        ),
        &mut buf,
    );
    assert_invalid_access(result, "stream write to a RECORD_ACCESS file");
    assert_eq!(&buf[..], SENTINEL);
    assert_eq!(record_count(&db), 3);
    assert_eq!(
        read_records(&db, 0, 3).unwrap().0,
        vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD], vec![0xEE]]
    );
}

#[test]
fn non_file_identifier_and_unknown_object_precede_start_position_gate() {
    let mut db = stream_file_db();
    db.add(Box::new(AnalogInputObject::new(1, "AI-1", 62).unwrap()))
        .unwrap();
    let out_of_range = FileAccessMethod::Stream {
        file_start_position: 999,
        requested_octet_count: 1,
    };

    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            out_of_range.clone(),
        ),
        &mut buf,
    );
    assert_protocol_error(
        result,
        ErrorClass::SERVICES,
        ErrorCode::INCONSISTENT_OBJECT_TYPE,
        "non-File identifier with an out-of-range start",
    );
    assert_eq!(&buf[..], SENTINEL);

    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            ObjectIdentifier::new(ObjectType::FILE, 99).unwrap(),
            out_of_range,
        ),
        &mut buf,
    );
    assert_protocol_error(
        result,
        ErrorClass::OBJECT,
        ErrorCode::UNKNOWN_OBJECT,
        "absent File with an out-of-range start",
    );
    assert_eq!(&buf[..], SENTINEL);
}
