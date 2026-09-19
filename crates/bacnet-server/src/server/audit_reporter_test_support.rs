//! Shared fixture and wire helpers for the Audit Reporter behavioral tests.

use super::super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::{
    analog::AnalogInputObject,
    audit::AuditReporterObject,
    binary::BinaryValueObject,
    device::{DeviceConfig, DeviceObject},
    traits::BACnetObject,
};
use bacnet_services::{audit::AuditNotificationRequest, write_property::WritePropertyRequest};
use bacnet_types::{
    bitstring::AuditOperationFlags,
    enums::{AuditLevel, AuditOperation},
};
use std::borrow::Cow;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex as StdMutex;

pub(super) const LOGGER: &[u8] = &[2];
pub(super) const SOURCE: &[u8] = &[3];

#[derive(Clone, Default)]
pub(super) struct CaptureTransport {
    pub(super) started: Arc<AtomicBool>,
    pub(super) sent: Arc<StdMutex<Vec<Bytes>>>,
    pub(super) fail: Arc<AtomicBool>,
    pub(super) block: Arc<AtomicBool>,
    pub(super) unblock: Arc<tokio::sync::Notify>,
    requests: Arc<AtomicU8>,
}

impl TransportPort for CaptureTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        self.started.store(true, Ordering::Release);
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
            self.unblock.notified().await;
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

pub(super) fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

pub(super) fn reporter() -> AuditReporterObject {
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
    attempts: Arc<AtomicUsize>,
    execution_error: Arc<StdMutex<Option<Error>>>,
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
        self.attempts.fetch_add(1, Ordering::AcqRel);
        if let Some(error) = self.execution_error.lock().unwrap().take() {
            return Err(error);
        }
        self.value
            .write_property(property, index, value, priority)?;
        self.writes.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.value.property_list()
    }
}

pub(super) struct Fixture {
    pub(super) server: BACnetServer<CaptureTransport>,
    pub(super) transport: CaptureTransport,
    pub(super) writes: Arc<AtomicUsize>,
    pub(super) attempts: Arc<AtomicUsize>,
    pub(super) execution_error: Arc<StdMutex<Option<Error>>>,
}

pub(super) async fn server(reporter: AuditReporterObject) -> Fixture {
    let mut db = ObjectDatabase::new();
    let writes = Arc::new(AtomicUsize::new(0));
    let attempts = Arc::new(AtomicUsize::new(0));
    let execution_error = Arc::new(StdMutex::new(None));
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
        attempts: Arc::clone(&attempts),
        execution_error: Arc::clone(&execution_error),
    }))
    .unwrap();
    db.add(Box::new(AnalogInputObject::new(1, "input", 0).unwrap()))
        .unwrap();
    db.add(Box::new(BinaryValueObject::new(2, "other-value").unwrap()))
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
        attempts,
        execution_error,
    }
}

pub(super) async fn dispatch(
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

pub(super) fn wp(
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

pub(super) fn notifications(sent: &StdMutex<Vec<Bytes>>) -> Vec<AuditNotificationRequest> {
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
