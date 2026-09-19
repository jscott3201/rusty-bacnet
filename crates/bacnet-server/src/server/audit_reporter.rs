use super::device_bindings::BindingFreshness;
use super::event_recipient_route::{ConfirmedRecipientRoute, RecipientRoute};
use super::*;
use crate::handlers::{WriteCommitObserver, WriteTarget};
use bacnet_objects::audit::AuditReporterStatus;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAddress, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::AuditOperation;

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(3);

/// One locally configured target WRITE/CREATE/DELETE Reporter and unicast recipient.
///
/// Reports successful inbound WP/WPM elements, AddListElement/RemoveListElement,
/// CreateObject/DeleteObject operations, and authorized execution errors.
/// Policy denials and undecoded/unattempted elements remain silent. Successes
/// omit Result; execution failures include the mapped BACnet Error. There is no
/// source-side reporting, per-object override, batching, forwarding, or durable outbox.
/// Configure the recipient with the builder's `device_binding` method. Server
/// startup returns [`Error::Encoding`] if the selected Reporter is absent or
/// lacks the Audit Reporter capability. A missing or unresolvable recipient
/// still permits startup and exposes CONFIGURATION_ERROR through the existing
/// enabled Reporter's Reliability.
/// At most 64 deliveries are active per server, with no waiting queue. Each
/// send/ACK has one total three-second deadline and no retries. Overflow or
/// delivery failure sets COMMUNICATION_FAILURE, never changes the write result,
/// and retains no record. Values larger than 32 encoded octets are omitted.
/// The complete APDU must fit the server's limit; outbound segmentation is not
/// implemented by this profile. Encoding/size failures are delivery failures.
/// A subsequent successful delivery clears communication failure unless a newer
/// failure occurred after that delivery began. Unconfirmed success proves only
/// transport acceptance, not storage by the recipient.
///
/// Audit_Source_Reporter remains false. No Device.Audit_Notification_Recipient,
/// per-object overrides, source reporting, direct local-write
/// reporting, batching, AUDITING_FAILURE records, or Python parity is claimed.
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
/// Deleting the selected Reporter is allowed: its last record owns the removed
/// instance's delivery health, then the unavailable profile remains silent.
///
/// List edits use WRITE, object/property/requested-index identity, no priority,
/// the requested delta as raw Target_Value, and the known pre-image as Current_Value.
/// Empty, structurally invalid, or over-32-octet values are omitted, never wrapped
/// or truncated. Element and framed-list decoding precedes observation; valid
/// execution failures retain the response-mapped Result. Successful no-op removals
/// still report once. AUDIT_CONFIG admits implemented non-Present_Value lists;
/// list services ignore priority filtering and reuse ordinary target selection.
/// This is not complete WRITE coverage: AtomicWriteFile is not reported and
/// inbound WriteGroup remains unsupported by design.
///
/// ```no_run
/// use bacnet_objects::{audit::AuditReporterObject, database::ObjectDatabase,
///     device::{DeviceConfig, DeviceObject}};
/// use bacnet_server::server::{AuditReporterConfig, BACnetServer, DeviceBinding};
/// use bacnet_types::{bitstring::AuditOperationFlags, enums::{AuditLevel,
///     AuditOperation, ObjectType}, primitives::ObjectIdentifier};
/// # async fn example() -> Result<(), bacnet_types::error::Error> {
/// let mut db = ObjectDatabase::new();
/// db.add(Box::new(DeviceObject::new(DeviceConfig::default())?))?;
/// let mut reporter = AuditReporterObject::new(1, "Target writes")?;
/// reporter.set_audit_level(AuditLevel::AUDIT_ALL)?;
/// let mut operations = AuditOperationFlags::empty();
/// operations.insert(AuditOperation::WRITE);
/// reporter.set_auditable_operations(operations);
/// reporter.set_issue_confirmed_notifications(true);
/// db.add(Box::new(reporter))?;
/// let recipient = ObjectIdentifier::new(ObjectType::DEVICE, 200)?;
/// let mut server = BACnetServer::builder().database(db)
///     .audit_reporter(AuditReporterConfig {
///         reporter: ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1)?,
///         recipient: Some(recipient),
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
    /// Destination Device, resolved through explicitly configured Device bindings.
    pub recipient: Option<ObjectIdentifier>,
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Enable the narrow target WRITE/CREATE/DELETE profile; see [`AuditReporterConfig`].
    pub fn audit_reporter(mut self, profile: AuditReporterConfig) -> Self {
        self.config.audit_reporter = Some(profile);
        self
    }
}

impl BipServerBuilder {
    /// Enable the narrow target WRITE/CREATE/DELETE profile; see [`AuditReporterConfig`].
    pub fn audit_reporter(mut self, profile: AuditReporterConfig) -> Self {
        self.config.audit_reporter = Some(profile);
        self
    }
}

fn local_device(db: &ObjectDatabase) -> Option<ObjectIdentifier> {
    db.list_objects()
        .into_iter()
        .find(|oid| oid.object_type() == ObjectType::DEVICE)
}

fn resolve(
    profile: &AuditReporterConfig,
    bindings: &DeviceBindingTable,
    is_broadcast: impl Fn(&[u8]) -> bool,
) -> Option<Arc<ConfirmedRecipientRoute>> {
    let recipient = profile.recipient?;
    let route = RecipientRoute::from_device_resolution(bindings.resolve_at(
        &recipient,
        Instant::now(),
        is_broadcast,
    ))
    .into_confirmed()?;
    // This slice owns a fixed configured destination, not discovery/refresh.
    if route.freshness != Some(BindingFreshness::Configured) {
        return None;
    }
    Some(Arc::new(route))
}

pub(super) fn initialize(
    db: &ObjectDatabase,
    config: &ServerConfig,
    bindings: &DeviceBindingTable,
    is_broadcast: impl Fn(&[u8]) -> bool,
) -> Result<(), Error> {
    if let Some(profile) = &config.audit_reporter {
        let reporter = db
            .get(&profile.reporter)
            .and_then(|object| object.audit_reporter_internal())
            .ok_or_else(|| Error::Encoding(format!(
                "invalid audit reporter: selected object {:?} is absent or lacks the Audit Reporter capability",
                profile.reporter,
            )))?;
        reporter.status_internal().set_configured(
            local_device(db).is_some() && resolve(profile, bindings, is_broadcast).is_some(),
        );
    }
    Ok(())
}

pub(super) struct WriteAudit<'a, T: TransportPort> {
    config: &'a ServerConfig,
    network: &'a Arc<NetworkLayer<T>>,
    transactions: &'a Arc<NotificationTransactions>,
    comm_state: &'a Arc<AtomicU8>,
    route: Option<Arc<ConfirmedRecipientRoute>>,
    source: BACnetRecipient,
    invoke_id: u8,
    pending: Option<PendingWrite>,
}

struct PendingWrite {
    status: Arc<AuditReporterStatus>,
    confirmed: bool,
    notification: BACnetAuditNotification,
}

impl<'a, T: TransportPort + 'static> WriteAudit<'a, T> {
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
        let (route, known_source) = if let Some(profile) = &config.audit_reporter {
            let bindings = bindings.read().await;
            (
                resolve(profile, &bindings, |mac| {
                    network.transport().is_broadcast_mac(mac)
                }),
                bindings.source_device(source_mac, source_network, |mac| {
                    network.transport().is_broadcast_mac(mac)
                }),
            )
        } else {
            (None, None)
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
            route,
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
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && self.route.is_some());
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
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && self.route.is_some());
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
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && self.route.is_some());
        let Some(device) = device else { return };
        let selected = target.map_or_else(
            || reporter.monitors_unassigned_create_internal(kind),
            |oid| reporter.monitors_object_internal(oid),
        );
        if !selected || !reporter.reports_lifecycle_internal(operation) {
            return;
        }
        self.pending = Some(PendingWrite {
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
        let Some(route) = self.route.clone() else {
            return;
        };
        // No await separates execution completion from notification admission.
        // A later response-send timeout cannot change the recorded outcome.
        pending.notification.result = result;
        pending.notification.target_timestamp =
            Some(super::event_timestamp::sample_event_timestamp(db).timestamp);
        let completion = DeliveryCompletion::new(pending.status);
        if self.comm_state.load(Ordering::Acquire) != 0 {
            return;
        }
        let Some(permit) = self.transactions.try_admit_audit() else {
            return;
        };
        let mut service = BytesMut::new();
        if (AuditNotificationRequest {
            notifications: vec![pending.notification],
        })
        .try_encode(&mut service)
        .is_err()
        {
            return;
        }
        let confirmed = pending.confirmed;
        let reserved = if confirmed {
            match self.transactions.reserve(
                route.canonical_peer.clone(),
                ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
            ) {
                Ok(reserved) => Some(reserved),
                Err(_) => return,
            }
        } else {
            None
        };
        let pdu = if let Some((operation, _)) = &reserved {
            Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: self.config.max_apdu_length as u16,
                invoke_id: operation.invoke_id(),
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
        if encode_apdu(&mut bytes, &pdu).is_err()
            || bytes.len() > self.config.max_apdu_length as usize
        {
            return;
        }
        let network = Arc::clone(self.network);
        let comm_state = Arc::clone(self.comm_state);
        // The absolute deadline includes scheduling and transport send, not only ACK wait.
        let deadline = tokio::time::Instant::now() + DELIVERY_TIMEOUT;
        self.transactions.spawn(async move {
            let _permit = permit;
            let send = || async {
                if comm_state.load(Ordering::Acquire) != 0 {
                    return Err(Error::Encoding("audit initiation disabled".into()));
                }
                match (&route.local_target, &route.remote) {
                    (Some(mac), None) => {
                        network
                            .send_apdu(&bytes, mac, confirmed, NetworkPriority::NORMAL)
                            .await
                    }
                    (None, Some((net, mac, Some(router)))) => {
                        network
                            .send_apdu_routed(
                                &bytes,
                                *net,
                                mac,
                                router,
                                confirmed,
                                NetworkPriority::NORMAL,
                            )
                            .await
                    }
                    _ => Err(Error::Encoding("audit destination is unavailable".into())),
                }
            };
            let delivered = tokio::time::timeout_at(deadline, async {
                if let Some((operation, receiver)) = reserved {
                    run_notification_worker(operation, receiver, DELIVERY_TIMEOUT, 0, |_| send())
                        .await
                        == NotificationWorkerResult::Ack
                } else {
                    send().await.is_ok()
                }
            })
            .await
            .unwrap_or(false);
            completion.finish(delivered);
        });
    }
}

/// Cancellation, rejected worker admission and panic also leave visible failure.
struct DeliveryCompletion {
    status: Arc<AuditReporterStatus>,
    epoch: u64,
    finished: bool,
}

impl DeliveryCompletion {
    fn new(status: Arc<AuditReporterStatus>) -> Self {
        let epoch = status.begin_delivery();
        Self {
            status,
            epoch,
            finished: false,
        }
    }
    fn finish(mut self, delivered: bool) {
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
