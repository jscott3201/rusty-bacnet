//! Shared fixture and wire helpers for the Audit Reporter behavioral tests.

use super::super::*;
use crate::server::test_transport::{SendMode, SentFrame, StartMode, TestTransport};
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::{
    analog::AnalogInputObject,
    audit::AuditReporterObject,
    binary::BinaryValueObject,
    device::{DeviceConfig, DeviceObject},
    traits::BACnetObject,
};
use bacnet_services::{audit::AuditNotificationRequest, write_property::WritePropertyRequest};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::{
    bitstring::AuditOperationFlags,
    enums::{AuditLevel, AuditOperation},
};
use std::borrow::Cow;
use std::sync::atomic::{AtomicU8, AtomicUsize};
use std::sync::Mutex as StdMutex;

pub(super) const LOGGER: &[u8] = &[2];
pub(super) const NEW_LOGGER: &[u8] = &[4];
pub(super) const SOURCE: &[u8] = &[3];

/// What the audit fixtures' link observed, and switches that steer it. The link
/// itself is the shared [`TestTransport`] built by [`AuditCapture::port`].
#[derive(Clone, Default)]
pub(super) struct AuditCapture {
    pub(super) six_byte_mac: bool,
    /// The link's B/IP broadcast endpoint, which makes it the B/IP link
    /// Address recipients need; sends then may go to any six-octet MAC.
    pub(super) bip_broadcast: Option<std::net::SocketAddrV4>,
    /// Record broadcasts in [`Self::broadcasts`] instead of dropping them.
    pub(super) record_broadcasts: bool,
    pub(super) broadcasts: Arc<StdMutex<Vec<Bytes>>>,
    /// Run the Network Number controls, fed through `incoming`.
    pub(super) number_controls: bool,
    /// The bound B/IP endpoint a registered Network Port publishes.
    pub(super) normal_bip: Option<std::net::SocketAddrV4>,
    pub(super) learned_broadcast: Option<MacAddr>,
    /// A MAC the link reports as a group destination, not its broadcast,
    /// once started (#1493).
    pub(super) learned_group: Option<MacAddr>,
    /// MACs the link's owned group rule takes in from the start, as a B/IP
    /// link's takes in its broadcast IP at any port (#1493).
    pub(super) group_macs: Vec<MacAddr>,
    pub(super) reject_route_callbacks: Arc<AtomicBool>,
    pub(super) route_callbacks: Arc<AtomicUsize>,
    pub(super) started: Arc<AtomicBool>,
    pub(super) sent: Arc<StdMutex<Vec<Bytes>>>,
    pub(super) destinations: Arc<StdMutex<Vec<Vec<u8>>>>,
    pub(super) responses: Arc<StdMutex<Vec<Bytes>>>,
    pub(super) fail: Arc<AtomicBool>,
    pub(super) fail_response: Arc<AtomicBool>,
    pub(super) block: Arc<AtomicBool>,
    pub(super) unblock: Arc<tokio::sync::Notify>,
    requests: Arc<AtomicU8>,
    pub(super) incoming:
        Arc<StdMutex<Option<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>>>>,
}

impl AuditCapture {
    fn route_callback(&self) {
        self.route_callbacks.fetch_add(1, Ordering::AcqRel);
        assert!(
            !self.reject_route_callbacks.load(Ordering::Acquire),
            "target route callback after startup"
        );
    }

    /// The link's `bip_broadcast_endpoint` answer, counted as a route callback.
    pub(super) fn bip_broadcast_endpoint(&self) -> Option<std::net::SocketAddrV4> {
        self.route_callback();
        self.bip_broadcast
    }

    /// The link's `is_broadcast_mac` answer, counted as a route callback.
    pub(super) fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        self.route_callback();
        self.started.load(Ordering::Acquire)
            && self
                .learned_broadcast
                .as_ref()
                .is_some_and(|broadcast| broadcast.as_slice() == mac)
    }

    /// The link's answer for a group destination beyond its broadcast,
    /// counted as a route callback.
    pub(super) fn is_learned_group(&self, mac: &[u8]) -> bool {
        self.route_callback();
        self.started.load(Ordering::Acquire)
            && self
                .learned_group
                .as_ref()
                .is_some_and(|group| group.as_slice() == mac)
    }

    async fn send(self: Arc<Self>, frame: SentFrame) -> Result<(), Error> {
        if frame.broadcast {
            self.broadcasts.lock().unwrap().push(frame.npdu);
            return Ok(());
        }
        let (bytes, mac) = (frame.npdu, frame.mac);
        if mac.as_slice() == SOURCE {
            self.responses.lock().unwrap().push(bytes);
            if self.fail_response.load(Ordering::Acquire) {
                return Err(Error::Encoding("injected response send failure".into()));
            }
            return Ok(());
        }
        assert!(
            mac.as_slice() == LOGGER
                || mac.as_slice() == NEW_LOGGER
                || (self.bip_broadcast.is_some() && mac.len() == 6)
        );
        self.destinations.lock().unwrap().push(mac.to_vec());
        self.sent.lock().unwrap().push(bytes);
        if self.block.load(Ordering::Acquire) {
            self.unblock.notified().await;
        }
        if self.fail.load(Ordering::Acquire) {
            return Err(Error::Encoding("injected send failure".into()));
        }
        Ok(())
    }

    /// Build a link that reports into this capture. Unicasts go to [`Self::send`],
    /// broadcasts succeed, recorded only when asked, and the capture rides
    /// along as the transport's state so helpers holding only the server can
    /// reach it.
    pub(super) fn port(&self) -> TestTransport {
        let start = match self.incoming.lock().unwrap().take() {
            Some(incoming) => StartMode::Inbound(Some(incoming)),
            None => StartMode::Closed,
        };
        let local_mac: &[u8] = if self.six_byte_mac {
            &[127, 0, 0, 1, 0xba, 0xc0]
        } else {
            &[1]
        };
        let capture = Arc::new(self.clone());
        let (on_start, on_routes, on_groups, on_endpoint, on_send) = (
            Arc::clone(&capture),
            Arc::clone(&capture),
            Arc::clone(&capture),
            Arc::clone(&capture),
            Arc::clone(&capture),
        );
        let mut builder = TestTransport::builder()
            .local_mac(local_mac)
            .start(start)
            .broadcast(if self.record_broadcasts {
                SendMode::Record
            } else {
                SendMode::Ignore
            })
            .on_start(move || on_start.started.store(true, Ordering::Release))
            .on_is_broadcast_mac(move |mac| on_routes.is_broadcast_mac(mac))
            .on_is_group_destination(move |mac| on_groups.is_learned_group(mac))
            .on_bip_broadcast_endpoint(move || on_endpoint.bip_broadcast_endpoint())
            .on_send(move |frame| Arc::clone(&on_send).send(frame));
        for group in &self.group_macs {
            builder = builder.group_mac(group);
        }
        if self.number_controls {
            builder = builder.number_controls();
        }
        if let Some(endpoint) = self.normal_bip {
            builder = builder.normal_bip(endpoint);
        }
        builder.state(capture).build()
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
    reporter.set_auditable_operations(operations).unwrap();
    reporter
}

/// Counts write attempts and commits on the wrapped object, and fails the next
/// write with a scripted error when one is set.
struct CountingValue<O = BinaryValueObject> {
    value: O,
    writes: Arc<AtomicUsize>,
    attempts: Arc<AtomicUsize>,
    execution_error: Arc<StdMutex<Option<Error>>>,
}

impl<O: BACnetObject> BACnetObject for CountingValue<O> {
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
    fn write_property_from(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &bacnet_objects::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        self.attempts.fetch_add(1, Ordering::AcqRel);
        if let Some(error) = self.execution_error.lock().unwrap().take() {
            return Err(error);
        }
        self.value
            .write_property_from(property, index, value, priority, origin)?;
        self.writes.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.value.property_list()
    }
}

pub(super) struct Fixture {
    pub(super) server: BACnetServer<TestTransport>,
    pub(super) transport: AuditCapture,
    pub(super) writes: Arc<AtomicUsize>,
    pub(super) attempts: Arc<AtomicUsize>,
    pub(super) execution_error: Arc<StdMutex<Option<Error>>>,
}

impl Fixture {
    /// Wrap `object` so its writes share this fixture's attempt and commit
    /// counters and its scripted execution error.
    pub(super) fn counting(&self, object: impl BACnetObject + 'static) -> Box<dyn BACnetObject> {
        Box::new(CountingValue {
            value: object,
            writes: Arc::clone(&self.writes),
            attempts: Arc::clone(&self.attempts),
            execution_error: Arc::clone(&self.execution_error),
        })
    }
}

pub(super) async fn server(reporter: AuditReporterObject) -> Fixture {
    server_with_recipient(
        reporter,
        bacnet_types::constructed::BACnetRecipient::Device(oid(ObjectType::DEVICE, 20)),
    )
    .await
}

pub(super) async fn server_with_recipient(
    reporter: AuditReporterObject,
    recipient: bacnet_types::constructed::BACnetRecipient,
) -> Fixture {
    try_server(
        reporter,
        &[10],
        Some(recipient),
        vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()],
    )
    .await
    .unwrap()
}

pub(super) async fn try_server(
    reporter: AuditReporterObject,
    devices: &[u32],
    recipient: Option<bacnet_types::constructed::BACnetRecipient>,
    bindings: Vec<DeviceBinding>,
) -> Result<Fixture, Error> {
    try_servers(vec![reporter], devices, recipient, bindings).await
}

pub(super) async fn try_servers(
    reporters: Vec<AuditReporterObject>,
    devices: &[u32],
    recipient: Option<bacnet_types::constructed::BACnetRecipient>,
    bindings: Vec<DeviceBinding>,
) -> Result<Fixture, Error> {
    try_servers_profile(reporters, devices, recipient, bindings, true).await
}

pub(super) async fn plain_server(reporter: AuditReporterObject) -> Fixture {
    try_servers_profile(vec![reporter], &[10], None, vec![], false)
        .await
        .unwrap()
}

async fn try_servers_profile(
    reporters: Vec<AuditReporterObject>,
    devices: &[u32],
    recipient: Option<bacnet_types::constructed::BACnetRecipient>,
    bindings: Vec<DeviceBinding>,
    enabled: bool,
) -> Result<Fixture, Error> {
    try_servers_config(
        reporters,
        devices,
        recipient,
        bindings,
        enabled,
        1476,
        AuditCapture::default(),
    )
    .await
}
pub(super) async fn try_servers_config(
    reporters: Vec<AuditReporterObject>,
    devices: &[u32],
    recipient: Option<bacnet_types::constructed::BACnetRecipient>,
    bindings: Vec<DeviceBinding>,
    enabled: bool,
    max_apdu_length: u32,
    transport: AuditCapture,
) -> Result<Fixture, Error> {
    let mut db = ObjectDatabase::new();
    let writes = Arc::new(AtomicUsize::new(0));
    let attempts = Arc::new(AtomicUsize::new(0));
    let execution_error = Arc::new(StdMutex::new(None));
    for &instance in devices {
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(instance, 0)
                    .expect("valid Device identity")
                    .with_name(format!("Device {instance}"))
                    .with_max_apdu(
                        u16::try_from(max_apdu_length.min(1476)).expect("Device APDU fits u16"),
                    )
                    .expect("valid Device APDU"),
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
    }
    for &instance in devices {
        if let Some(recipient) = &recipient {
            db.get_mut(&oid(ObjectType::DEVICE, instance))
                .unwrap()
                .device_authority_internal()
                .unwrap()
                .provision_audit_recipient(recipient.clone())?;
        }
    }
    db.add(Box::new(CountingValue {
        value: BinaryValueObject::new(1, "value").unwrap(),
        writes: Arc::clone(&writes),
        attempts: Arc::clone(&attempts),
        execution_error: Arc::clone(&execution_error),
    }))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "input", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(2, "other-value").unwrap()))
        .unwrap();
    let identities = reporters
        .iter()
        .map(BACnetObject::object_identifier)
        .collect();
    for reporter in reporters {
        db.add(Box::new(reporter)).unwrap();
    }
    let server = BACnetServer::start_with_clock_mode_and_bindings(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(
                devices.iter().copied().min().unwrap_or(1),
                0,
            )
            .unwrap()
            .with_max_apdu(u16::try_from(max_apdu_length).expect("Device APDU fits u16"))
            .expect("valid Device APDU"),
            audit_reporters: enabled.then_some(AuditReportersConfig {
                reporters: identities,
            }),
            ..Default::default()
        },
        db,
        transport.port(),
        None,
        bindings,
    )
    .await?;
    Ok(Fixture {
        server,
        transport,
        writes,
        attempts,
        execution_error,
    })
}

pub(super) async fn dispatch(
    server: &BACnetServer<TestTransport>,
    service: ConfirmedServiceChoice,
    data: Bytes,
) -> Apdu {
    dispatch_optional(server, service, data)
        .await
        .expect("response")
}

pub(super) async fn dispatch_optional(
    server: &BACnetServer<TestTransport>,
    service: ConfirmedServiceChoice,
    data: Bytes,
) -> Option<Apdu> {
    dispatch_from(server, service, data, SOURCE, None).await
}

pub(super) async fn dispatch_from(
    server: &BACnetServer<TestTransport>,
    service: ConfirmedServiceChoice,
    data: Bytes,
    source_mac: &[u8],
    source_network: Option<NpduAddress>,
) -> Option<Apdu> {
    let (tx, rx) = oneshot::channel();
    let invoke_id = 77u8.wrapping_add(
        server
            .test_network()
            .transport()
            .state::<AuditCapture>()
            .requests
            .fetch_add(1, Ordering::AcqRel),
    );
    BACnetServer::handle_confirmed_request(
        &server.test_services(),
        &server.confirmed_request_tracker,
        &server.request_tasks.spawner(),
        source_mac,
        source_network,
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
    rx.await
        .ok()
        .map(|bytes| decode_apdu(decode_npdu(bytes).unwrap().payload).unwrap())
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
    .encode(&mut data)
    .unwrap();
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
