//! Access-method cross-checking for AtomicReadFile / AtomicWriteFile
//! (#287): a request whose access method differs from the File object's
//! declared File_Access_Method (Clause 12.13) must be refused with
//! SERVICES / INVALID_FILE_ACCESS_METHOD (Clauses 14.1, 14.2, 18) before
//! any ACK is encoded or any object state is mutated.
//!
//! The request CHOICE tags (stream [0], record [1]) are deliberately not
//! compared against the property's BACnetFileAccessMethod enumeration
//! (record-access = 0, stream-access = 1); the mapping here is semantic.
//!
//! Non-File identifier classification (#398): the Clause 14.1.4.1 and
//! 14.2.4.1 error tables pair "A non-File Object Identifier was provided"
//! with SERVICES / INCONSISTENT_OBJECT_TYPE. The standard does not sequence
//! that check against "The File object does not exist"; the handlers
//! classify the identifier by type before the object lookup, so an absent
//! non-File identifier gets the type error and only a missing File object
//! gets OBJECT / UNKNOWN_OBJECT.

use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::file::FileObject;
use bacnet_services::file::{
    AtomicReadFileAck, AtomicReadFileRequest, AtomicWriteFileAck, AtomicWriteFileRequest,
    FileAccessMethod, FileReadAckMethod, FileWriteAccessMethod,
};
use bacnet_types::enums::FileAccessMethod as ObjectFileAccessMethod;

pub(super) const SENTINEL: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];

pub(super) fn stream_file_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_data(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    db.add(Box::new(file)).unwrap();
    db
}

pub(super) fn record_file_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(ObjectFileAccessMethod::RECORD_ACCESS.to_raw());
    file.set_records(vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD], vec![0xEE]]);
    db.add(Box::new(file)).unwrap();
    db
}

pub(super) fn file_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::FILE, 1).unwrap()
}

pub(super) fn read_wire_for(oid: ObjectIdentifier, access: FileAccessMethod) -> Vec<u8> {
    let request = AtomicReadFileRequest {
        file_identifier: oid,
        access,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    buf.to_vec()
}

pub(super) fn write_wire_for(oid: ObjectIdentifier, access: FileWriteAccessMethod) -> Vec<u8> {
    let request = AtomicWriteFileRequest {
        file_identifier: oid,
        access,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    buf.to_vec()
}

fn read_wire(access: FileAccessMethod) -> Vec<u8> {
    read_wire_for(file_oid(), access)
}

fn write_wire(access: FileWriteAccessMethod) -> Vec<u8> {
    write_wire_for(file_oid(), access)
}

pub(super) fn assert_invalid_access(result: Result<(), Error>, context: &str) {
    match result.expect_err(&format!("{context}: request must be refused")) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::SERVICES.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                ErrorCode::INVALID_FILE_ACCESS_METHOD.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected SERVICES/INVALID_FILE_ACCESS_METHOD, got {other:?}"),
    }
}

pub(super) fn assert_file_access_denied(result: Result<(), Error>, context: &str) {
    match result.expect_err(&format!("{context}: request must be refused")) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::SERVICES.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                ErrorCode::FILE_ACCESS_DENIED.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected SERVICES/FILE_ACCESS_DENIED, got {other:?}"),
    }
}

fn assert_inconsistent_object_type(result: Result<(), Error>, context: &str) {
    match result.expect_err(&format!("{context}: request must be refused")) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::SERVICES.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                ErrorCode::INCONSISTENT_OBJECT_TYPE.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected SERVICES/INCONSISTENT_OBJECT_TYPE, got {other:?}"),
    }
}

fn assert_unknown_object(result: Result<(), Error>, context: &str) {
    match result.expect_err(&format!("{context}: request must be refused")) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::OBJECT.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                ErrorCode::UNKNOWN_OBJECT.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected OBJECT/UNKNOWN_OBJECT, got {other:?}"),
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Mismatch matrix (#287) — all four directions refuse with the exact
// Clause 14 error, leaving the response buffer untouched.
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn read_stream_against_record_access_file_is_refused() {
    let db = record_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire(FileAccessMethod::Stream {
            file_start_position: 0,
            requested_octet_count: 4,
        }),
        &mut buf,
    );
    assert_invalid_access(result, "read stream vs RECORD_ACCESS");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused read must not touch the response buffer"
    );
}

#[test]
fn read_record_against_stream_access_file_is_refused() {
    let db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire(FileAccessMethod::Record {
            file_start_record: 0,
            requested_record_count: 2,
        }),
        &mut buf,
    );
    assert_invalid_access(result, "read record vs STREAM_ACCESS");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused read must not touch the response buffer"
    );
}

#[test]
fn write_stream_against_record_access_file_is_refused() {
    let mut db = record_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire(FileWriteAccessMethod::Stream {
            file_start_position: 0,
            file_data: vec![0x01, 0x02],
        }),
        &mut buf,
    );
    assert_invalid_access(result, "write stream vs RECORD_ACCESS");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );
}

#[test]
fn write_record_against_stream_access_file_is_refused() {
    let mut db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire(FileWriteAccessMethod::Record {
            file_start_record: 0,
            record_count: 1,
            file_record_data: vec![vec![0x01, 0x02]],
        }),
        &mut buf,
    );
    assert_invalid_access(result, "write record vs STREAM_ACCESS");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );
}

#[test]
fn read_only_stream_write_returns_services_file_access_denied_without_mutation() {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_data(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    file.set_read_only(true);
    db.add(Box::new(file)).unwrap();

    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire(FileWriteAccessMethod::Stream {
            file_start_position: 0,
            file_data: vec![0x01, 0x02, 0x03],
        }),
        &mut buf,
    );
    assert_file_access_denied(result, "read-only STREAM_ACCESS file");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );

    let object = db.get(&file_oid()).unwrap();
    let size = object
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .unwrap();
    assert_eq!(size, PropertyValue::Unsigned(8), "FILE_SIZE changed");
}

#[test]
fn read_only_record_write_returns_services_file_access_denied_without_mutation() {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(ObjectFileAccessMethod::RECORD_ACCESS.to_raw());
    file.set_records(vec![vec![0xAA, 0xBB], vec![0xCC, 0xDD], vec![0xEE]]);
    file.set_read_only(true);
    db.add(Box::new(file)).unwrap();

    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire(FileWriteAccessMethod::Record {
            file_start_record: 0,
            record_count: 1,
            file_record_data: vec![vec![0x01, 0x02, 0x03, 0x04]],
        }),
        &mut buf,
    );
    assert_file_access_denied(result, "read-only RECORD_ACCESS file");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );

    let object = db.get(&file_oid()).unwrap();
    let size = object
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .unwrap();
    assert_eq!(size, PropertyValue::Unsigned(5), "FILE_SIZE changed");
    let count = object
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .unwrap();
    assert_eq!(count, PropertyValue::Unsigned(3), "RECORD_COUNT changed");
}

// ──────────────────────────────────────────────────────────────────────────
// No mutation on mismatched writes — observable File state is unchanged.
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn mismatched_write_leaves_record_file_state_unchanged() {
    let mut db = record_file_db();
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire(FileWriteAccessMethod::Stream {
            file_start_position: 0,
            file_data: vec![0x01, 0x02, 0x03],
        }),
        &mut BytesMut::new(),
    );
    assert_invalid_access(result, "write stream vs RECORD_ACCESS");

    let object = db.get(&file_oid()).unwrap();
    let size = object
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .unwrap();
    assert_eq!(size, PropertyValue::Unsigned(5), "FILE_SIZE changed");
    let count = object
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .unwrap();
    assert_eq!(count, PropertyValue::Unsigned(3), "RECORD_COUNT changed");
}

#[test]
fn mismatched_write_leaves_stream_file_state_unchanged() {
    let mut db = stream_file_db();
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire(FileWriteAccessMethod::Record {
            file_start_record: 0,
            record_count: 1,
            file_record_data: vec![vec![0x01, 0x02]],
        }),
        &mut BytesMut::new(),
    );
    assert_invalid_access(result, "write record vs STREAM_ACCESS");

    let object = db.get(&file_oid()).unwrap();
    let size = object
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .unwrap();
    assert_eq!(size, PropertyValue::Unsigned(8), "FILE_SIZE changed");
}

// ──────────────────────────────────────────────────────────────────────────
// Matched-method smoke coverage — the validator passes valid combinations
// through to the existing success paths.
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn matched_stream_read_still_succeeds() {
    let db = stream_file_db();
    let mut buf = BytesMut::new();
    handle_atomic_read_file(
        &db,
        &read_wire(FileAccessMethod::Stream {
            file_start_position: 2,
            requested_octet_count: 4,
        }),
        &mut buf,
    )
    .expect("matched stream read must succeed");
    let ack = AtomicReadFileAck::decode(&buf).unwrap();
    match ack.access {
        FileReadAckMethod::Stream {
            file_start_position,
            file_data,
        } => {
            assert_eq!(file_start_position, 2);
            assert_eq!(file_data, vec![3, 4, 5, 6]);
        }
        other => panic!("expected stream ACK method, got {other:?}"),
    }
}

#[test]
fn matched_record_read_still_succeeds() {
    let db = record_file_db();
    let mut buf = BytesMut::new();
    handle_atomic_read_file(
        &db,
        &read_wire(FileAccessMethod::Record {
            file_start_record: 1,
            requested_record_count: 2,
        }),
        &mut buf,
    )
    .expect("matched record read must succeed");
    let ack = AtomicReadFileAck::decode(&buf).unwrap();
    match ack.access {
        FileReadAckMethod::Record {
            returned_record_count,
            file_record_data,
            ..
        } => {
            assert_eq!(returned_record_count, 2);
            assert_eq!(file_record_data, vec![vec![0xCC, 0xDD], vec![0xEE]]);
        }
        other => panic!("expected record ACK method, got {other:?}"),
    }
}

#[test]
fn matched_record_write_still_succeeds() {
    let mut db = record_file_db();
    let mut buf = BytesMut::new();
    handle_atomic_write_file(
        &mut db,
        &write_wire(FileWriteAccessMethod::Record {
            file_start_record: 7,
            record_count: 1,
            file_record_data: vec![vec![0x01]],
        }),
        &mut buf,
    )
    .expect("matched record write must succeed");
    let ack = AtomicWriteFileAck::decode(&buf).unwrap();
    assert!(matches!(
        ack.access,
        bacnet_services::file::FileWriteAckMethod::Record {
            file_start_record: 7
        }
    ));
    let count = db
        .get(&file_oid())
        .unwrap()
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .unwrap();
    assert_eq!(
        count,
        PropertyValue::Unsigned(8),
        "write at record 7 must extend"
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Fail-closed on an unsupported File_Access_Method raw value — the object
// surface already accepts arbitrary raw values via set_file_access_method,
// so no production API expansion is needed to represent this state.
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn unknown_access_method_raw_value_fails_closed() {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_file_access_method(99);
    db.add(Box::new(file)).unwrap();

    let mut buf = BytesMut::new();
    let result = handle_atomic_read_file(
        &db,
        &read_wire(FileAccessMethod::Stream {
            file_start_position: 0,
            requested_octet_count: 4,
        }),
        &mut buf,
    );
    assert_invalid_access(result, "unknown raw access method");
    assert_eq!(&buf[..], &[], "buffer must stay empty");
}

// ──────────────────────────────────────────────────────────────────────────
// Non-File identifier classification (#398) — Clauses 14.1.4.1 / 14.2.4.1
// pair a non-File Object Identifier with SERVICES / INCONSISTENT_OBJECT_TYPE.
// The standard does not sequence that against "The File object does not
// exist", so these handlers classify by type first: a non-File identifier
// gets the type error whether or not it names an object, and only a FILE
// identifier that names no object gets OBJECT / UNKNOWN_OBJECT.
// ──────────────────────────────────────────────────────────────────────────

fn analog_input_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn absent_non_file_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 7).unwrap()
}

fn absent_file_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::FILE, 99).unwrap()
}

fn stream_file_and_analog_input_db() -> ObjectDatabase {
    let mut db = stream_file_db();
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.set_present_value(72.5);
    db.add(Box::new(ai)).unwrap();
    db
}

#[test]
fn read_existing_non_file_identifier_is_inconsistent_object_type() {
    let db = stream_file_and_analog_input_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            analog_input_oid(),
            FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 4,
            },
        ),
        &mut buf,
    );
    assert_inconsistent_object_type(result, "read of existing ANALOG_INPUT");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused read must not touch the response buffer"
    );
}

#[test]
fn read_absent_non_file_identifier_is_inconsistent_object_type() {
    let db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            absent_non_file_oid(),
            FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 4,
            },
        ),
        &mut buf,
    );
    assert_inconsistent_object_type(result, "read of absent MULTI_STATE_INPUT");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused read must not touch the response buffer"
    );
}

#[test]
fn read_absent_file_identifier_is_unknown_object() {
    let db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_read_file(
        &db,
        &read_wire_for(
            absent_file_oid(),
            FileAccessMethod::Stream {
                file_start_position: 0,
                requested_octet_count: 4,
            },
        ),
        &mut buf,
    );
    assert_unknown_object(result, "read of absent FILE");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused read must not touch the response buffer"
    );
}

#[test]
fn write_existing_non_file_identifier_is_inconsistent_object_type() {
    let mut db = stream_file_and_analog_input_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            analog_input_oid(),
            FileWriteAccessMethod::Stream {
                file_start_position: 0,
                file_data: vec![0x01, 0x02, 0x03],
            },
        ),
        &mut buf,
    );
    assert_inconsistent_object_type(result, "write to existing ANALOG_INPUT");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );

    let ai = db.get(&analog_input_oid()).unwrap();
    let pv = ai
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Real(72.5), "PRESENT_VALUE changed");
    let file = db.get(&file_oid()).unwrap();
    let size = file
        .read_property(PropertyIdentifier::FILE_SIZE, None)
        .unwrap();
    assert_eq!(size, PropertyValue::Unsigned(8), "FILE_SIZE changed");
}

#[test]
fn write_absent_non_file_identifier_is_inconsistent_object_type() {
    let mut db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            absent_non_file_oid(),
            FileWriteAccessMethod::Record {
                file_start_record: 0,
                record_count: 1,
                file_record_data: vec![vec![0x01, 0x02]],
            },
        ),
        &mut buf,
    );
    assert_inconsistent_object_type(result, "write to absent MULTI_STATE_INPUT");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );
}

#[test]
fn write_absent_file_identifier_is_unknown_object() {
    let mut db = stream_file_db();
    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(
            absent_file_oid(),
            FileWriteAccessMethod::Stream {
                file_start_position: 0,
                file_data: vec![0x01, 0x02, 0x03],
            },
        ),
        &mut buf,
    );
    assert_unknown_object(result, "write to absent FILE");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );
}

#[test]
fn non_file_identifier_is_refused_before_read_only_and_access_method_gates() {
    let mut db = ObjectDatabase::new();
    let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
    file.set_data(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    file.set_read_only(true);
    db.add(Box::new(file)).unwrap();
    db.add(Box::new(AnalogInputObject::new(1, "AI-1", 62).unwrap()))
        .unwrap();

    let record_write = FileWriteAccessMethod::Record {
        file_start_record: 0,
        record_count: 1,
        file_record_data: vec![vec![0x01, 0x02]],
    };

    let mut buf = BytesMut::from(SENTINEL);
    let result = handle_atomic_write_file(
        &mut db,
        &write_wire_for(analog_input_oid(), record_write.clone()),
        &mut buf,
    );
    assert_inconsistent_object_type(result, "record write to ANALOG_INPUT");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );

    let mut buf = BytesMut::from(SENTINEL);
    let result =
        handle_atomic_write_file(&mut db, &write_wire_for(file_oid(), record_write), &mut buf);
    assert_file_access_denied(result, "record write to read-only FILE");
    assert_eq!(
        &buf[..],
        SENTINEL,
        "refused write must not touch the response buffer"
    );
}
