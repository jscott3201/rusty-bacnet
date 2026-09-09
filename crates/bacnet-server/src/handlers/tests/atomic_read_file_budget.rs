use super::*;
use crate::server::AtomicReadFileBudget;
use bacnet_objects::file::{
    FileObject, FileRecordRead, FileStorage, FileStreamRead, FileWriteStart,
};
use bacnet_services::file::{
    AtomicReadFileAck, AtomicReadFileRequest, FileAccessMethod, FileReadAckMethod,
};
use bacnet_types::enums::{AbortReason, FileAccessMethod as Method};
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct Probe {
    file: FileObject,
    reads: Arc<AtomicUsize>,
    count: Arc<AtomicUsize>,
    hook: bool,
    failure: bool,
    window: bool,
    giant: bool,
}
impl BACnetObject for Probe {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.file.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.file.object_name()
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        assert_eq!(
            p,
            PropertyIdentifier::FILE_ACCESS_METHOD,
            "no new size/metadata reads"
        );
        self.file.read_property(p, i)
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("read-only test")
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        self.hook.then_some(self)
    }
}
impl FileStorage for Probe {
    fn read_stream(&self, start: u64, count: u64) -> Result<FileStreamRead, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.count.store(count as usize, Ordering::SeqCst);
        if self.failure {
            return Err(storage_error());
        }
        self.file.read_stream(start, count)
    }
    fn read_records(&self, start: u64, count: u64) -> Result<FileRecordRead, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.count.store(count as usize, Ordering::SeqCst);
        if self.failure {
            return Err(storage_error());
        }
        if self.giant {
            return Ok(FileRecordRead {
                records: vec![vec![42; 1_000_000]],
                end_of_file: false,
            });
        }
        if self.window {
            return Ok(FileRecordRead {
                records: vec![vec![]; count as usize],
                end_of_file: false,
            });
        }
        self.file.read_records(start, count)
    }
    fn write_stream(&mut self, _: FileWriteStart, _: &[u8]) -> Result<u64, Error> {
        panic!("no writes")
    }
    fn write_records(&mut self, _: FileWriteStart, _: &[Vec<u8>]) -> Result<u64, Error> {
        panic!("no writes")
    }
}
fn storage_error() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES.to_raw() as u32,
        code: ErrorCode::FILE_ACCESS_DENIED.to_raw() as u32,
    }
}
fn probe(record: bool) -> Probe {
    let mut file = FileObject::new(1, "file", "binary").unwrap();
    if record {
        file.set_file_access_method(Method::RECORD_ACCESS.to_raw());
        file.set_records(vec![vec![], vec![1, 2, 3, 4, 5]]);
    } else {
        file.set_data(vec![1, 2, 3, 4, 5]);
    }
    Probe {
        file,
        reads: Arc::new(AtomicUsize::new(0)),
        count: Arc::new(AtomicUsize::new(0)),
        hook: true,
        failure: false,
        window: false,
        giant: false,
    }
}
fn wire(record: bool, start: i32, count: u32) -> BytesMut {
    let mut wire = BytesMut::new();
    AtomicReadFileRequest {
        file_identifier: ObjectIdentifier::new(ObjectType::FILE, 1).unwrap(),
        access: if record {
            FileAccessMethod::Record {
                file_start_record: start,
                requested_record_count: count,
            }
        } else {
            FileAccessMethod::Stream {
                file_start_position: start,
                requested_octet_count: count,
            }
        },
    }
    .encode(&mut wire);
    wire
}
fn abort(result: Result<(), Error>, reason: AbortReason) {
    assert!(matches!(result, Err(Error::Abort { reason: actual }) if actual == reason.to_raw()));
}
fn call(
    db: &ObjectDatabase,
    request: &[u8],
    cap: AtomicReadFileBudget,
) -> (Result<(), Error>, BytesMut) {
    let mut out = BytesMut::from(&b"sentinel"[..]);
    let result =
        handle_atomic_read_file_budgeted(db, request, &mut out, cap).map_err(
            |failure| match failure {
                AtomicReadFileFailure::Service(error) => error,
                AtomicReadFileFailure::Budget(reason) => Error::Abort {
                    reason: reason.to_raw(),
                },
            },
        );
    if result.is_err() {
        assert_eq!(&out[..], b"sentinel");
        assert_eq!(out.capacity(), 8);
    }
    (result, out)
}

#[test]
fn atomic_read_file_raw_count_exact_over_and_opaque_failure_precedence() {
    for record in [false, true] {
        for failure in [false, true] {
            let mut p = probe(record);
            p.failure = failure;
            let reads = p.reads.clone();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(p)).unwrap();
            let cap = AtomicReadFileBudget {
                max_requested_stream_octets: 5,
                max_requested_records: 5,
                ..Default::default()
            };
            for start in [0, 1, 2, 5, 6] {
                abort(
                    call(&db, &wire(record, start, 6), cap).0,
                    AbortReason::OUT_OF_RESOURCES,
                );
            }
            assert_eq!(reads.load(Ordering::SeqCst), 0);
            let result = call(&db, &wire(record, 0, 5), cap).0;
            if failure {
                assert_eq!(
                    format!("{:?}", result.unwrap_err()),
                    format!("{:?}", storage_error())
                );
            } else {
                result.unwrap();
            }
            assert_eq!(reads.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn atomic_read_file_validation_precedes_budget_and_preserves_output() {
    for record in [false, true] {
        for hook in [false, true] {
            for declared_record in [false, true] {
                let mut p = probe(declared_record);
                p.hook = hook;
                let reads = p.reads.clone();
                let mut db = ObjectDatabase::new();
                db.add(Box::new(p)).unwrap();
                for request in [
                    BytesMut::new(),
                    BytesMut::from(&b"\xc4"[..]),
                    wire(record, -1, u32::MAX),
                ] {
                    let mut legacy = BytesMut::from(&b"sentinel"[..]);
                    let expected = handle_atomic_read_file(&db, &request, &mut legacy).unwrap_err();
                    let (actual, _) = call(&db, &request, AtomicReadFileBudget::default());
                    assert_eq!(
                        format!("{:?}", actual.unwrap_err()),
                        format!("{expected:?}")
                    );
                }
                assert_eq!(reads.load(Ordering::SeqCst), 0);
            }
        }
        for object_type in [ObjectType::FILE, ObjectType::ANALOG_INPUT] {
            let mut request = wire(record, -1, u32::MAX);
            let oid = ObjectIdentifier::new(object_type, 2).unwrap();
            let mut decoded = AtomicReadFileRequest::decode(&request).unwrap();
            decoded.file_identifier = oid;
            request.clear();
            decoded.encode(&mut request);
            for mut db in [ObjectDatabase::new(), make_db_with_ai()] {
                if object_type == ObjectType::ANALOG_INPUT {
                    db.add(Box::new(AnalogInputObject::new(2, "AI-2", 62).unwrap()))
                        .unwrap();
                }
                let expected =
                    handle_atomic_read_file(&db, &request, &mut BytesMut::new()).unwrap_err();
                let actual = call(&db, &request, AtomicReadFileBudget::default())
                    .0
                    .unwrap_err();
                assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
            }
        }
    }
}

#[test]
fn atomic_read_file_admitted_zero_empty_eof_and_errors_match_legacy() {
    for record in [false, true] {
        for empty in [false, true] {
            let mut p = probe(record);
            if empty {
                if record {
                    p.file.set_records(vec![]);
                } else {
                    p.file.set_data(vec![]);
                }
            }
            let mut db = ObjectDatabase::new();
            db.add(Box::new(p)).unwrap();
            for start in [0, 1, 2, 5, 6] {
                for count in [0, 1, 5] {
                    let request = wire(record, start, count);
                    let mut legacy = BytesMut::from(&b"sentinel"[..]);
                    let expected = handle_atomic_read_file(&db, &request, &mut legacy);
                    let (actual, out) = call(&db, &request, AtomicReadFileBudget::default());
                    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
                    assert_eq!(out, legacy);
                }
            }
            if empty {
                abort(
                    call(
                        &db,
                        &wire(record, 0, u32::MAX),
                        AtomicReadFileBudget::default(),
                    )
                    .0,
                    AbortReason::OUT_OF_RESOURCES,
                );
            }
        }
    }
}

#[test]
fn atomic_read_file_raw_10001_checked_before_legacy_10000_window() {
    let mut p = probe(true);
    p.window = true;
    let reads = p.reads.clone();
    let count = p.count.clone();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(p)).unwrap();
    for cap in [256, 10000] {
        abort(
            call(
                &db,
                &wire(true, 0, 10001),
                AtomicReadFileBudget {
                    max_requested_records: cap,
                    ..Default::default()
                },
            )
            .0,
            AbortReason::OUT_OF_RESOURCES,
        );
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    let (result, out) = call(
        &db,
        &wire(true, 0, 10001),
        AtomicReadFileBudget {
            max_requested_records: 10001,
            ..Default::default()
        },
    );
    result.unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(count.load(Ordering::SeqCst), 10000);
    let ack = AtomicReadFileAck::decode(&out[8..]).unwrap();
    assert!(!ack.end_of_file);
    assert!(matches!(
        ack.access,
        FileReadAckMethod::Record {
            returned_record_count: 10000,
            ..
        }
    ));
}

#[test]
fn atomic_read_file_service_bytes_exact_tiny_and_no_payload_copy_on_refusal() {
    for record in [false, true] {
        let mut p = probe(record);
        // Stream: bool/open/signed(2)/octets(2+5)/close = 12.
        // Record adds returned count(2) and an empty octet string(1) = 15.
        let expected = if record { 15 } else { 12 };
        p.failure = false;
        let reads = p.reads.clone();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(p)).unwrap();
        for cap in [1, expected - 1, expected] {
            let (result, out) = call(
                &db,
                &wire(record, 0, 5),
                AtomicReadFileBudget {
                    max_service_ack_bytes: cap,
                    ..Default::default()
                },
            );
            if cap < expected {
                abort(result, AbortReason::BUFFER_OVERFLOW);
            } else {
                result.unwrap();
                assert_eq!(out.len(), expected + 8);
            }
        }
        assert_eq!(reads.load(Ordering::SeqCst), 3);
    }
    let mut p = probe(true);
    p.giant = true;
    let reads = p.reads.clone();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(p)).unwrap();
    abort(
        call(&db, &wire(true, 0, 1), AtomicReadFileBudget::default()).0,
        AbortReason::BUFFER_OVERFLOW,
    );
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn atomic_read_file_default_payload_requires_ack_overhead_allowance() {
    for record in [false, true] {
        let mut p = probe(record);
        if record {
            p.file.set_records(vec![vec![42; 16384]]);
        } else {
            p.file.set_data(vec![42; 16384]);
        }
        let mut db = ObjectDatabase::new();
        db.add(Box::new(p)).unwrap();
        let request = wire(record, 0, if record { 1 } else { 16384 });
        abort(
            call(&db, &request, AtomicReadFileBudget::default()).0,
            AbortReason::BUFFER_OVERFLOW,
        );
        let cap = 16384 + if record { 11 } else { 9 };
        let (result, out) = call(
            &db,
            &request,
            AtomicReadFileBudget {
                max_service_ack_bytes: cap,
                ..Default::default()
            },
        );
        result.unwrap();
        assert_eq!(out.len(), 8 + cap);
        assert!(AtomicReadFileAck::decode(&out[8..]).unwrap().end_of_file);
    }
}

#[test]
fn atomic_read_file_default_exact_counts_and_empty_ack_caps() {
    for record in [false, true] {
        let mut p = probe(record);
        if record {
            p.file.set_records(vec![]);
        } else {
            p.file.set_data(vec![]);
        }
        let mut db = ObjectDatabase::new();
        db.add(Box::new(p)).unwrap();
        let default = AtomicReadFileBudget::default();
        let count = if record { 256 } else { 16384 };
        let size = if record { 7 } else { 6 };
        for requested in [0, count] {
            for cap in [size - 1, size] {
                let (result, out) = call(
                    &db,
                    &wire(record, 0, requested),
                    AtomicReadFileBudget {
                        max_service_ack_bytes: cap,
                        ..default
                    },
                );
                if cap < size {
                    abort(result, AbortReason::BUFFER_OVERFLOW);
                } else {
                    result.unwrap();
                    assert_eq!(out.len(), 8 + size);
                }
            }
        }
        abort(
            call(&db, &wire(record, 0, count + 1), default).0,
            AbortReason::OUT_OF_RESOURCES,
        );
        let mut p = probe(record);
        p.file.set_file_access_method(99);
        let reads = p.reads.clone();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(p)).unwrap();
        let request = wire(record, 0, u32::MAX);
        let expected = handle_atomic_read_file(&db, &request, &mut BytesMut::new()).unwrap_err();
        let actual = call(&db, &request, default).0.unwrap_err();
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }
}
