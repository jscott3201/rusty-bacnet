use super::*;
use bacnet_objects::audit::{
    AuditLogObject, AuditLogPersistence, AuditLogQueryPage, AuditLogSnapshot, AuditLogStorage,
};
use bacnet_types::{
    constructed::{BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult},
    primitives::{Date, Time},
};

pub(super) const SERVICE: ConfirmedServiceChoice = ConfirmedServiceChoice::AUDIT_LOG_QUERY;

pub(super) fn target() -> ObjectIdentifier {
    oid(ObjectType::AUDIT_LOG, 9)
}

pub(super) fn expected_query(
    target: ObjectIdentifier,
    invoke: u8,
    sequence: u16,
    result: Option<(ErrorClass, ErrorCode)>,
) -> BACnetAuditNotification {
    let mut notification = expected(
        target,
        PropertyIdentifier::LOG_BUFFER,
        None,
        invoke,
        sequence,
        result,
    );
    notification.target_property = None;
    notification
}

pub(super) fn query(start: Option<u64>, count: u16) -> AuditLogQueryRequest {
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::READ);
    AuditLogQueryRequest {
        audit_log: target(),
        query_parameters: BACnetAuditLogQueryParameters::BySource {
            source_device_identifier: oid(ObjectType::DEVICE, 1),
            source_device_address: None,
            source_object_identifier: Some(oid(ObjectType::ANALOG_INPUT, 1)),
            operations: Some(operations),
            successful_actions_only: BACnetSuccessFilter::ALL,
        },
        start_at_sequence_number: start,
        requested_count: count,
    }
}

pub(super) fn encode(request: &AuditLogQueryRequest) -> Bytes {
    let mut bytes = BytesMut::new();
    request.try_encode(&mut bytes).unwrap();
    bytes.freeze()
}

pub(super) fn stored_record(sequence: u64, large: bool) -> BACnetAuditLogRecordResult {
    let mut notification = expected_query(target(), 12, 45, None);
    notification.source_device = BACnetRecipient::Device(oid(ObjectType::DEVICE, 1));
    notification.source_object = Some(oid(ObjectType::ANALOG_INPUT, 1));
    notification.source_user_id = Some(123);
    notification.source_user_role = Some(4);
    notification.source_comment = Some("private source".into());
    notification.target_comment = Some(if large {
        "x".repeat(2000)
    } else {
        "private target".into()
    });
    notification.target_property = Some(AuditPropertyReference {
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: Some(7),
    });
    notification.target_priority = Some(5);
    notification.target_value = Some(vec![0x21, 42]);
    notification.current_value = Some(vec![0x21, 41]);
    if sequence == 2 {
        notification.result = Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT));
    }
    BACnetAuditLogRecordResult {
        sequence_number: sequence,
        record: BACnetAuditLogRecord {
            timestamp: (
                Date {
                    year: 126,
                    month: 9,
                    day: 22,
                    day_of_week: 2,
                },
                Time {
                    hour: 12,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
            ),
            datum: BACnetAuditLogDatum::AuditNotification(notification),
        },
    }
}

#[derive(Default)]
pub(super) struct Memory(pub(super) StdMutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for Memory {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

pub(super) struct Probe {
    log: AuditLogObject,
    reads: Arc<AtomicUsize>,
    mode: &'static str,
}

impl BACnetObject for Probe {
    fn object_identifier(&self) -> ObjectIdentifier {
        target()
    }
    fn object_name(&self) -> &str {
        "query log"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
    fn read_property(&self, _: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        panic!("query must not reread properties")
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("query must not mutate the log")
    }
    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        (self.mode != "no capability").then_some(self)
    }
}

impl AuditLogStorage for Probe {
    fn query(
        &self,
        parameters: &BACnetAuditLogQueryParameters,
        start: Option<u64>,
        count: u16,
    ) -> AuditLogQueryPage {
        self.reads.fetch_add(1, Ordering::AcqRel);
        if self.mode == "bad ack" && count != 0 {
            let mut record = stored_record(1, false);
            record.record.datum = BACnetAuditLogDatum::LogStatus(0b1000);
            return AuditLogQueryPage {
                records: vec![record],
                no_more_items: true,
            };
        }
        self.log.query(parameters, start, count)
    }
}

pub(super) async fn add_log(
    fixture: &Fixture,
    count: u64,
    mode: &'static str,
) -> (Arc<AtomicUsize>, Arc<Memory>) {
    let persistence = Arc::new(Memory::default());
    let mut log = AuditLogObject::new(9, "log", 4, persistence.clone()).unwrap();
    for sequence in 1..=count {
        log.add_record(stored_record(sequence, mode == "large").record)
            .unwrap();
    }
    let reads = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(Probe {
            log,
            reads: reads.clone(),
            mode,
        }))
        .unwrap();
    (reads, persistence)
}

pub(super) fn assert_idle(fixture: &Fixture) {
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
}

pub(super) fn request(data: Bytes) -> ConfirmedRequestPdu {
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 77,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: SERVICE,
        service_request: data,
    }
}

pub(super) async fn ingress(
    server: &BACnetServer<CaptureTransport>,
    req: ConfirmedRequestPdu,
    reply_tx: Option<oneshot::Sender<Bytes>>,
) {
    BACnetServer::dispatch(
        &server.db,
        &server.network,
        &server.cov_table,
        &server.seg_ack_senders,
        &server.seg_send_permits,
        &server.cov_in_flight,
        &server.server_tsm,
        &server.notification_transactions,
        &server.confirmed_request_tracker,
        &server.device_bindings,
        &server.comm_state,
        &server.dcc_timer,
        &server.dcc_outcomes,
        &server.mutation_decisions,
        &Arc::new(server.config.clone()),
        &server._clock,
        &server.discovery_limiter,
        &server.time_sync_limiter,
        &server.request_tasks,
        SOURCE,
        Apdu::ConfirmedRequest(req),
        bacnet_network::layer::ReceivedApdu {
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(SOURCE),
            ingress_network: None,
            source_network: None,
            link_layer_group: false,
            is_group: false,
            data_attributes: vec![],
            provenance: bacnet_transport::port::TransportProvenance::unverified(),
            reply_tx,
        },
    )
    .await;
}
