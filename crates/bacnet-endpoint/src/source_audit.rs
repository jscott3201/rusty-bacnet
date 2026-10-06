//! Session-private source Audit ownership. No discovery cache or persistent outbox.
use std::net::Ipv4Addr;
use std::sync::{Arc, Weak};

use bacnet_client::{
    EndpointOperationAck, EndpointOperationRequest, EndpointReadAck, EndpointReadRequest,
    EndpointRequester,
};
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_endpoint_core::endpoint_ingress::{EndpointApduDestination, EndpointEgress};
use bacnet_objects::audit::AuditReporterStatus;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::{
    __endpoint_AuditFailureContext as AuditFailureContext,
    __endpoint_AuditFailureQueue as AuditFailureQueue,
    __endpoint_AuditFailureTicket as AuditFailureTicket,
    __endpoint_NotificationTransactions as NotificationTransactions,
};
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_transport::port::DataAttribute;
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAddress, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditLevel, AuditOperation, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;
use tokio::sync::{oneshot, RwLock, Semaphore};

#[path = "source_recipient.rs"]
pub(crate) mod recipient;
use bacnet_objects::database::AuditOwnership;
use bacnet_objects::device::AuditRecipientChangeSink;
use recipient::{SourceRecipient, SourceRoutes};

#[path = "source_audit_delivery.rs"]
mod delivery;
#[path = "source_audit_failures.rs"]
mod failures;

pub(crate) struct SourceAudit {
    db: Arc<RwLock<ObjectDatabase>>,
    selected: ObjectIdentifier,
    device: ObjectIdentifier,
    runtime: Weak<SourceRecipient>,
    broadcast: Ipv4Addr,
    egress: EndpointEgress,
    notifications: Weak<NotificationTransactions>,
    operations: Arc<Semaphore>,
    max_apdu: u16,
    failures: Arc<AuditFailureQueue<MacAddr>>,
    #[cfg(test)]
    pub(crate) summary_queue_full: tokio::sync::Notify,
}

impl SourceAudit {
    pub(crate) fn new(
        db: Arc<RwLock<ObjectDatabase>>,
        selected: ObjectIdentifier,
        routes: SourceRoutes,
        broadcast: Ipv4Addr,
        egress: EndpointEgress,
        notifications: &Arc<NotificationTransactions>,
        max_apdu: u16,
    ) -> Result<(Arc<Self>, Arc<SourceRecipient>), Error> {
        let mut database = db.try_write().expect("unshared startup database");
        let device = database
            .local_device()
            .identifier()
            .expect("preflight local Device");
        let status = database
            .get(&selected)
            .and_then(|object| object.audit_reporter_internal())
            .expect("preflight Reporter")
            .status_internal();
        let initial = database
            .get_mut(&device)
            .and_then(|object| object.device_authority_internal())
            .and_then(|authority| authority.provisioned_audit_recipient().cloned())
            .expect("preflight recipient");
        routes.validate_initial(&initial)?;
        status.set_configured(routes.resolve(&initial).is_some());
        let failures = Arc::new(AuditFailureQueue::default());
        let runtime = Arc::new(SourceRecipient {
            device,
            status,
            routes,
            owner: AuditOwnership::for_source(device, selected),
            sequence: database.event_sequence_internal(),
            egress: egress.clone(),
            notifications: Arc::downgrade(notifications),
            max_apdu,
            failures: Arc::clone(&failures),
        });
        let source = Arc::new(Self {
            db: Arc::clone(&db),
            selected,
            device,
            runtime: Arc::downgrade(&runtime),
            broadcast,
            egress,
            notifications: Arc::downgrade(notifications),
            operations: Arc::new(Semaphore::new(64)),
            max_apdu,
            failures,
            #[cfg(test)]
            summary_queue_full: tokio::sync::Notify::new(),
        });
        database
            .with_object_adapter(&selected, |slot| {
                crate::session::source_reporter::install(slot, &runtime.owner)
            })?
            .expect("preflight Reporter")?;
        let sink: Arc<dyn AuditRecipientChangeSink> = runtime.clone();
        database
            .get_mut(&device)
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .install_audit_recipient(&sink)?;
        database.protect_audit_internal(&runtime.owner)?;
        notifications.set_audit_owner(&runtime.owner);
        Ok((source, runtime))
    }

    #[cfg(test)]
    pub(crate) fn available_operations(&self) -> usize {
        self.operations.available_permits()
    }

    pub(crate) fn close(&self) {
        if let Some(runtime) = self.runtime.upgrade() {
            runtime.seal();
        }
        self.operations.close();
    }

    pub(crate) async fn read(
        self: &Arc<Self>,
        requester: &EndpointRequester,
        destination: EndpointApduDestination,
        attributes: Vec<DataAttribute>,
        request: EndpointReadRequest,
    ) -> Result<EndpointReadAck, Error> {
        self.operate(
            requester,
            destination,
            attributes,
            EndpointOperationRequest::Read(request),
            None,
        )
        .await?
        .into_read()
    }

    pub(crate) async fn write(
        self: &Arc<Self>,
        requester: &EndpointRequester,
        destination: EndpointApduDestination,
        request: bacnet_services::write_property::WritePropertyRequest,
        commandability: crate::roles::Commandability,
    ) -> Result<(), Error> {
        let priority = match commandability {
            crate::roles::Commandability::Commandable => Some(request.priority.unwrap_or(16)),
            crate::roles::Commandability::Noncommandable => None,
        };
        self.operate(
            requester,
            destination,
            Vec::new(),
            EndpointOperationRequest::Write(request),
            priority,
        )
        .await?
        .into_write()
    }

    async fn operate(
        self: &Arc<Self>,
        requester: &EndpointRequester,
        destination: EndpointApduDestination,
        attributes: Vec<DataAttribute>,
        request: EndpointOperationRequest,
        command_priority: Option<u8>,
    ) -> Result<EndpointOperationAck, Error> {
        request.validate()?;
        let identities = request.identities();
        let audit_operation = match &request {
            EndpointOperationRequest::Read(_) => AuditOperation::READ,
            EndpointOperationRequest::Write(_) => AuditOperation::WRITE,
        };
        let target_value = match &request {
            EndpointOperationRequest::Write(write) if write.property_value.len() <= 32 => {
                Some(write.property_value.clone())
            }
            _ => None,
        };
        // Wait for the destination's turn first (#1542), in the caller's
        // task: a wait holds no admission permit and no database guard, a
        // caller giving up then cancels nothing admitted, and the record's
        // timestamp below is taken once the request may go.
        let operation = requester
            .pace_operation(destination, attributes, request)
            .await?;
        // Bound every retained operation, including time before lease acquisition
        // and after dispatch releases the request lease. Never spawn permit waiters.
        let permit = Arc::clone(&self.operations)
            .try_acquire_owned()
            .map_err(|_| Error::Encoding("source Audit admission is closed or full".into()))?;
        let mut db = self.db.write().await;
        if self.operations.is_closed() {
            return Err(Error::Encoding("endpoint shutdown".into()));
        }
        let runtime = self
            .runtime
            .upgrade()
            .filter(|runtime| runtime.owner.is_active())
            .ok_or_else(|| Error::Encoding("endpoint shutdown".into()))?;
        let reporter = db
            .get(&self.selected)
            .and_then(|object| object.audit_reporter_internal())
            .ok_or_else(|| Error::Encoding("source Reporter is unavailable".into()))?;
        if reporter
            .property_list()
            .contains(&PropertyIdentifier::MONITORED_OBJECTS)
        {
            reporter.status_internal().set_configured(false);
            return Err(Error::Encoding(
                "source Audit does not support Monitored_Objects".into(),
            ));
        }
        let level = match reporter.read_property(PropertyIdentifier::AUDIT_LEVEL, None)? {
            PropertyValue::Enumerated(level) => AuditLevel::from_raw(level),
            _ => return Err(Error::Encoding("invalid source audit level".into())),
        };
        let operations =
            match reporter.read_property(PropertyIdentifier::AUDITABLE_OPERATIONS, None)? {
                PropertyValue::BitString { unused_bits, data } => {
                    AuditOperationFlags::from_bacnet(unused_bits, &data)?
                }
                _ => return Err(Error::Encoding("invalid source operation flags".into())),
            };
        let priority_allowed = if let Some(priority) = command_priority {
            match reporter.read_property(PropertyIdentifier::AUDIT_PRIORITY_FILTER, None)? {
                PropertyValue::BitString { unused_bits, data } => {
                    bacnet_types::bitstring::BACnetPriorityFilter::from_bacnet(unused_bits, &data)?
                        .contains(priority)
                }
                _ => return Err(Error::Encoding("invalid source priority filter".into())),
            }
        } else {
            true
        };
        let eligible: Vec<_> = identities
            .into_iter()
            .enumerate()
            .filter(|(_, (_, property, _))| {
                level != AuditLevel::NONE
                    && operations.contains(audit_operation)
                    && priority_allowed
                    && !(level == AuditLevel::AUDIT_CONFIG
                        && *property == PropertyIdentifier::PRESENT_VALUE)
            })
            .collect();
        if eligible.is_empty() {
            drop(db);
            drop(runtime);
            drop(permit);
            return operation.prepare()?.execute().await.result;
        }
        let status = reporter.status_internal();
        let confirmed = reporter.confirmed_internal();
        let route = runtime.recipient_route(&db);
        status.set_configured(route.is_some());
        let Some(route) = route else {
            drop(db);
            drop(runtime);
            drop(permit);
            return operation.prepare()?.execute().await.result;
        };
        // A routed destination on this network goes to the station directly
        // (#1403), as pacing localized it, so it is audited as the direct
        // operation it is.
        let EndpointApduDestination::Direct { destination_mac } = operation.destination() else {
            return Err(Error::Encoding(
                "audited operation requires direct B/IP IPv4 unicast".into(),
            ));
        };
        let destination_mac = destination_mac.clone();
        let (ip, port) = decode_bip_mac(&destination_mac)?;
        let ip = Ipv4Addr::from(ip);
        if ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_broadcast()
            || ip == self.broadcast
            || port == 0
        {
            return Err(Error::Encoding(
                "audited operation requires direct B/IP IPv4 unicast".into(),
            ));
        }
        let device = self.device;
        if db.local_device().identifier() != Some(device) {
            status.set_configured(false);
            return Err(Error::Encoding("source Device is unavailable".into()));
        }
        let operation = operation.prepare()?;
        let timestamp = match db
            .clock_frame()
            .filter(|frame| frame.is_valid_actual_datetime())
        {
            Some(frame) => BACnetTimeStamp::DateTime {
                date: frame.local_date,
                time: frame.local_time,
            },
            None => BACnetTimeStamp::SequenceNumber(db.next_event_sequence_number()),
        };
        let failure = status.auditing_failure_epoch().and_then(|epoch| {
            self.failures.observe(AuditFailureContext {
                status: Arc::clone(&status),
                epoch,
                device,
                confirmed,
                peer: CanonicalPeer::direct(route.as_slice()),
                route: route.clone(),
                max_apdu: u32::from(self.max_apdu),
            })
        });
        let completion = status.begin_delivery();
        let notification = BACnetAuditNotification {
            source_timestamp: Some(timestamp),
            target_timestamp: None,
            source_device: BACnetRecipient::Device(device),
            source_object: None,
            operation: audit_operation,
            source_comment: None,
            target_comment: None,
            invoke_id: Some(operation.invoke_id()),
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: destination_mac,
            }),
            target_object: None,
            target_property: None,
            target_priority: command_priority,
            target_value,
            current_value: None,
            result: None,
        };
        status.set_configured(true);
        drop(db);
        drop(runtime);
        let owner = self
            .notifications
            .upgrade()
            .ok_or_else(|| Error::Encoding("endpoint shutdown".into()))?;
        let weak_owner = Arc::downgrade(&owner);
        let source = Arc::clone(self);
        let (reply, response) = oneshot::channel();
        // Spawn/close are serialized by the same worker owner. A rejected spawn
        // drops the operation, permit and reply. The worker never retains owner.
        owner.spawn(async move {
            let _permit = permit;
            let outcome = operation.execute().await;
            if outcome.attempted {
                // Correlation of the complete ACK precedes every occurrence's
                // projection. Whole-operation failure fans out the same failure.
                let results = outcome
                    .result
                    .as_ref()
                    .ok()
                    .and_then(EndpointOperationAck::as_read)
                    .map(EndpointReadAck::audit_results);
                let device = outcome
                    .result
                    .as_ref()
                    .ok()
                    .and_then(EndpointOperationAck::as_read)
                    .and_then(EndpointReadAck::target_device);
                if let Some(owner) = weak_owner.upgrade() {
                    for (position, (object, property, index)) in eligible {
                        let mut record = notification.clone();
                        record.target_object = Some(object);
                        record.target_property = Some(AuditPropertyReference {
                            property_identifier: property,
                            property_array_index: index.map(u64::from),
                        });
                        if let Some(results) = &results {
                            record.target_object = Some(results[position].0);
                            record.result = results[position].1;
                        } else {
                            record.result = delivery::result(&outcome.result);
                        }
                        if let Some(device) = device {
                            record.target_device = BACnetRecipient::Device(device);
                        }
                        delivery::admit(
                            &source,
                            &owner,
                            confirmed,
                            route.clone(),
                            record,
                            delivery::Completion::new(Arc::clone(&status), completion),
                            failure.clone(),
                        );
                    }
                }
            }
            let _ = reply.send(outcome.result);
        });
        drop(owner);
        response
            .await
            .map_err(|_| Error::Encoding("endpoint shutdown".into()))?
    }
}

#[cfg(test)]
#[path = "source_read_failure_admission_tests.rs"]
mod failure_admission_tests;
