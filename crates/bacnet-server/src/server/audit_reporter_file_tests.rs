use super::*;
use bacnet_objects::{
    file::{FileObject, FileRecordRead, FileStorage, FileStreamRead, FileWriteStart},
    traits::BACnetObject,
};
use bacnet_services::file::{AtomicWriteFileRequest, FileWriteAccessMethod};
use bacnet_types::{constructed::BACnetObjectSelector, enums::FileAccessMethod};
use std::{borrow::Cow, sync::atomic::AtomicUsize};

pub(super) const SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::ATOMIC_WRITE_FILE;

fn file_object(record: bool) -> FileObject {
    let mut file = FileObject::new(1, "file", "binary").unwrap();
    if record {
        file.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
        file.set_records(vec![vec![1], vec![2]]);
    } else {
        file.set_data(vec![1, 2]);
    }
    file.set_archive(true);
    file
}

pub(super) fn access(record: bool, start: i32) -> FileWriteAccessMethod {
    if record {
        FileWriteAccessMethod::Record {
            file_start_record: start,
            record_count: 2,
            file_record_data: vec![vec![9], vec![8]],
        }
    } else {
        FileWriteAccessMethod::Stream {
            file_start_position: start,
            file_data: vec![9, 8],
        }
    }
}

pub(super) fn request(target: ObjectIdentifier, access: FileWriteAccessMethod) -> Bytes {
    let mut bytes = BytesMut::new();
    AtomicWriteFileRequest {
        file_identifier: target,
        access,
    }
    .encode(&mut bytes);
    bytes.freeze()
}

pub(super) async fn file_server(record: bool) -> Fixture {
    let fixture = server(reporter()).await;
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(file_object(record)))
        .unwrap();
    fixture
}

async fn contents(fixture: &Fixture, record: bool) -> Vec<Vec<u8>> {
    let db = fixture.server.db.read().await;
    let storage = db
        .get(&oid(ObjectType::FILE, 1))
        .unwrap()
        .file_storage_internal()
        .unwrap();
    if record {
        storage.read_records(0, 100).unwrap().records
    } else {
        vec![storage.read_stream(0, 100).unwrap().data]
    }
}

async fn metadata(fixture: &Fixture) -> Vec<PropertyValue> {
    let db = fixture.server.db.read().await;
    let file = db.get(&oid(ObjectType::FILE, 1)).unwrap();
    [
        PropertyIdentifier::FILE_SIZE,
        PropertyIdentifier::MODIFICATION_DATE,
        PropertyIdentifier::ARCHIVE,
    ]
    .into_iter()
    .map(|p| file.read_property(p, None).unwrap())
    .collect()
}

fn wire(response: &Apdu) -> BytesMut {
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, response).unwrap();
    bytes
}

fn assert_ack(response: &Apdu, record: bool, invoke: u8, position: u8) {
    // Independent nonsegmented ComplexACK + context signed position vector.
    assert_eq!(
        wire(response).as_ref(),
        &[0x30, invoke, 7, if record { 0x19 } else { 0x09 }, position]
    );
}

fn expected(
    target: ObjectIdentifier,
    step: u8,
    result: Option<(ErrorClass, ErrorCode)>,
) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: Some(BACnetTimeStamp::SequenceNumber(u16::from(step))),
        source_device: BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(SOURCE),
        }),
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: Some(77 + step),
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
        target_object: Some(target),
        target_property: None,
        target_priority: None,
        target_value: None,
        current_value: None,
        result,
    }
}

fn assert_error(response: &Apdu, pair: (ErrorClass, ErrorCode)) {
    let Apdu::Error(error) = response else {
        panic!("{response:?}")
    };
    assert_eq!(error.service_choice, SERVICE);
    assert_eq!((error.error_class, error.error_code), pair);
    assert!(error.error_data.is_empty());
}

#[tokio::test]
async fn audit_reporter_atomic_write_file_success_shape_positions_and_single_completion() {
    for record in [false, true] {
        let mut fixture = file_server(record).await;
        let target = oid(ObjectType::FILE, 1);
        for (step, start) in [-1, 0, 6].into_iter().enumerate() {
            let response = dispatch(
                &fixture.server,
                SERVICE,
                request(target, access(record, start)),
            )
            .await;
            assert_ack(
                &response,
                record,
                77 + step as u8,
                if start == -1 { 2 } else { start as u8 },
            );
            let bytes = match step {
                0 => vec![1, 2, 9, 8],
                1 => vec![9, 8, 9, 8],
                _ => vec![9, 8, 9, 8, 0, 0, 9, 8],
            };
            let after = if record {
                bytes
                    .into_iter()
                    .map(|byte| if byte == 0 { vec![] } else { vec![byte] })
                    .collect()
            } else {
                vec![bytes]
            };
            assert_eq!(contents(&fixture, record).await, after);
            settle().await;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), step + 1);
            assert_eq!(
                records[step].notifications,
                vec![expected(target, step as u8, None)]
            );
        }
        fixture.server.stop().await.unwrap();
        assert_eq!(
            notifications(&fixture.transport.sent).len(),
            3,
            "no recursion"
        );
    }
}

#[tokio::test]
async fn audit_reporter_atomic_write_file_preserves_decoder_acceptance_boundary() {
    for record in [false, true] {
        let mut fixture = file_server(record).await;
        let target = oid(ObjectType::FILE, 1);
        let mut accepted = request(target, access(record, -1)).to_vec();
        accepted.extend_from_slice(&[0, 0]);
        assert!(AtomicWriteFileRequest::decode(&accepted).is_ok());
        let response = dispatch(&fixture.server, SERVICE, Bytes::from(accepted)).await;
        assert_ack(&response, record, 77, 2);
        assert_eq!(
            contents(&fixture, record).await,
            if record {
                vec![vec![1], vec![2], vec![9], vec![8]]
            } else {
                vec![vec![1, 2, 9, 8]]
            }
        );
        settle().await;
        assert_eq!(
            notifications(&fixture.transport.sent)[0].notifications,
            vec![expected(target, 0, None)]
        );
        let before = contents(&fixture, record).await;
        let before_metadata = metadata(&fixture).await;
        for rejected in [
            Bytes::new(),
            {
                let mut bytes = request(target, access(record, -1)).to_vec();
                bytes.pop();
                Bytes::from(bytes)
            },
            request(
                target,
                FileWriteAccessMethod::Record {
                    file_start_record: -1,
                    record_count: 2,
                    file_record_data: vec![vec![9]],
                },
            ),
        ] {
            let error = AtomicWriteFileRequest::decode(&rejected).unwrap_err();
            let response = dispatch(&fixture.server, SERVICE, rejected).await;
            let invoke_id = match &response {
                Apdu::Error(e) => e.invoke_id,
                Apdu::Reject(r) => r.invoke_id,
                _ => panic!("{response:?}"),
            };
            assert_eq!(
                wire(&response),
                wire(&BACnetServer::<CaptureTransport>::error_apdu_from_error(
                    invoke_id, SERVICE, &error
                ))
            );
            assert_eq!(contents(&fixture, record).await, before);
            assert_eq!(metadata(&fixture).await, before_metadata);
        }
        settle().await;
        assert_eq!(notifications(&fixture.transport.sent).len(), 1);
        fixture.server.stop().await.unwrap();
    }
}

// A file backend with controlled outcomes, without changing the service decoder
// or treating storage failures as admission overload.
struct ProbeFile {
    file: FileObject,
    storage: bool,
    mutable_storage: bool,
    bad_position: bool,
    attempts: Arc<AtomicUsize>,
    error: Arc<StdMutex<Option<Error>>>,
}

impl BACnetObject for ProbeFile {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.file.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.file.object_name()
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        self.file.read_property(p, i)
    }
    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.file.write_property(p, i, v, priority)
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.file.property_list()
    }
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        self.storage.then_some(self)
    }
    fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
        if self.mutable_storage {
            Some(self)
        } else {
            None
        }
    }
}

impl FileStorage for ProbeFile {
    fn read_stream(&self, start: u64, count: u64) -> Result<FileStreamRead, Error> {
        self.file.read_stream(start, count)
    }
    fn read_records(&self, start: u64, count: u64) -> Result<FileRecordRead, Error> {
        self.file.read_records(start, count)
    }
    fn write_stream(&mut self, start: FileWriteStart, data: &[u8]) -> Result<u64, Error> {
        self.attempts.fetch_add(1, Ordering::AcqRel);
        if let Some(error) = self.error.lock().unwrap().take() {
            return Err(error);
        }
        let actual = self.file.write_stream(start, data)?;
        Ok(if self.bad_position {
            i32::MAX as u64 + 1
        } else {
            actual
        })
    }
    fn write_records(&mut self, start: FileWriteStart, records: &[Vec<u8>]) -> Result<u64, Error> {
        self.attempts.fetch_add(1, Ordering::AcqRel);
        if let Some(error) = self.error.lock().unwrap().take() {
            return Err(error);
        }
        let actual = self.file.write_records(start, records)?;
        Ok(if self.bad_position {
            i32::MAX as u64 + 1
        } else {
            actual
        })
    }
}

fn probe(fixture: &Fixture, record: bool) -> ProbeFile {
    ProbeFile {
        file: file_object(record),
        storage: true,
        mutable_storage: true,
        bad_position: false,
        attempts: fixture.attempts.clone(),
        error: fixture.execution_error.clone(),
    }
}

#[tokio::test]
async fn audit_reporter_atomic_write_file_execution_gates_precedence_and_results() {
    for record in [false, true] {
        for case in 0..10 {
            let mut fixture = server(reporter()).await;
            let mut file = probe(&fixture, record);
            let mut target = oid(ObjectType::FILE, 1);
            let mut start = -1;
            let pair = match case {
                0 => {
                    target = oid(ObjectType::FILE, 999);
                    (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)
                }
                1 | 2 => {
                    target = oid(ObjectType::BINARY_VALUE, if case == 1 { 1 } else { 999 });
                    (ErrorClass::SERVICES, ErrorCode::INCONSISTENT_OBJECT_TYPE)
                }
                3 => {
                    file.storage = false;
                    (ErrorClass::SERVICES, ErrorCode::FILE_ACCESS_DENIED)
                }
                4 => {
                    file.mutable_storage = false;
                    (ErrorClass::SERVICES, ErrorCode::FILE_ACCESS_DENIED)
                }
                5 => {
                    file.file.set_read_only(true);
                    file.file.set_file_access_method(999);
                    (ErrorClass::SERVICES, ErrorCode::FILE_ACCESS_DENIED)
                }
                6 => {
                    file.file.set_file_access_method(
                        if record {
                            FileAccessMethod::STREAM_ACCESS
                        } else {
                            FileAccessMethod::RECORD_ACCESS
                        }
                        .to_raw(),
                    );
                    (ErrorClass::SERVICES, ErrorCode::INVALID_FILE_ACCESS_METHOD)
                }
                7 => {
                    start = -2;
                    (ErrorClass::SERVICES, ErrorCode::INVALID_FILE_START_POSITION)
                }
                8 => {
                    file.file.set_max_file_size(2);
                    (ErrorClass::OBJECT, ErrorCode::FILE_FULL)
                }
                _ => {
                    file.bad_position = true;
                    (ErrorClass::DEVICE, ErrorCode::INTERNAL_ERROR)
                }
            };
            fixture.server.db.write().await.add(Box::new(file)).unwrap();
            let before = metadata(&fixture).await;
            if case < 8 {
                // Earlier semantic errors must still win over an oversized request.
                fixture.server.config.atomic_write_file_budget = AtomicWriteFileBudget {
                    max_stream_payload_octets: 1,
                    max_records: 1,
                    max_record_payload_bytes: 1,
                };
            }
            let response = dispatch(
                &fixture.server,
                SERVICE,
                request(target, access(record, start)),
            )
            .await;
            assert_error(&response, pair);
            assert_eq!(
                fixture.attempts.load(Ordering::Acquire),
                usize::from(case >= 8)
            );
            if case != 9 {
                assert_eq!(metadata(&fixture).await, before);
            }
            if matches!(case, 0..=4 | 7..=9) && case != 3 {
                let after = if case == 9 {
                    if record {
                        vec![vec![1], vec![2], vec![9], vec![8]]
                    } else {
                        vec![vec![1, 2, 9, 8]]
                    }
                } else if record {
                    vec![vec![1], vec![2]]
                } else {
                    vec![vec![1, 2]]
                };
                assert_eq!(contents(&fixture, record).await, after);
            }
            settle().await;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), 1, "case {case}");
            assert_eq!(
                records[0].notifications,
                vec![expected(target, 0, Some(pair))]
            );
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_atomic_write_file_backend_mapping_and_unknown_outcome_silence() {
    for record in [false, true] {
        for (error, report) in [
            (
                Error::Protocol {
                    class: 128,
                    code: 512,
                },
                true,
            ),
            (Error::Encoding("backend".into()), true),
            (Error::decoding(0, "backend, not service decoding"), true),
            (Error::OutOfRange("backend".into()), true),
            (Error::Timeout(Duration::from_secs(1)), false),
            (
                Error::Reject {
                    reason: RejectReason::OTHER.to_raw(),
                },
                false,
            ),
            (
                Error::Abort {
                    reason: AbortReason::OUT_OF_RESOURCES.to_raw(),
                },
                false,
            ),
        ] {
            let mut fixture = server(reporter()).await;
            fixture
                .server
                .db
                .write()
                .await
                .add(Box::new(probe(&fixture, record)))
                .unwrap();
            let expected_response =
                BACnetServer::<CaptureTransport>::error_apdu_from_error(77, SERVICE, &error);
            *fixture.execution_error.lock().unwrap() = Some(error);
            let before = metadata(&fixture).await;
            let target = oid(ObjectType::FILE, 1);
            let response = dispatch(
                &fixture.server,
                SERVICE,
                request(target, access(record, -1)),
            )
            .await;
            assert_eq!(wire(&response), wire(&expected_response));
            assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
            assert_eq!(metadata(&fixture).await, before);
            assert_eq!(
                contents(&fixture, record).await,
                if record {
                    vec![vec![1], vec![2]]
                } else {
                    vec![vec![1, 2]]
                }
            );
            settle().await;
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), usize::from(report));
            if report {
                let Apdu::Error(error) = response else {
                    unreachable!()
                };
                assert_eq!(
                    records[0].notifications,
                    vec![expected(
                        target,
                        0,
                        Some((error.error_class, error.error_code))
                    )]
                );
            }
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_atomic_write_file_budget_and_authorization_are_silent() {
    for record in [false, true] {
        for case in 0..5 {
            let mut fixture = file_server(record).await;
            match case {
                0 => {
                    fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::DenyAll
                }
                1 => fixture.server.config.mutation_authorizer = Some(Arc::new(|_| false)),
                2 => {
                    fixture.server.config.mutation_authorizer =
                        Some(Arc::new(|_| panic!("fail closed")))
                }
                3 => {
                    fixture
                        .server
                        .config
                        .atomic_write_file_budget
                        .max_stream_payload_octets = 1;
                    fixture.server.config.atomic_write_file_budget.max_records = 1;
                }
                _ => {
                    fixture
                        .server
                        .config
                        .atomic_write_file_budget
                        .max_stream_payload_octets = 1;
                    fixture
                        .server
                        .config
                        .atomic_write_file_budget
                        .max_record_payload_bytes = 1;
                }
            }
            let before = contents(&fixture, record).await;
            let before_metadata = metadata(&fixture).await;
            let response = dispatch(
                &fixture.server,
                SERVICE,
                request(oid(ObjectType::FILE, 1), access(record, -1)),
            )
            .await;
            if case < 3 {
                assert_error(
                    &response,
                    (ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED),
                );
            } else {
                assert_eq!(
                    wire(&response).as_ref(),
                    &[0x71, 77, AbortReason::OUT_OF_RESOURCES.to_raw()]
                );
            }
            assert_eq!(contents(&fixture, record).await, before);
            assert_eq!(metadata(&fixture).await, before_metadata);
            settle().await;
            assert!(notifications(&fixture.transport.sent).is_empty());
            assert_eq!(
                health(&fixture.server).await,
                Reliability::NO_FAULT_DETECTED
            );
            fixture.server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_reporter_atomic_write_file_selection_levels_priority_and_reporter_bypass() {
    let target = oid(ObjectType::FILE, 1);
    for (selectors, selected) in [
        (None, true),
        (
            Some(vec![
                BACnetObjectSelector::Object(target),
                BACnetObjectSelector::Object(target),
            ]),
            true,
        ),
        (
            Some(vec![
                BACnetObjectSelector::None,
                BACnetObjectSelector::ObjectType(ObjectType::FILE),
            ]),
            true,
        ),
        (Some(vec![]), false),
        (Some(vec![BACnetObjectSelector::None]), false),
        (
            Some(vec![
                BACnetObjectSelector::Object(oid(ObjectType::FILE, 2)),
                BACnetObjectSelector::ObjectType(ObjectType::BINARY_VALUE),
            ]),
            false,
        ),
    ] {
        for level in [
            AuditLevel::NONE,
            AuditLevel::AUDIT_CONFIG,
            AuditLevel::AUDIT_ALL,
        ] {
            for write in [false, true] {
                let mut reporter = reporter();
                reporter.set_audit_level(level).unwrap();
                reporter.set_monitored_objects(selectors.clone()).unwrap();
                reporter
                    .set_audit_priority_filter(BACnetPriorityFilter::empty())
                    .unwrap();
                if !write {
                    reporter
                        .set_auditable_operations(AuditOperationFlags::empty())
                        .unwrap();
                }
                let mut fixture = server(reporter).await;
                fixture
                    .server
                    .db
                    .write()
                    .await
                    .add(Box::new(file_object(false)))
                    .unwrap();
                let emitted = usize::from(selected && level != AuditLevel::NONE && write);
                for start in [-1, -2] {
                    let response = dispatch(
                        &fixture.server,
                        SERVICE,
                        request(target, access(false, start)),
                    )
                    .await;
                    if start == -1 {
                        assert_ack(&response, false, 77, 2);
                    } else {
                        assert_error(
                            &response,
                            (ErrorClass::SERVICES, ErrorCode::INVALID_FILE_START_POSITION),
                        );
                    }
                }
                settle().await;
                let records = notifications(&fixture.transport.sent);
                assert_eq!(records.len(), 2 * emitted);
                if emitted != 0 {
                    assert_eq!(records[0].notifications, vec![expected(target, 0, None)]);
                    assert_eq!(
                        records[1].notifications,
                        vec![expected(
                            target,
                            1,
                            Some((ErrorClass::SERVICES, ErrorCode::INVALID_FILE_START_POSITION))
                        )]
                    );
                }
                assert_eq!(contents(&fixture, false).await, vec![vec![1, 2, 9, 8]]);
                // A decoded wrong-type Reporter target keeps the enabled bypass,
                // but still returns its original service error and never recurses.
                let self_target = oid(ObjectType::AUDIT_REPORTER, 1);
                assert_error(
                    &dispatch(
                        &fixture.server,
                        SERVICE,
                        request(self_target, access(false, -1)),
                    )
                    .await,
                    (ErrorClass::SERVICES, ErrorCode::INCONSISTENT_OBJECT_TYPE),
                );
                settle().await;
                let records = notifications(&fixture.transport.sent);
                assert_eq!(
                    records.len(),
                    2 * emitted + usize::from(level != AuditLevel::NONE)
                );
                if level != AuditLevel::NONE {
                    let last = &records.last().unwrap().notifications;
                    assert_eq!(last.len(), 1);
                    assert_eq!(last[0].target_object, Some(self_target));
                    assert_eq!(
                        last[0].result,
                        Some((ErrorClass::SERVICES, ErrorCode::INCONSISTENT_OBJECT_TYPE))
                    );
                    assert_eq!(last[0].target_property, None);
                }
                fixture.server.stop().await.unwrap();
            }
        }
    }
}
