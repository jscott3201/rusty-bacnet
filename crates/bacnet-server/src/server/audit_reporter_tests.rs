use super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::{analog::AnalogInputObject, traits::BACnetObject};
use bacnet_objects::{
    audit::AuditReporterObject,
    binary::BinaryValueObject,
    device::{DeviceConfig, DeviceObject},
};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::{audit::AuditNotificationRequest, write_property::WritePropertyRequest};
use bacnet_types::{
    bitstring::{AuditOperationFlags, BACnetPriorityFilter},
    constructed::{
        AuditPropertyReference, BACnetAddress, BACnetAuditNotification, BACnetRecipient,
    },
    enums::{AuditLevel, AuditOperation, Reliability},
    primitives::BACnetTimeStamp,
};
use std::borrow::Cow;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex as StdMutex;

const LOGGER: &[u8] = &[2];
const SOURCE: &[u8] = &[3];

#[derive(Clone, Default)]
struct CaptureTransport {
    sent: Arc<StdMutex<Vec<Bytes>>>,
    fail: Arc<AtomicBool>,
    block: Arc<AtomicBool>,
    requests: Arc<AtomicU8>,
}

impl TransportPort for CaptureTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        Ok(mpsc::channel(1).1)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, bytes: &[u8], mac: &[u8]) -> Result<(), Error> {
        assert_eq!(mac, LOGGER);
        self.sent
            .lock()
            .unwrap()
            .push(Bytes::copy_from_slice(bytes));
        if self.block.load(Ordering::Acquire) {
            std::future::pending::<()>().await;
        }
        if self.fail.load(Ordering::Acquire) {
            return Err(Error::Encoding("injected send failure".into()));
        }
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

fn reporter() -> AuditReporterObject {
    let mut reporter = AuditReporterObject::new(1, "reporter").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    reporter.set_auditable_operations(operations);
    reporter
}

struct CountingValue {
    value: BinaryValueObject,
    writes: Arc<AtomicUsize>,
}

impl BACnetObject for CountingValue {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.value.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.value.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.value.read_property(property, index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.value
            .write_property(property, index, value, priority)?;
        self.writes.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.value.property_list()
    }
}

struct Fixture {
    server: BACnetServer<CaptureTransport>,
    transport: CaptureTransport,
    writes: Arc<AtomicUsize>,
}

async fn server(reporter: AuditReporterObject) -> Fixture {
    let mut db = ObjectDatabase::new();
    let writes = Arc::new(AtomicUsize::new(0));
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 10,
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(CountingValue {
        value: BinaryValueObject::new(1, "value").unwrap(),
        writes: Arc::clone(&writes),
    }))
    .unwrap();
    db.add(Box::new(AnalogInputObject::new(1, "input", 0).unwrap()))
        .unwrap();
    db.add(Box::new(reporter)).unwrap();
    let transport = CaptureTransport::default();
    let captured = transport.clone();
    let server = BACnetServer::start_with_clock_mode_and_bindings(
        ServerConfig {
            audit_reporter: Some(AuditReporterConfig {
                reporter: oid(ObjectType::AUDIT_REPORTER, 1),
                recipient: Some(oid(ObjectType::DEVICE, 20)),
            }),
            ..Default::default()
        },
        db,
        transport,
        None,
        vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()],
    )
    .await
    .unwrap();
    Fixture {
        server,
        transport: captured,
        writes,
    }
}

async fn dispatch(
    server: &BACnetServer<CaptureTransport>,
    service: ConfirmedServiceChoice,
    data: Bytes,
) -> Apdu {
    let (tx, rx) = oneshot::channel();
    let invoke_id = 77u8.wrapping_add(
        server
            .network
            .transport()
            .requests
            .fetch_add(1, Ordering::AcqRel),
    );
    BACnetServer::handle_confirmed_request(
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
        &server.config,
        &server.request_tasks.spawner(),
        SOURCE,
        None,
        ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: data,
        },
        Some(tx),
    )
    .await;
    decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap()
}

fn wp(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    value: Vec<u8>,
    priority: Option<u8>,
) -> Bytes {
    let mut data = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: None,
        property_value: value,
        priority,
    }
    .encode(&mut data);
    data.freeze()
}

fn notifications(sent: &StdMutex<Vec<Bytes>>) -> Vec<AuditNotificationRequest> {
    sent.lock()
        .unwrap()
        .iter()
        .map(|bytes| {
            let npdu = decode_npdu(bytes.clone()).unwrap();
            match decode_apdu(npdu.payload).unwrap() {
                Apdu::UnconfirmedRequest(request) => {
                    assert_eq!(
                        request.service_choice,
                        UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
                    );
                    AuditNotificationRequest::decode(&request.service_request).unwrap()
                }
                other => panic!("unexpected {other:?}"),
            }
        })
        .collect()
}

#[tokio::test]
async fn audit_reporter_wp_emits_one_success_after_commit() {
    let mut fixture = server(reporter()).await;
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            vec![0x91, 1],
            None,
        ),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications.len(), 1);
    assert_eq!(
        records[0].notifications[0],
        BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: Some(BACnetTimeStamp::SequenceNumber(0)),
            source_device: BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(SOURCE)
            }),
            source_object: None,
            operation: AuditOperation::WRITE,
            source_comment: None,
            target_comment: None,
            invoke_id: Some(77),
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
            target_object: Some(oid(ObjectType::BINARY_VALUE, 1)),
            target_property: Some(AuditPropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None
            }),
            target_priority: Some(16),
            target_value: Some(vec![0x91, 1]),
            current_value: Some(vec![0x91, 0]),
            result: None,
        }
    );
    assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
}

async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

async fn health(server: &BACnetServer<CaptureTransport>) -> Reliability {
    let db = server.db.read().await;
    let value = db
        .get(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    let PropertyValue::Enumerated(value) = value else {
        panic!("unexpected reliability")
    };
    Reliability::from_raw(value)
}

async fn write_value(server: &BACnetServer<CaptureTransport>, priority: Option<u8>) -> Apdu {
    dispatch(
        server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            vec![0x91, 1],
            priority,
        ),
    )
    .await
}

#[tokio::test]
async fn audit_reporter_filters_disabled_write_bit_and_priority_without_filtering_description() {
    for (enabled, write_bit, priority, expected) in [
        (false, true, None, 0),
        (true, false, None, 0),
        (true, true, Some(1), 1),
        (true, true, Some(2), 0),
        (true, true, Some(15), 0),
        (true, true, Some(16), 1),
        (true, true, None, 1),
    ] {
        let mut reporter = reporter();
        if !enabled {
            reporter.set_audit_level(AuditLevel::NONE).unwrap();
        }
        if !write_bit {
            reporter.set_auditable_operations(AuditOperationFlags::empty());
        }
        reporter.set_audit_priority_filter(BACnetPriorityFilter::from_bits(0x8001));
        let mut fixture = server(reporter).await;
        assert!(matches!(
            write_value(&fixture.server, priority).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_eq!(notifications(&fixture.transport.sent).len(), expected);
        assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
        fixture.server.stop().await.unwrap();
    }
    let mut reporter = reporter();
    reporter.set_audit_priority_filter(BACnetPriorityFilter::empty());
    let mut fixture = server(reporter).await;
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::DESCRIPTION,
                vec![0x72, 0, b'x'],
                Some(2)
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications[0].target_priority, None);
    fixture.server.stop().await.unwrap();
}

fn wpm(properties: Vec<BACnetPropertyValue>) -> Bytes {
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid(ObjectType::BINARY_VALUE, 1),
            list_of_properties: properties,
        }],
    }
    .encode(&mut bytes);
    bytes.freeze()
}

fn element(property: PropertyIdentifier, value: Vec<u8>) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value,
        priority: None,
    }
}

#[tokio::test]
async fn audit_reporter_wpm_reports_each_committed_element_not_failed_or_later_elements() {
    let mut fixture = server(reporter()).await;
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm(vec![
            element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]),
            element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 0]),
            element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 9]),
            element(PropertyIdentifier::DESCRIPTION, vec![0x72, 0, b'x']),
        ]),
    )
    .await;
    assert!(matches!(response, Apdu::Error(_)));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].notifications[0].target_value,
        Some(vec![0x91, 1])
    );
    assert_eq!(
        records[0].notifications[0].current_value,
        Some(vec![0x91, 0])
    );
    assert_eq!(
        records[1].notifications[0].target_value,
        Some(vec![0x91, 0])
    );
    assert_eq!(
        records[1].notifications[0].current_value,
        Some(vec![0x91, 1])
    );
    assert!(records.iter().all(
        |request| request.notifications.len() == 1 && request.notifications[0].result.is_none()
    ));
    assert_eq!(fixture.writes.load(Ordering::Acquire), 2);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_denied_wp_and_wpm_suffix_have_no_audit_side_effects() {
    let mut fixture = server(reporter()).await;
    fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::DenyAll;
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::Error(_)
    ));
    settle().await;
    assert!(fixture.transport.sent.lock().unwrap().is_empty());
    assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
    fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::Permissive;
    fixture.server.config.mutation_authorizer = Some(Arc::new(|context| match &context.target {
        crate::mutation::MutationTarget::WritePropertyMultiple(attempt) => {
            attempt.reference.property_identifier == PropertyIdentifier::PRESENT_VALUE.to_raw()
        }
        _ => false,
    }));
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            wpm(vec![
                element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]),
                element(PropertyIdentifier::DESCRIPTION, vec![0x72, 0, b'x']),
                element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 0]),
            ])
        )
        .await,
        Apdu::Error(_)
    ));
    settle().await;
    assert_eq!(notifications(&fixture.transport.sent).len(), 1);
    assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_missing_recipient_is_configuration_failure_without_growth() {
    for recipient in [None, Some(oid(ObjectType::DEVICE, 999))] {
        let mut fixture = server(reporter()).await;
        fixture
            .server
            .config
            .audit_reporter
            .as_mut()
            .unwrap()
            .recipient = recipient;
        for _ in 0..100 {
            assert!(matches!(
                write_value(&fixture.server, None).await,
                Apdu::SimpleAck(_)
            ));
        }
        settle().await;
        assert_eq!(
            health(&fixture.server).await,
            Reliability::CONFIGURATION_ERROR
        );
        assert!(fixture.transport.sent.lock().unwrap().is_empty());
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture.server.notification_transactions.workers_empty());
        assert_eq!(fixture.writes.load(Ordering::Acquire), 100);
        fixture.server.stop().await.unwrap();
    }
}

fn confirmed_notification(sent: &StdMutex<Vec<Bytes>>, index: usize) -> ConfirmedRequestPdu {
    match decode_apdu(
        decode_npdu(sent.lock().unwrap()[index].clone())
            .unwrap()
            .payload,
    )
    .unwrap()
    {
        Apdu::ConfirmedRequest(request) => {
            assert_eq!(
                request.service_choice,
                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
            );
            request
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_missing_ack_and_failed_delivery_are_communication_failures_and_recover() {
    let mut reporter = reporter();
    reporter.set_issue_confirmed_notifications(true);
    let mut fixture = server(reporter).await;
    write_value(&fixture.server, None).await;
    settle().await;
    let first = confirmed_notification(&fixture.transport.sent, 0);
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id: first.invoke_id,
        service_choice: first.service_choice,
    });
    assert!(!fixture
        .server
        .notification_transactions
        .admit_terminal(SOURCE, None, &ack));
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    fixture.transport.fail.store(true, Ordering::Release);
    write_value(&fixture.server, None).await;
    settle().await;
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    fixture.transport.fail.store(false, Ordering::Release);
    write_value(&fixture.server, None).await;
    settle().await;
    let request = confirmed_notification(&fixture.transport.sent, 2);
    assert!(fixture.server.notification_transactions.admit_terminal(
        LOGGER,
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice
        })
    ));
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    assert_eq!(fixture.writes.load(Ordering::Acquire), 3);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_self_write_is_one_notification_and_sensor_sampling_is_excluded() {
    let mut reporter = reporter();
    reporter.set_auditable_operations(AuditOperationFlags::empty());
    let mut fixture = server(reporter).await;
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::AUDIT_REPORTER, 1),
                PropertyIdentifier::DESCRIPTION,
                vec![0x72, 0, b'x'],
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    fixture
        .server
        .set_present_value_local(&oid(ObjectType::ANALOG_INPUT, 1), PropertyValue::Real(12.0))
        .await
        .unwrap();
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].notifications[0].target_object,
        Some(oid(ObjectType::AUDIT_REPORTER, 1))
    );
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_overflow_and_shutdown_with_inflight_send_are_bounded() {
    let mut reporter = reporter();
    reporter.set_issue_confirmed_notifications(true);
    let mut fixture = server(reporter).await;
    fixture.transport.block.store(true, Ordering::Release);
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm((0..65)
            .map(|_| element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]))
            .collect()),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)));
    settle().await;
    assert_eq!(fixture.writes.load(Ordering::Acquire), 65);
    assert_eq!(fixture.transport.sent.lock().unwrap().len(), 64);
    assert_eq!(fixture.server.notification_transactions.active_count(), 64);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    tokio::time::timeout(Duration::from_secs(1), fixture.server.stop())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    assert!(fixture.server.notification_transactions.workers_empty());
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_blocked_transport_has_a_total_deadline_even_without_ack_wait() {
    for confirmed in [false, true] {
        let mut reporter = reporter();
        reporter.set_issue_confirmed_notifications(confirmed);
        let mut fixture = server(reporter).await;
        fixture.transport.block.store(true, Ordering::Release);
        write_value(&fixture.server, None).await;
        settle().await;
        assert_eq!(fixture.transport.sent.lock().unwrap().len(), 1);
        tokio::time::advance(Duration::from_secs(3)).await;
        settle().await;
        assert_eq!(
            health(&fixture.server).await,
            Reliability::COMMUNICATION_FAILURE
        );
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert!(fixture.server.notification_transactions.workers_empty());
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_known_source_array_coordinate_and_large_value_policy() {
    let mut fixture = server(reporter()).await;
    fixture
        .server
        .device_bindings
        .write()
        .await
        .insert_configured(
            DeviceBinding::local(oid(ObjectType::DEVICE, 30), SOURCE).unwrap(),
            |_| false,
        )
        .unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::BINARY_VALUE, 1),
        property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
        property_array_index: Some(8),
        property_value: vec![0x91, 1],
        priority: None,
    }
    .encode(&mut request);
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request.freeze()
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    let record = &records[0].notifications[0];
    assert_eq!(
        record.source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 30))
    );
    assert_eq!(
        record
            .target_property
            .as_ref()
            .unwrap()
            .property_array_index,
        Some(8)
    );
    assert_eq!(record.current_value, Some(vec![0]));
    assert_eq!(record.target_priority, None);
    for size in [29, 30, 31, 1000] {
        let mut encoded = BytesMut::new();
        encode_property_value(
            &mut encoded,
            &PropertyValue::CharacterString("x".repeat(size)),
        )
        .unwrap();
        let included = (encoded.len() <= 32).then(|| encoded.to_vec());
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::DESCRIPTION,
                encoded.to_vec(),
                None,
            ),
        )
        .await;
        settle().await;
        assert_eq!(
            notifications(&fixture.transport.sent)
                .last()
                .unwrap()
                .notifications[0]
                .target_value,
            included
        );
    }
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_noncommandable_present_value_ignores_priority_filter() {
    let mut reporter = reporter();
    reporter.set_audit_priority_filter(BACnetPriorityFilter::empty());
    let mut fixture = server(reporter).await;
    fixture
        .server
        .write_local(
            &oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .await
        .unwrap();
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::ANALOG_INPUT, 1),
            PropertyIdentifier::PRESENT_VALUE,
            vec![0x44, 0x41, 0x20, 0, 0],
            Some(8),
        ),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications[0].target_priority, None);
    assert_eq!(
        records[0].notifications[0].target_value,
        Some(vec![0x44, 0x41, 0x20, 0, 0])
    );
    fixture.server.stop().await.unwrap();
}
