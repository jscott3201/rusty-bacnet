use super::*;
use crate::server::AtomicWriteFileBudget;
use bacnet_objects::file::{
    FileObject, FileRecordRead, FileStorage, FileStreamRead, FileWriteStart,
};
use bacnet_services::file::{
    AtomicWriteFileAck, AtomicWriteFileRequest, FileWriteAccessMethod, FileWriteAckMethod,
};
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct Probe {
    file: FileObject,
    calls: Arc<AtomicUsize>,
    gate: u8,
    failure: u8,
}
impl BACnetObject for Probe {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.file.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.file.object_name()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        assert!([
            PropertyIdentifier::READ_ONLY,
            PropertyIdentifier::FILE_ACCESS_METHOD
        ]
        .contains(&p));
        if p == PropertyIdentifier::READ_ONLY {
            match self.gate {
                2 => return Ok(PropertyValue::Boolean(true)),
                3 => return Err(backend_error(1)),
                4 => return Ok(PropertyValue::Unsigned(0)),
                _ => {}
            }
        } else if self.gate == 5 {
            return Ok(PropertyValue::Enumerated(99));
        }
        self.file.read_property(p, i)
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("no property writes")
    }
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        (self.gate != 1).then_some(self)
    }
    fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
        if self.gate == 6 {
            None
        } else {
            Some(self)
        }
    }
}
impl FileStorage for Probe {
    fn read_stream(&self, s: u64, n: u64) -> Result<FileStreamRead, Error> {
        self.file.read_stream(s, n)
    }
    fn read_records(&self, s: u64, n: u64) -> Result<FileRecordRead, Error> {
        self.file.read_records(s, n)
    }
    fn write_stream(&mut self, s: FileWriteStart, data: &[u8]) -> Result<u64, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failure != 0 {
            return Err(backend_error(self.failure));
        }
        self.file.write_stream(s, data)
    }
    fn write_records(&mut self, s: FileWriteStart, data: &[Vec<u8>]) -> Result<u64, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failure != 0 {
            return Err(backend_error(self.failure));
        }
        self.file.write_records(s, data)
    }
}
fn backend_error(kind: u8) -> Error {
    if kind == 2 {
        Error::Abort { reason: 9 }
    } else {
        Error::Protocol {
            class: if kind == 0 {
                ErrorClass::OBJECT
            } else {
                ErrorClass::SERVICES
            }
            .to_raw() as u32,
            code: ErrorCode::FILE_FULL.to_raw() as u32,
        }
    }
}
fn probe(record: bool) -> Probe {
    let mut file = FileObject::new(1, "file", "binary").unwrap();
    if record {
        file.set_file_access_method(bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw());
        file.set_records(vec![b"sentinel".to_vec()]);
    } else {
        file.set_data(b"sentinel".to_vec());
    }
    Probe {
        file,
        calls: Arc::new(AtomicUsize::new(0)),
        gate: 0,
        failure: 0,
    }
}
fn wire(record: bool, start: i32, records: Vec<Vec<u8>>) -> BytesMut {
    let mut out = BytesMut::new();
    AtomicWriteFileRequest {
        file_identifier: ObjectIdentifier::new(ObjectType::FILE, 1).unwrap(),
        access: if record {
            FileWriteAccessMethod::Record {
                file_start_record: start,
                record_count: records.len() as u32,
                file_record_data: records,
            }
        } else {
            FileWriteAccessMethod::Stream {
                file_start_position: start,
                file_data: records.concat(),
            }
        },
    }
    .encode(&mut out);
    out
}
fn database(p: Probe) -> (ObjectDatabase, Arc<AtomicUsize>) {
    let calls = p.calls.clone();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(p)).unwrap();
    (db, calls)
}
fn state(db: &ObjectDatabase, record: bool) -> Vec<Vec<u8>> {
    let s = db
        .get(&ObjectIdentifier::new(ObjectType::FILE, 1).unwrap())
        .unwrap()
        .file_storage_internal()
        .unwrap();
    if record {
        s.read_records(0, 10000).unwrap().records
    } else {
        vec![s.read_stream(0, 100000).unwrap().data]
    }
}
fn call(
    db: &mut ObjectDatabase,
    req: &[u8],
    budget: AtomicWriteFileBudget,
) -> Result<BytesMut, AtomicWriteFileFailure> {
    let mut out = BytesMut::from(&b"sentinel"[..]);
    let result = handle_atomic_write_file_budgeted(db, req, &mut out, budget);
    if result.is_err() {
        assert_eq!(&out[..], b"sentinel");
        assert_eq!(out.capacity(), 8);
    }
    result.map(|()| out.split_off(8))
}

#[test]
fn atomic_write_file_exact_over_independent_payload_count_limits() {
    let defaults = AtomicWriteFileBudget::default();
    for (record, payload, budget, admitted) in [
        (false, vec![vec![42; 16384]], defaults, true),
        (false, vec![vec![42; 16385]], defaults, false),
        (true, vec![vec![]; 256], defaults, true),
        (true, vec![vec![]; 257], defaults, false),
        (true, vec![vec![42; 8192]; 2], defaults, true),
        (true, vec![vec![42; 5462]; 3], defaults, false),
        (
            true,
            vec![vec![42; 3]; 2],
            AtomicWriteFileBudget {
                max_records: 2,
                max_record_payload_bytes: 5,
                ..defaults
            },
            false,
        ),
        (
            true,
            vec![vec![42; 3]; 2],
            AtomicWriteFileBudget {
                max_records: 1,
                max_record_payload_bytes: 6,
                ..defaults
            },
            false,
        ),
        (
            true,
            vec![vec![42; 3]; 2],
            AtomicWriteFileBudget {
                max_records: 2,
                max_record_payload_bytes: 6,
                ..defaults
            },
            true,
        ),
        (true, vec![vec![]; 10000], defaults, false),
    ] {
        let (mut db, calls) = database(probe(record));
        let before = state(&db, record);
        let result = call(&mut db, &wire(record, -1, payload.clone()), budget);
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(admitted));
        if admitted {
            let ack = AtomicWriteFileAck::decode(&result.unwrap()).unwrap();
            assert_eq!(
                ack.access,
                if record {
                    FileWriteAckMethod::Record {
                        file_start_record: 1,
                    }
                } else {
                    FileWriteAckMethod::Stream {
                        file_start_position: 8,
                    }
                }
            );
            let expected = if record {
                [before, payload].concat()
            } else {
                vec![[b"sentinel".to_vec(), payload.concat()].concat()]
            };
            assert_eq!(state(&db, record), expected);
        } else {
            assert!(matches!(result, Err(AtomicWriteFileFailure::Budget)));
            assert_eq!(state(&db, record), before);
        }
    }
}

#[test]
fn atomic_write_file_legacy_parity_zero_append_position_gap_and_raised_count() {
    for record in [false, true] {
        for start in [-2, -1, 0, 1, 20] {
            for payload in [vec![], vec![vec![]], vec![vec![7], vec![], vec![8, 9]]] {
                let (mut legacy, _) = database(probe(record));
                let (mut db, calls) = database(probe(record));
                let request = wire(record, start, payload);
                let mut expected = BytesMut::new();
                let result = handle_atomic_write_file(&mut legacy, &request, &mut expected);
                let actual = call(&mut db, &request, AtomicWriteFileBudget::default());
                match (result, actual) {
                    (Ok(()), Ok(bytes)) => assert_eq!(bytes, expected),
                    (Err(a), Err(AtomicWriteFileFailure::Service(b))) => {
                        assert_eq!(format!("{a:?}"), format!("{b:?}"))
                    }
                    pair => panic!("parity: {pair:?}"),
                }
                assert_eq!(state(&db, record), state(&legacy, record));
                assert_eq!(calls.load(Ordering::SeqCst), usize::from(start >= -1));
            }
        }
    }
    // The decoder's 10000 ceiling is independent of raised local admission.
    for count in [10000, 10001] {
        let mut p = probe(true);
        p.file.set_records(vec![]);
        let (mut db, calls) = database(p);
        let req = wire(true, 0, vec![vec![]; count]);
        let result = call(
            &mut db,
            &req,
            AtomicWriteFileBudget {
                max_records: 10001,
                ..Default::default()
            },
        );
        if count == 10000 {
            result.unwrap();
        } else {
            let expected =
                handle_atomic_write_file(&mut db, &req, &mut BytesMut::new()).unwrap_err();
            assert!(
                matches!(result, Err(AtomicWriteFileFailure::Service(e)) if format!("{e:?}") == format!("{expected:?}"))
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(count == 10000));
    }
}

#[test]
fn atomic_write_file_preadmission_gates_precede_oversized_payload() {
    for record in [false, true] {
        let request = wire(
            record,
            -1,
            vec![vec![42; 16385]; if record { 257 } else { 1 }],
        );
        for gate in 1..=6 {
            let mut p = probe(record);
            p.gate = gate;
            let (mut db, calls) = database(p);
            let expected =
                handle_atomic_write_file(&mut db, &request, &mut BytesMut::new()).unwrap_err();
            assert!(
                matches!(call(&mut db, &request, AtomicWriteFileBudget::default()), Err(AtomicWriteFileFailure::Service(e)) if format!("{e:?}") == format!("{expected:?}"))
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
        for kind in 0..5 {
            let (mut db, calls) = database(probe(record));
            let mut req = request.clone();
            match kind {
                0 => req = wire(record, -2, vec![vec![42; 16385]; 257]),
                1 => req[4] = 2,                  // absent File instance
                2 => req[1] = 0,                  // non-File type, absent
                3 => req.truncate(req.len() - 1), // missing close
                4 => {
                    // Wrong request access method, independent of payload size.
                    req = wire(!record, -1, vec![vec![42; 16385]; 257]);
                }
                _ => unreachable!(),
            }
            let expected =
                handle_atomic_write_file(&mut db, &req, &mut BytesMut::new()).unwrap_err();
            assert!(
                matches!(call(&mut db, &req, AtomicWriteFileBudget::default()), Err(AtomicWriteFileFailure::Service(e)) if format!("{e:?}") == format!("{expected:?}"))
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }
    // Cardinality mismatch remains decoder error, not local admission.
    let mut req = wire(true, 0, vec![vec![]; 257]);
    assert_eq!(&req[8..11], &[0x22, 1, 1]);
    req[10] = 0;
    let (mut db, calls) = database(probe(true));
    let expected = handle_atomic_write_file(&mut db, &req, &mut BytesMut::new()).unwrap_err();
    assert!(
        matches!(call(&mut db, &req, AtomicWriteFileBudget::default()), Err(AtomicWriteFileFailure::Service(e)) if format!("{e:?}") == format!("{expected:?}"))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn atomic_write_file_admitted_backend_errors_are_not_budget_errors() {
    for record in [false, true] {
        for failure in [0, 1, 2] {
            for over in [false, true] {
                let mut p = probe(record);
                p.failure = failure;
                // Actual backend zero growth cap (existing sentinel is retained).
                p.file.set_max_file_size(0);
                p.file.set_max_record_count(0);
                let (mut db, calls) = database(p);
                let request = wire(record, -1, vec![vec![42; if over { 16385 } else { 1 }]]);
                let result = call(&mut db, &request, AtomicWriteFileBudget::default());
                if over {
                    assert!(matches!(result, Err(AtomicWriteFileFailure::Budget)));
                } else {
                    assert!(
                        matches!(result, Err(AtomicWriteFileFailure::Service(e)) if format!("{e:?}") == format!("{:?}", backend_error(failure)))
                    );
                }
                assert_eq!(calls.load(Ordering::SeqCst), usize::from(!over));
                assert_eq!(state(&db, record), vec![b"sentinel".to_vec()]);
            }
        }
    }
}
