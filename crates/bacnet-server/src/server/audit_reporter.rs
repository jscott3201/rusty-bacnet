use super::event_recipient_route::ConfirmedRecipientRoute;
use super::notification_transactions::{
    AuditFailureContext, AuditFailureTicket, NotificationReservation, NotificationReserveError,
};
use super::*;
use crate::handlers::{WriteCommitObserver, WriteTarget};
use bacnet_objects::audit::AuditReporterStatus;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAddress, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditLevel, AuditOperation};

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[path = "audit_reporter_read.rs"]
mod read;
pub(super) use read::ReadAuditIntent;

#[path = "audit_reporter_device.rs"]
mod device;

#[path = "audit_reporter_write.rs"]
mod write;
use write::WriteSelection;

#[path = "audit_policy_precommit.rs"]
mod policy_precommit;

/// A bounded set of target READ/WRITE/CREATE/DELETE Reporters sharing one Device recipient.
///
/// Reports successful inbound WP/WPM elements, AddListElement/RemoveListElement,
/// AtomicWriteFile, CreateObject/DeleteObject operations, and authorized execution errors.
/// Policy denials and undecoded/unattempted elements remain silent. Successes
/// omit Result; execution failures include the mapped BACnet Error. There is no
/// source-side reporting, forwarding, or durable outbox.
/// Provision the typed recipient on the built-in Device before startup; configure
/// Device routes with `device_binding`. Startup requires a concrete built-in
/// local Device ([`ObjectDatabase::local_device`]: the lowest instance when the
/// database holds several), a provision, and the selected Reporter capability.
/// That Device names this device in every record and owns the recipient; any
/// other Device in the database takes no part. Unresolved
/// configured Device routes permit startup with CONFIGURATION_ERROR. Address
/// choices require direct unicast IPv4 B/IP, on network zero or on this
/// network's number once it is known (#1460); one naming a network the number
/// in force does not name starts unresolved the same way. The active Device recipient is
/// required/writable; actual local or authorized WP/WPM changes atomically reserve
/// two attempts before commit, independently of ordinary reporting filters: to
/// the old and the new recipient. The new route must be usable. An unavailable
/// old route requires reconfiguration and restart, except an old Address the
/// number does not name: an unconfirmed global broadcast takes its attempt
/// (Clause 12.11.66). Active Device/Reporter membership is protected until quiescence.
/// At most 64 deliveries are active per server. Optional object-owned
/// Maximum_Send_Delay/Send_Now retains ordinary records in a bounded target queue:
/// 256 records/256 KiB globally and 64 records/64 KiB per Reporter. Mandatory
/// records remain immediate. Each send/ACK has one total three-second deadline
/// and no retry. Known local losses use captured, bounded historical contexts;
/// summary filtering remains the documented partial-profile policy. Unconfirmed
/// success proves only transport acceptance. See docs/delayed-target-audit.md.
///
/// Audit_Source_Reporter remains false. AV/BV instance-owned overrides apply to
/// target observations. Supported `write_local` operations share this observer;
/// raw database authoring remains a bypass. Source reporting is a separate profile.
/// Ordinary sensor samples and internal reliability updates never enter this
/// producer. An enabled external write to a Reporter produces one record.
/// Locally configured Monitored_Objects selects ordinary targets by exact object
/// or object type. Omitted selection preserves catch-all behavior; an empty or
/// all-NULL selection reports no ordinary targets. Reporter writes bypass it.
/// Enabled nominal overlaps fault all affected Reporters; only the lowest instance
/// emits, before ordinary filters. Live local setters and aggregate changes use
/// atomic admission; equal configuration is silent. Network selection writes are
/// not supported. See docs/target-audit-reporters.md for selected fallback policies.
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
/// priority filtering is irrelevant. Admission follows the service decoder's
/// acceptance boundary, which refuses octets after the request's last member.
/// Decoder rejections and configured payload/count budget Aborts remain silent.
/// An inbound WriteGroup reports WRITE (Table 19-5) once per Channel it writes,
/// as a WriteProperty of that Channel's Present_Value at the priority used, with
/// the requester as source and no invoke ID; a Channel write the mutation
/// authorizer denied makes none (#1318). A Channel's Present_Value is
/// commandable (Clause 12.53.5), so its records, a WriteProperty's included,
/// carry the priority and Audit_Priority_Filter applies to them (Clause
/// 19.6.3): one at a disabled priority is dropped.
///
/// READ covers completed, unsegmented RP/RPM responses, one record per
/// returned property outcome in result order, including inline RPM errors.
/// Read values, comments, and priorities are always omitted. READ requires its
/// operation bit; AUDIT_CONFIG excludes Present_Value. RPM's existing result
/// budget bounds provisional intents, which are discarded on whole-request
/// failure. Admission follows release of the read guard and uses the same
/// captured delivery, optional delay and bounded resource-loss policy.
///
/// ```no_run
/// use bacnet_objects::{audit::AuditReporterObject, database::ObjectDatabase,
///     device::{DeviceConfig, DeviceObject}};
/// use bacnet_server::server::{AuditReportersConfig, BACnetServer, DeviceBinding};
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
/// reporter.set_auditable_operations(operations)?;
/// reporter.set_issue_confirmed_notifications(true)?;
/// db.add(Box::new(reporter))?;
/// let mut server = BACnetServer::bip_builder().database(db)
///     .audit_reporters(AuditReportersConfig {
///         reporters: vec![ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1)?],
///     })
///     .device_binding(DeviceBinding::local(recipient, [127, 0, 0, 1, 0xBA, 0xC1])?)?
///     .build().await?;
/// server.stop().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct AuditReportersConfig {
    /// Between one and 64 distinct concrete target Reporter identifiers.
    /// Startup validates and canonicalizes them by instance; this is a local bound.
    pub reporters: Vec<ObjectIdentifier>,
}

impl AuditReportersConfig {
    pub(super) fn canonicalize(&mut self) -> Result<(), Error> {
        if self.reporters.is_empty()
            || self.reporters.len() > 64
            || self.reporters.iter().any(|oid| {
                oid.object_type() != ObjectType::AUDIT_REPORTER
                    || oid.instance_number() == ObjectIdentifier::MAX_INSTANCE
            })
        {
            return Err(Error::Encoding(
                "target Audit requires 1..=64 concrete Reporter identifiers".into(),
            ));
        }
        self.reporters.sort_by_key(|oid| oid.instance_number());
        if self.reporters.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::Encoding(
                "duplicate target Audit Reporter identifier".into(),
            ));
        }
        Ok(())
    }
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Enable the narrow target audit profile; see [`AuditReportersConfig`].
    pub fn audit_reporters(mut self, profile: AuditReportersConfig) -> Self {
        self.config.audit_reporters = Some(profile);
        self
    }
}

impl BipServerBuilder {
    /// Enable the narrow target audit profile; see [`AuditReportersConfig`].
    pub fn audit_reporters(mut self, profile: AuditReportersConfig) -> Self {
        self.config.audit_reporters = Some(profile);
        self
    }
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

/// Where a request came from: the peer's link address, its network address
/// when routed, and a confirmed request's invoke ID.
#[derive(Clone, Copy)]
pub(super) struct RequestSource<'s> {
    pub(super) mac: &'s [u8],
    pub(super) network: Option<&'s NpduAddress>,
    pub(super) invoke_id: u8,
    /// This network's own number as read for the request, the one its
    /// command origin was matched against too, so a binding routed through
    /// it names a direct request from its final MAC (#1404).
    pub(super) local_network: Option<u16>,
}

pub(super) struct WriteAudit<'a, T: TransportPort> {
    config: &'a ServerConfig,
    network: &'a Arc<NetworkLayer<T>>,
    transactions: &'a Arc<NotificationTransactions>,
    source: BACnetRecipient,
    invoke_id: Option<u8>,
    /// Boxed: a write's audit record is large, and a read, which never
    /// has one, would otherwise carry its space in its handler (#1713).
    pending: Option<Box<PendingWrite>>,
}

struct PendingWrite {
    delay: Option<bacnet_objects::audit::AuditSendDelay>,
    selection: Option<WriteSelection>,
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
            invoke_id: self.invoke_id.expect("network write source"),
        }
    }

    pub(super) fn local(
        config: &'a ServerConfig,
        network: &'a Arc<NetworkLayer<T>>,
        transactions: &'a Arc<NotificationTransactions>,
        db: &ObjectDatabase,
    ) -> Option<Self> {
        Some(Self {
            config,
            network,
            transactions,
            source: BACnetRecipient::Device(db.local_device().identifier()?),
            invoke_id: None,
            pending: None,
        })
    }

    pub(super) async fn new(
        config: &'a ServerConfig,
        network: &'a Arc<NetworkLayer<T>>,
        transactions: &'a Arc<NotificationTransactions>,
        bindings: &Arc<RwLock<DeviceBindingTable>>,
        request: RequestSource<'_>,
    ) -> Self {
        let RequestSource {
            mac,
            network: source_network,
            invoke_id,
            local_network,
        } = request;
        let source = Self::requester(
            config,
            transactions,
            bindings,
            mac,
            source_network,
            local_network,
        )
        .await;
        Self::requested_by(config, network, transactions, source, Some(invoke_id))
    }

    /// The audit of writes `source` asked for, in a request carrying
    /// `invoke_id`; `None` for an unconfirmed one (Table 19-4).
    pub(super) fn requested_by(
        config: &'a ServerConfig,
        network: &'a Arc<NetworkLayer<T>>,
        transactions: &'a Arc<NotificationTransactions>,
        source: BACnetRecipient,
        invoke_id: Option<u8>,
    ) -> Self {
        Self {
            config,
            network,
            transactions,
            source,
            invoke_id,
            pending: None,
        }
    }

    /// The device a request from `source_mac`, routed from `source_network`,
    /// came from, as an Audit record names it: the Device bound to that
    /// address, while the audit profile is on and one is, otherwise the
    /// address. `local_network` is this network's number as read for the
    /// request.
    pub(super) async fn requester(
        config: &ServerConfig,
        transactions: &Arc<NotificationTransactions>,
        bindings: &Arc<RwLock<DeviceBindingTable>>,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        local_network: Option<u16>,
    ) -> BACnetRecipient {
        // Entries were checked against the concrete link at configuration or
        // observation admission. Correlation needs no caller code under locks.
        let known_source = if config.audit_reporters.is_some() {
            bindings
                .read()
                .await
                .source_device(source_mac, source_network, local_network, |mac| {
                    transactions
                        .audit_routes
                        .get()
                        .is_some_and(|routes| routes.is_group(mac))
                })
        } else {
            None
        };
        known_source
            .map(BACnetRecipient::Device)
            .unwrap_or_else(|| {
                BACnetRecipient::Address(BACnetAddress {
                    network_number: source_network.map_or(0, |source| source.network),
                    mac_address: source_network.map_or_else(
                        || MacAddr::from_slice(source_mac),
                        |source| source.mac_address.clone(),
                    ),
                })
            })
    }
}

impl<T: TransportPort + 'static> WriteAudit<'_, T> {
    pub(super) fn select(
        &self,
        target: Option<ObjectIdentifier>,
        kind: ObjectType,
    ) -> Option<bacnet_objects::audit::SelectedAuditReporter> {
        self.transactions
            .audit_association
            .get()?
            .select(target, kind, None)
    }
    /// Local attempt-accounting policy for the supported network WRITE family.
    /// A configured enabled Reporter falls back to itself without adding nominal
    /// membership. Ordinary targets (including a disabled Reporter) use filters.
    pub(super) fn select_write(
        &self,
        db: &ObjectDatabase,
        target: ObjectIdentifier,
    ) -> Option<(bacnet_objects::audit::SelectedAuditReporter, bool)> {
        let mandatory = db
            .get(&target)
            .and_then(|object| object.audit_reporter_internal())
            .is_some_and(|reporter| reporter.configuration_internal().enabled());
        let association = self.transactions.audit_association.get()?;
        let selected = if mandatory {
            association.select_change(target, None)
        } else {
            association.select(Some(target), target.object_type(), None)
        }?;
        Some((selected, mandatory))
    }
    /// The file handler calls once after execution, while still holding the DB
    /// guard, and never for decoder rejections or configured budget overload.
    pub(super) fn file_completed(
        &mut self,
        db: &mut ObjectDatabase,
        target: ObjectIdentifier,
        result: &Result<(), Error>,
    ) {
        self.pending = None;
        let Some((selected_reporter, mandatory)) = self.select_write(db, target) else {
            return;
        };
        let reporter = &selected_reporter.configuration;
        let device = db.local_device().identifier();
        let route = device
            .and_then(|device| recipient(db, device))
            .and_then(|value| self.transactions.audit_routes.get()?.resolve(&value));
        let status = Arc::clone(&selected_reporter.status);
        status.set_configured(device.is_some() && route.is_some());
        let Some(device) = device else { return };
        let policy = db
            .get(&target)
            .map(|object| object.audit_object_policy_internal())
            .unwrap_or_default()
            .effective_internal(reporter);
        let selected = if mandatory {
            true
        } else {
            policy.reports(AuditOperation::WRITE, None, None)
        };
        if !selected {
            return;
        }
        self.pending = Some(Box::new(PendingWrite {
            delay: (!mandatory)
                .then_some(reporter.maximum_send_delay)
                .flatten(),
            selection: None,
            failure: self.failure_ticket(&status, reporter.confirmed, device, route.clone()),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed,
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: self.invoke_id,
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
        }));
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
        let Some((selected_reporter, mandatory)) = self.select_write(db, request.object_identifier)
        else {
            return;
        };
        let reporter = &selected_reporter.configuration;
        let device = db.local_device().identifier();
        let route = device
            .and_then(|device| recipient(db, device))
            .and_then(|value| self.transactions.audit_routes.get()?.resolve(&value));
        let status = Arc::clone(&selected_reporter.status);
        status.set_configured(device.is_some() && route.is_some());
        let Some(device) = device else { return };
        let policy = db
            .get(&request.object_identifier)
            .map(|object| object.audit_object_policy_internal())
            .unwrap_or_default()
            .effective_internal(reporter);
        let selected = if mandatory {
            true
        } else {
            policy.reports(
                AuditOperation::WRITE,
                Some(request.property_identifier),
                None,
            )
        };
        if !selected {
            return;
        }
        self.pending = Some(Box::new(PendingWrite {
            delay: (!mandatory)
                .then_some(reporter.maximum_send_delay)
                .flatten(),
            selection: None,
            failure: self.failure_ticket(&status, reporter.confirmed, device, route.clone()),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed,
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: self.invoke_id,
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(device),
                target_object: Some(request.object_identifier),
                target_property: Some(AuditPropertyReference {
                    property_identifier: request.property_identifier,
                    property_array_index: request.property_array_index.map(u64::from),
                }),
                target_priority: None,
                target_value: (request.list_of_elements.len() <= 32
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
        }));
    }

    /// Called under the execution database guard: after CREATE's final/candidate
    /// identity is known, or before DELETE removes the selected Reporter itself.
    pub(super) fn before_lifecycle(
        &mut self,
        db: &ObjectDatabase,
        operation: AuditOperation,
        target: Option<ObjectIdentifier>,
        kind: ObjectType,
        use_object_policy: bool,
    ) {
        self.pending = None;
        let Some(selected_reporter) = self.select(target, kind) else {
            return;
        };
        let reporter = &selected_reporter.configuration;
        let device = db.local_device().identifier();
        let route = device
            .and_then(|device| recipient(db, device))
            .and_then(|value| self.transactions.audit_routes.get()?.resolve(&value));
        let status = Arc::clone(&selected_reporter.status);
        status.set_configured(device.is_some() && route.is_some());
        let Some(device) = device else { return };
        let selected = target.map_or_else(
            || reporter.monitors_unassigned(kind),
            |oid| reporter.monitors(oid),
        );
        let policy = target
            .filter(|_| use_object_policy)
            .and_then(|oid| db.get(&oid))
            .map(|object| object.audit_object_policy_internal())
            .unwrap_or_default()
            .effective_internal(reporter);
        if !selected || !policy.reports(operation, None, None) {
            return;
        }
        self.pending = Some(Box::new(PendingWrite {
            delay: reporter.maximum_send_delay,
            selection: None,
            failure: self.failure_ticket(&status, reporter.confirmed, device, route.clone()),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed,
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation,
                source_comment: None,
                target_comment: None,
                invoke_id: self.invoke_id,
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
        }));
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
        if pending
            .selection
            .as_ref()
            .is_some_and(|selection| !selection.selected(db, result.is_none()))
        {
            return;
        }
        if pending.route.is_none() || pending.failure.is_none() {
            return;
        }
        // No await separates execution completion from notification admission.
        // A later response-send timeout cannot change the recorded outcome.
        pending.notification.result = result;
        pending.notification.target_timestamp =
            Some(super::event_timestamp::sample_event_timestamp(db).timestamp);
        self.admit(*pending);
    }

    fn admit(&self, pending: PendingWrite) {
        if let Some(delay) = pending.delay.filter(|delay| delay.seconds() != 0) {
            let completion = DeliveryCompletion {
                status: Arc::clone(&pending.status),
                epoch: pending.completion,
                finished: false,
            };
            if let Some(queue) = self
                .transactions
                .audit_batch
                .get()
                .and_then(std::sync::Weak::upgrade)
            {
                let mut record = BytesMut::new();
                if bacnet_encoding::constructed::encode_audit_notification(
                    &pending.notification,
                    &mut record,
                )
                .is_err()
                {
                    return;
                }
                let queued = super::audit_batch_queue::QueuedAudit::new(
                    completion,
                    record,
                    pending
                        .notification
                        .target_timestamp
                        .clone()
                        .expect("completed record"),
                    pending.failure.clone().expect("captured ordinary record"),
                    delay,
                );
                if queue.enqueue(queued).is_err() {
                    self.resource_drop(&pending);
                }
                return;
            }
        }
        let Some(route) = pending.route.clone() else {
            return;
        };
        let completion = DeliveryCompletion {
            status: Arc::clone(&pending.status),
            epoch: pending.completion,
            finished: false,
        };
        // Validate encoding and APDU fit before resource admission: those
        // failures must not become resource-drop counts even under overload.
        let Some(mut bytes) = encode_notification(
            &pending.notification,
            pending.confirmed,
            u32::from(self.config.device.max_apdu_length()),
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
                u32::from(self.config.device.max_apdu_length()),
                operation.invoke_id(),
            ) else {
                return;
            };
            bytes = encoded;
        }
        let network = Arc::clone(self.network);
        // The absolute deadline includes scheduling and transport send, not only ACK wait.
        let deadline = tokio::time::Instant::now() + DELIVERY_TIMEOUT;
        let context_pin = pending.failure;
        self.transactions.spawn(async move {
            let _permit = permit;
            let _context_pin = context_pin;
            let delivered = deliver(&network, &route, &bytes, reserved, deadline).await;
            completion.finish(delivered);
        });
    }
}

#[path = "audit_delivery.rs"]
mod delivery;
pub(super) use delivery::{
    DeliveryCompletion, deliver, deliver_global_broadcast, deliver_observed, encode_notification,
};

/// Table 19-4 permits omission above 32 encoded octets. Bound encoding work,
/// including nested lists, before handing a value to the ordinary encoder.
pub(super) fn small_value(value: &PropertyValue) -> Option<Vec<u8>> {
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
                return None;
            }
            PropertyValue::BitString { data, .. } if data.len() > 32 => return None,
            _ => encode_property_value(out, value).ok()?,
        }
        (out.len() <= 32).then_some(())
    }
    let mut bytes = BytesMut::new();
    append(value, &mut bytes, &mut 64)?;
    Some(bytes.to_vec())
}

#[path = "audit_reporter_failure.rs"]
mod failure;
pub(super) use failure::record_resource_drop;
