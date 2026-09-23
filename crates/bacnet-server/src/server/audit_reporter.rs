use super::device_bindings::BindingFreshness;
use super::event_recipient_route::{ConfirmedRecipientRoute, RecipientRoute};
use super::notification_transactions::{
    AuditFailureContext, AuditFailureTicket, NotificationReservation, NotificationReserveError,
};
use super::*;
use crate::handlers::{WriteCommitObserver, WriteTarget};
use bacnet_objects::audit::AuditReporterStatus;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAddress, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditLevel, AuditOperation};

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[path = "audit_reporter_read.rs"]
mod read;

/// One locally configured target READ/WRITE/CREATE/DELETE Reporter and unicast recipient.
///
/// Reports successful inbound WP/WPM elements, AddListElement/RemoveListElement,
/// AtomicWriteFile, CreateObject/DeleteObject operations, and authorized execution errors.
/// Policy denials and undecoded/unattempted elements remain silent. Successes
/// omit Result; execution failures include the mapped BACnet Error. There is no
/// source-side reporting, per-object override, batching, forwarding, or durable outbox.
/// Provision the typed recipient on the built-in Device before startup; configure
/// Device routes with `device_binding`. Startup requires exactly one concrete
/// built-in Device, a provision, and the selected Reporter capability. Unresolved
/// configured Device routes permit startup with CONFIGURATION_ERROR. Address
/// choices require direct unicast IPv4 B/IP. The active Device recipient is
/// required/writable; actual local or authorized WP/WPM changes atomically reserve
/// old/new attempts before commit, independently of ordinary reporting filters.
/// Both routes must be usable; an unavailable old route requires reconfiguration
/// and restart. Active Device/Reporter membership is protected until quiescence.
/// At most 64 deliveries are active per server, with no ordinary-record queue. Each
/// send/ACK has one total three-second deadline and no retries. Overflow or
/// delivery failure sets COMMUNICATION_FAILURE, never changes the write result,
/// and retains no ordinary record. Resource-admission drops can be summarized
/// by one memory-only, saturating AUDITING_FAILURE count when its operation bit
/// and Audit_Level are enabled. One owned worker waits for capacity and coalesces
/// further drops; summary failure never counts itself. Values over 32 octets are omitted.
/// The complete APDU must fit the server's limit; outbound segmentation is not
/// implemented by this profile. Encoding/size failures are delivery failures.
/// A subsequent successful delivery clears communication failure unless a newer
/// failure occurred after that delivery began. Unconfirmed success proves only
/// transport acceptance, not storage by the recipient.
///
/// Audit_Source_Reporter remains false. Per-object overrides, source reporting,
/// ordinary direct local-write reporting and ordinary batching remain unsupported.
/// Recipient changes are the explicit local-write exception.
/// Ordinary sensor samples and internal reliability updates never enter this
/// producer. An enabled external write to a Reporter produces one record.
/// Locally configured Monitored_Objects selects ordinary targets by exact object
/// or object type. Omitted selection preserves catch-all behavior; an empty or
/// all-NULL selection reports no ordinary targets. Reporter writes bypass it.
/// Network selection writes and multi-Reporter arbitration are not supported.
/// CREATE/DELETE require their operation bit, count as configuration operations,
/// and ignore the priority filter. Records use the final/candidate created OID or
/// captured deleted OID, with no property, priority, or values; initial values do
/// not generate WRITE records. A failed by-type creation without an assigned,
/// representable OID omits the target; only catch-all or type selection can match.
/// Deleting or replacing the selected Reporter is denied until the target
/// runtime has stopped and released membership protection.
///
/// List edits use WRITE, object/property/requested-index identity, no priority,
/// the requested delta as raw Target_Value, and the known pre-image as Current_Value.
/// Empty, structurally invalid, or over-32-octet values are omitted, never wrapped
/// or truncated. Element and framed-list decoding precedes observation; valid
/// execution failures retain the response-mapped Result. Successful no-op removals
/// still report once. AUDIT_CONFIG admits implemented non-Present_Value lists;
/// list services ignore priority filtering and reuse ordinary target selection.
/// AtomicWriteFile uses WRITE and the known target OID, with no property,
/// priority, or values (Table 19-5). AUDIT_CONFIG and AUDIT_ALL admit file writes;
/// priority filtering is irrelevant. Admission follows the existing service
/// decoder acceptance boundary, including tolerated trailing bytes, not strict
/// input consumption. Decoder rejections and configured payload/count budget
/// Aborts remain silent. Inbound WriteGroup remains unsupported by design, so
/// complete WRITE coverage is not claimed.
///
/// READ covers completed, unsegmented RP/RPM responses, one record per
/// returned property outcome in result order, including inline RPM errors.
/// Read values, comments, and priorities are always omitted. READ requires its
/// operation bit; AUDIT_CONFIG excludes Present_Value. RPM's existing result
/// budget bounds provisional intents, which are discarded on whole-request
/// failure. Admission follows release of the read guard and uses the same
/// immediate drop/summary policy, without an additional cap or queue.
///
/// ```no_run
/// use bacnet_objects::{audit::AuditReporterObject, database::ObjectDatabase,
///     device::{DeviceConfig, DeviceObject}};
/// use bacnet_server::server::{AuditReporterConfig, BACnetServer, DeviceBinding};
/// use bacnet_types::{bitstring::AuditOperationFlags, enums::{AuditLevel,
///     AuditOperation, ObjectType}, primitives::ObjectIdentifier, constructed::BACnetRecipient};
/// # async fn example() -> Result<(), bacnet_types::error::Error> {
/// let mut db = ObjectDatabase::new();
/// let recipient = ObjectIdentifier::new(ObjectType::DEVICE, 200)?;
/// let mut device = DeviceObject::new(DeviceConfig::default())?;
/// device.provision_audit_recipient(BACnetRecipient::Device(recipient))?;
/// db.add(Box::new(device))?;
/// let mut reporter = AuditReporterObject::new(1, "Target writes")?;
/// reporter.set_audit_level(AuditLevel::AUDIT_ALL)?;
/// let mut operations = AuditOperationFlags::empty();
/// operations.insert(AuditOperation::WRITE);
/// reporter.set_auditable_operations(operations);
/// reporter.set_issue_confirmed_notifications(true);
/// db.add(Box::new(reporter))?;
/// let mut server = BACnetServer::builder().database(db)
///     .audit_reporter(AuditReporterConfig {
///         reporter: ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1)?,
///     })
///     .device_binding(DeviceBinding::local(recipient, [127, 0, 0, 1, 0xBA, 0xC1])?)?
///     .build().await?;
/// server.stop().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct AuditReporterConfig {
    /// The one AuditReporterObject selected for this server.
    pub reporter: ObjectIdentifier,
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Enable the narrow target audit profile; see [`AuditReporterConfig`].
    pub fn audit_reporter(mut self, profile: AuditReporterConfig) -> Self {
        self.config.audit_reporter = Some(profile);
        self
    }
}

impl BipServerBuilder {
    /// Enable the narrow target audit profile; see [`AuditReporterConfig`].
    pub fn audit_reporter(mut self, profile: AuditReporterConfig) -> Self {
        self.config.audit_reporter = Some(profile);
        self
    }
}

fn local_device(db: &ObjectDatabase) -> Option<ObjectIdentifier> {
    let mut devices = db
        .list_objects()
        .into_iter()
        .filter(|oid| oid.object_type() == ObjectType::DEVICE);
    let device = devices.next()?;
    devices.next().is_none().then_some(device)
}

pub(super) fn resolve_recipient<T: TransportPort + 'static>(
    recipient: &BACnetRecipient,
    bindings: &DeviceBindingTable,
    transport: &T,
) -> Option<Arc<ConfirmedRecipientRoute>> {
    let route = match recipient {
        BACnetRecipient::Device(device) => {
            let route = RecipientRoute::from_device_resolution(bindings.resolve_at(
                device,
                Instant::now(),
                |mac| transport.is_broadcast_mac(mac),
            ))
            .into_confirmed()?;
            if route.freshness != Some(BindingFreshness::Configured) {
                return None;
            }
            route
        }
        BACnetRecipient::Address(address) => {
            if !transport.is_bip_ipv4()
                || !valid_bip_audit_address(address)
                || transport.is_broadcast_mac(&address.mac_address)
            {
                return None;
            }
            RecipientRoute::LocalUnicast(address.mac_address.clone()).into_confirmed()?
        }
    };
    Some(Arc::new(route))
}

/// Validate the supported direct unicast IPv4 Audit address shape. Link-kind
/// and interface-specific broadcast checks remain the runtime owner's duty.
#[doc(hidden)]
pub fn valid_bip_audit_address(address: &BACnetAddress) -> bool {
    if address.network_number != 0 || address.mac_address.len() != 6 {
        return false;
    }
    let mac = address.mac_address.as_slice();
    let ip = std::net::Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]);
    !ip.is_unspecified()
        && !ip.is_multicast()
        && !ip.is_broadcast()
        && mac[0] != 0
        && mac[0] < 240
        && (mac[4] != 0 || mac[5] != 0)
}

fn recipient(db: &ObjectDatabase, device: ObjectIdentifier) -> Option<BACnetRecipient> {
    let PropertyValue::ApplicationData(value) = db
        .get(&device)?
        .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
        .ok()?
    else {
        return None;
    };
    bacnet_encoding::constructed::decode_recipient(&value, 0)
        .ok()
        .map(|(value, _)| value)
}

pub(super) struct WriteAudit<'a, T: TransportPort> {
    config: &'a ServerConfig,
    network: &'a Arc<NetworkLayer<T>>,
    transactions: &'a Arc<NotificationTransactions>,
    comm_state: &'a Arc<AtomicU8>,
    bindings: DeviceBindingTable,
    source: BACnetRecipient,
    invoke_id: u8,
    pending: Option<PendingWrite>,
}

struct PendingWrite {
    route: Option<Arc<ConfirmedRecipientRoute>>,
    failure: Option<AuditFailureTicket<Arc<ConfirmedRecipientRoute>>>,
    completion: bacnet_objects::audit::AuditDeliveryToken,
    status: Arc<AuditReporterStatus>,
    confirmed: bool,
    notification: BACnetAuditNotification,
}

impl<'a, T: TransportPort + 'static> WriteAudit<'a, T> {
    pub(super) fn write_source(&self) -> bacnet_objects::device::AuditWriteSource {
        bacnet_objects::device::AuditWriteSource {
            device: self.source.clone(),
            invoke_id: self.invoke_id,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn new(
        config: &'a ServerConfig,
        network: &'a Arc<NetworkLayer<T>>,
        transactions: &'a Arc<NotificationTransactions>,
        bindings: &Arc<RwLock<DeviceBindingTable>>,
        comm_state: &'a Arc<AtomicU8>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
    ) -> Self {
        let (snapshot, known_source) = if config.audit_reporter.is_some() {
            let bindings = bindings.read().await;
            (
                bindings.clone(),
                bindings.source_device(source_mac, source_network, |mac| {
                    network.transport().is_broadcast_mac(mac)
                }),
            )
        } else {
            (DeviceBindingTable::new(), None)
        };
        let source = known_source
            .map(BACnetRecipient::Device)
            .unwrap_or_else(|| {
                BACnetRecipient::Address(BACnetAddress {
                    network_number: source_network.map_or(0, |source| source.network),
                    mac_address: source_network.map_or_else(
                        || MacAddr::from_slice(source_mac),
                        |source| source.mac_address.clone(),
                    ),
                })
            });
        Self {
            config,
            network,
            transactions,
            comm_state,
            bindings: snapshot,
            source,
            invoke_id,
            pending: None,
        }
    }
}

impl<T: TransportPort + 'static> WriteCommitObserver for WriteAudit<'_, T> {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>) {
        self.pending = None;
        let Some(profile) = &self.config.audit_reporter else {
            return;
        };
        let Some(reporter) = db
            .get(&profile.reporter)
            .and_then(|object| object.audit_reporter_internal())
        else {
            return;
        };
        let device = local_device(db);
        let route = device
            .and_then(|device| recipient(db, device))
            .and_then(|value| resolve_recipient(&value, &self.bindings, self.network.transport()));
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && route.is_some());
        let Some(device) = device else { return };
        let Some(object) = db.get(&write.oid) else {
            return;
        };
        // Standard commandable properties use Present_Value + Priority_Array.
        // A supplied priority on Description (etc.) must never filter the write.
        let command_priority = (write.property == PropertyIdentifier::PRESENT_VALUE
            && object
                .property_list()
                .contains(&PropertyIdentifier::PRIORITY_ARRAY))
        .then_some(write.priority.unwrap_or(16));
        if !reporter.monitors_object_internal(write.oid)
            || !reporter.reports_write_internal(
                write.property,
                command_priority,
                write.oid.object_type() == ObjectType::AUDIT_REPORTER,
            )
        {
            return;
        }
        let current_value = object
            .read_property(write.property, write.array_index)
            .ok()
            .and_then(|value| small_value(&value));
        self.pending = Some(PendingWrite {
            failure: self.failure_ticket(
                &status,
                reporter.confirmed_internal(),
                device,
                route.clone(),
            ),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed_internal(),
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: Some(self.invoke_id),
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(device),
                target_object: Some(write.oid),
                target_property: Some(AuditPropertyReference {
                    property_identifier: write.property,
                    property_array_index: write.array_index.map(u64::from),
                }),
                target_priority: command_priority,
                target_value: (write.value.len() <= 32).then(|| write.value.to_vec()),
                current_value,
                result: None,
            },
        });
    }

    fn committed(&mut self, db: &mut ObjectDatabase) {
        self.complete(db, None);
    }

    fn failed(&mut self, db: &mut ObjectDatabase, error: &Error) {
        // An unknown transaction outcome is not an execution failure. In
        // particular, never manufacture an Error for a timeout/Reject/Abort.
        if matches!(
            error,
            Error::Timeout(_) | Error::Reject { .. } | Error::Abort { .. }
        ) {
            self.pending = None;
            return;
        }
        self.complete(
            db,
            Some(super::requests::confirmed_response::error_fields(error)),
        );
    }
}

impl<T: TransportPort + 'static> WriteAudit<'_, T> {
    /// The file handler calls once after execution, while still holding the DB
    /// guard, and never for decoder rejections or configured budget overload.
    pub(super) fn file_completed(
        &mut self,
        db: &mut ObjectDatabase,
        target: ObjectIdentifier,
        result: &Result<(), Error>,
    ) {
        self.pending = None;
        let Some(profile) = &self.config.audit_reporter else {
            return;
        };
        let Some(reporter) = db
            .get(&profile.reporter)
            .and_then(|object| object.audit_reporter_internal())
        else {
            return;
        };
        let device = local_device(db);
        let route = device
            .and_then(|device| recipient(db, device))
            .and_then(|value| resolve_recipient(&value, &self.bindings, self.network.transport()));
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && route.is_some());
        let Some(device) = device else { return };
        // Read actual Reporter configuration, not a synthetic property-write
        // target. File writes are locally designated configuration operations.
        let enabled = matches!(
            reporter.read_property(PropertyIdentifier::AUDIT_LEVEL, None),
            Ok(PropertyValue::Enumerated(level)) if level != AuditLevel::NONE.to_raw()
        );
        let write_enabled =
            match reporter.read_property(PropertyIdentifier::AUDITABLE_OPERATIONS, None) {
                Ok(PropertyValue::BitString { unused_bits, data }) => {
                    AuditOperationFlags::from_bacnet(unused_bits, &data)
                        .is_ok_and(|operations| operations.contains(AuditOperation::WRITE))
                }
                _ => false,
            };
        if !enabled
            || !reporter.monitors_object_internal(target)
            || (!write_enabled && target.object_type() != ObjectType::AUDIT_REPORTER)
        {
            return;
        }
        self.pending = Some(PendingWrite {
            failure: self.failure_ticket(
                &status,
                reporter.confirmed_internal(),
                device,
                route.clone(),
            ),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed_internal(),
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: Some(self.invoke_id),
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(device),
                target_object: Some(target),
                target_property: None,
                target_priority: None,
                target_value: None,
                current_value: None,
                result: None,
            },
        });
        match result {
            Ok(()) => self.committed(db),
            Err(error) => self.failed(db, error),
        }
    }

    /// List services have no priority. The handler supplies its existing pre-image
    /// only after all element/framed decoding; an absent object is still a known
    /// target for a decoded execution failure.
    pub(super) fn before_list(
        &mut self,
        db: &ObjectDatabase,
        request: &bacnet_services::list_manipulation::ListElementRequest,
        current: Option<&PropertyValue>,
    ) {
        self.pending = None;
        let Some(profile) = &self.config.audit_reporter else {
            return;
        };
        let Some(reporter) = db
            .get(&profile.reporter)
            .and_then(|object| object.audit_reporter_internal())
        else {
            return;
        };
        let device = local_device(db);
        let route = device
            .and_then(|device| recipient(db, device))
            .and_then(|value| resolve_recipient(&value, &self.bindings, self.network.transport()));
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && route.is_some());
        let Some(device) = device else { return };
        if !reporter.monitors_object_internal(request.object_identifier)
            || !reporter.reports_write_internal(
                request.property_identifier,
                None,
                request.object_identifier.object_type() == ObjectType::AUDIT_REPORTER,
            )
        {
            return;
        }
        self.pending = Some(PendingWrite {
            failure: self.failure_ticket(
                &status,
                reporter.confirmed_internal(),
                device,
                route.clone(),
            ),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed_internal(),
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: Some(self.invoke_id),
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(device),
                target_object: Some(request.object_identifier),
                target_property: Some(AuditPropertyReference {
                    property_identifier: request.property_identifier,
                    property_array_index: request.property_array_index.map(u64::from),
                }),
                target_priority: None,
                target_value: (!request.list_of_elements.is_empty()
                    && request.list_of_elements.len() <= 32
                    && bacnet_encoding::constructed::validate_tlv_sequence(
                        &request.list_of_elements,
                        "list delta",
                    )
                    .is_ok())
                .then(|| request.list_of_elements.clone()),
                current_value: current.and_then(small_value).filter(|bytes| {
                    bacnet_encoding::constructed::validate_tlv_sequence(bytes, "list pre-image")
                        .is_ok()
                }),
                result: None,
            },
        });
    }

    /// Called under the execution database guard: after CREATE's final/candidate
    /// identity is known, or before DELETE removes the selected Reporter itself.
    pub(super) fn before_lifecycle(
        &mut self,
        db: &ObjectDatabase,
        operation: AuditOperation,
        target: Option<ObjectIdentifier>,
        kind: ObjectType,
    ) {
        self.pending = None;
        let Some(profile) = &self.config.audit_reporter else {
            return;
        };
        let Some(reporter) = db
            .get(&profile.reporter)
            .and_then(|object| object.audit_reporter_internal())
        else {
            return;
        };
        let device = local_device(db);
        let route = device
            .and_then(|device| recipient(db, device))
            .and_then(|value| resolve_recipient(&value, &self.bindings, self.network.transport()));
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && route.is_some());
        let Some(device) = device else { return };
        let selected = target.map_or_else(
            || reporter.monitors_unassigned_create_internal(kind),
            |oid| reporter.monitors_object_internal(oid),
        );
        if !selected || !reporter.reports_lifecycle_internal(operation) {
            return;
        }
        self.pending = Some(PendingWrite {
            failure: self.failure_ticket(
                &status,
                reporter.confirmed_internal(),
                device,
                route.clone(),
            ),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed_internal(),
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation,
                source_comment: None,
                target_comment: None,
                invoke_id: Some(self.invoke_id),
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(device),
                target_object: target,
                target_property: None,
                target_priority: None,
                target_value: None,
                current_value: None,
                result: None,
            },
        });
    }

    pub(super) fn lifecycle_completed(
        &mut self,
        db: &mut ObjectDatabase,
        result: &Result<(), Error>,
    ) {
        match result {
            Ok(()) => self.committed(db),
            Err(error) => self.failed(db, error),
        }
    }

    fn complete(&mut self, db: &mut ObjectDatabase, result: Option<(ErrorClass, ErrorCode)>) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        if pending.route.is_none() {
            return;
        }
        // No await separates execution completion from notification admission.
        // A later response-send timeout cannot change the recorded outcome.
        pending.notification.result = result;
        pending.notification.target_timestamp =
            Some(super::event_timestamp::sample_event_timestamp(db).timestamp);
        self.admit(pending);
    }

    fn admit(&self, pending: PendingWrite) {
        let Some(route) = pending.route.clone() else {
            return;
        };
        let completion = DeliveryCompletion {
            status: Arc::clone(&pending.status),
            epoch: pending.completion,
            finished: false,
        };
        if self.comm_state.load(Ordering::Acquire) != 0 {
            return;
        }
        // Validate encoding and APDU fit before resource admission: those
        // failures must not become resource-drop counts even under overload.
        let Some(mut bytes) = encode_notification(
            &pending.notification,
            pending.confirmed,
            self.config.max_apdu_length,
            0,
        ) else {
            return;
        };
        let permit = match self.transactions.try_admit_audit() {
            Ok(permit) => permit,
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                self.resource_drop(&pending);
                return;
            }
            Err(tokio::sync::TryAcquireError::Closed) => return,
        };
        let confirmed = pending.confirmed;
        let reserved = if confirmed {
            match self.transactions.reserve(
                route.canonical_peer.clone(),
                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            ) {
                Ok(reserved) => Some(reserved),
                Err(NotificationReserveError::Coordinator(
                    bacnet_endpoint_core::coordinator::ReserveError::Exhausted,
                )) => {
                    self.resource_drop(&pending);
                    return;
                }
                Err(_) => return,
            }
        } else {
            None
        };
        if let Some((operation, _)) = &reserved {
            let Some(encoded) = encode_notification(
                &pending.notification,
                confirmed,
                self.config.max_apdu_length,
                operation.invoke_id(),
            ) else {
                return;
            };
            bytes = encoded;
        }
        let network = Arc::clone(self.network);
        let comm_state = Arc::clone(self.comm_state);
        // The absolute deadline includes scheduling and transport send, not only ACK wait.
        let deadline = tokio::time::Instant::now() + DELIVERY_TIMEOUT;
        self.transactions.spawn(async move {
            let _permit = permit;
            let delivered =
                deliver(&network, &comm_state, &route, &bytes, reserved, deadline).await;
            completion.finish(delivered);
        });
    }
}

pub(super) fn encode_notification(
    notification: &BACnetAuditNotification,
    confirmed: bool,
    max_apdu: u32,
    invoke_id: u8,
) -> Option<BytesMut> {
    let mut service = BytesMut::new();
    AuditNotificationRequest {
        notifications: vec![notification.clone()],
    }
    .try_encode(&mut service)
    .ok()?;
    let pdu = if confirmed {
        Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: max_apdu as u16,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            service_request: service.freeze(),
        })
    } else {
        Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
            service_request: service.freeze(),
        })
    };
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, &pdu).ok()?;
    (bytes.len() <= max_apdu as usize).then_some(bytes)
}

pub(super) async fn deliver<T: TransportPort + 'static>(
    network: &NetworkLayer<T>,
    comm_state: &AtomicU8,
    route: &ConfirmedRecipientRoute,
    bytes: &[u8],
    reserved: Option<NotificationReservation>,
    deadline: tokio::time::Instant,
) -> bool {
    let confirmed = reserved.is_some();
    let send = || async {
        if comm_state.load(Ordering::Acquire) != 0 {
            return Err(Error::Encoding("audit initiation disabled".into()));
        }
        match (&route.local_target, &route.remote) {
            (Some(mac), None) => {
                network
                    .send_apdu(bytes, mac, confirmed, NetworkPriority::NORMAL)
                    .await
            }
            (None, Some((net, mac, Some(router)))) => {
                network
                    .send_apdu_routed(bytes, *net, mac, router, confirmed, NetworkPriority::NORMAL)
                    .await
            }
            _ => Err(Error::Encoding("audit destination is unavailable".into())),
        }
    };
    tokio::time::timeout_at(deadline, async {
        if let Some((operation, receiver)) = reserved {
            run_notification_worker(operation, receiver, DELIVERY_TIMEOUT, 0, |_| send()).await
                == NotificationWorkerResult::Ack
        } else {
            send().await.is_ok()
        }
    })
    .await
    .unwrap_or(false)
}

/// Cancellation, rejected worker admission and panic also leave visible failure.
pub(super) struct DeliveryCompletion {
    pub(super) status: Arc<AuditReporterStatus>,
    pub(super) epoch: bacnet_objects::audit::AuditDeliveryToken,
    pub(super) finished: bool,
}

impl DeliveryCompletion {
    fn auditing_failure(status: Arc<AuditReporterStatus>, expected: u64) -> Option<Self> {
        let epoch = status.begin_auditing_failure_delivery(expected)?;
        Some(Self {
            status,
            epoch,
            finished: false,
        })
    }
    pub(super) fn finish(mut self, delivered: bool) {
        self.status.complete_delivery(self.epoch, delivered);
        self.finished = true;
    }
}

impl Drop for DeliveryCompletion {
    fn drop(&mut self) {
        if !self.finished {
            self.status.complete_delivery(self.epoch, false);
        }
    }
}

/// Table 19-4 permits omission above 32 encoded octets. Bound encoding work,
/// including nested lists, before handing a value to the ordinary encoder.
fn small_value(value: &PropertyValue) -> Option<Vec<u8>> {
    fn append(value: &PropertyValue, out: &mut BytesMut, remaining: &mut usize) -> Option<()> {
        if *remaining == 0 || out.len() > 32 {
            return None;
        }
        *remaining -= 1;
        match value {
            PropertyValue::List(values) => {
                if values.len() > 32 {
                    return None;
                }
                for value in values {
                    append(value, out, remaining)?;
                }
            }
            PropertyValue::CharacterString(value) if value.len() > 32 => return None,
            PropertyValue::OctetString(value) | PropertyValue::ApplicationData(value)
                if value.len() > 32 =>
            {
                return None
            }
            PropertyValue::BitString { data, .. } if data.len() > 32 => return None,
            _ => encode_property_value(out, value).ok()?,
        }
        (out.len() <= 32).then_some(())
    }
    let mut bytes = BytesMut::new();
    append(value, &mut bytes, &mut 64)?;
    (!bytes.is_empty()).then(|| bytes.to_vec())
}

#[path = "audit_reporter_failure.rs"]
mod failure;
