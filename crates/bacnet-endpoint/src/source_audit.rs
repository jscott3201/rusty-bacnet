//! Session-private source Audit ownership. No discovery cache or persistent outbox.
use bacnet_objects::property_metadata::has_property;
use std::net::Ipv4Addr;
use std::sync::{Arc, Weak};

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
use bacnet_types::MacAddr;
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAddress, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditLevel, AuditOperation, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use tokio::sync::{RwLock, oneshot};

#[path = "source_recipient.rs"]
pub(crate) mod recipient;
#[path = "source_routers.rs"]
pub(crate) mod routers;
use bacnet_objects::database::AuditOwnership;
use bacnet_objects::device::AuditRecipientChangeSink;
use recipient::{SourceRecipient, SourceRoutes};
use routers::{SourceRoute, SourceRouters};

#[path = "source_audit_delivery.rs"]
mod delivery;
#[path = "source_audit_failures.rs"]
mod failures;
#[path = "source_audit_local.rs"]
mod local;
#[path = "source_audit_seated.rs"]
mod seated;
#[path = "source_audit_slots.rs"]
pub(crate) mod slots;
pub(crate) use seated::SeatAuditor;

pub(crate) struct SourceAudit {
    /// Released off the runtime by whichever caller, worker or session
    /// drops the source last (#1561).
    db: bacnet_server::caller_waits::HeldDatabase,
    selected: ObjectIdentifier,
    device: ObjectIdentifier,
    runtime: Weak<SourceRecipient>,
    broadcast: Ipv4Addr,
    egress: EndpointEgress,
    notifications: Weak<NotificationTransactions>,
    /// The 64 operation slots, shared out by lane (#2652).
    slots: Arc<slots::Slots>,
    max_apdu: u16,
    failures: Arc<AuditFailureQueue<SourceRoute>>,
    /// The routers learned for routed bindings' networks (#2371).
    routers: Arc<SourceRouters>,
    #[cfg(test)]
    pub(crate) summary_queue_full: tokio::sync::Notify,
}

impl SourceAudit {
    /// Install the source on `database`, the guarded contents of `db`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        db: Arc<RwLock<ObjectDatabase>>,
        database: &mut ObjectDatabase,
        selected: ObjectIdentifier,
        routes: SourceRoutes,
        broadcast: Ipv4Addr,
        egress: EndpointEgress,
        notifications: &Arc<NotificationTransactions>,
        max_apdu: u16,
    ) -> Result<(Arc<Self>, Arc<SourceRecipient>), Error> {
        let device = database
            .local_device()
            .identifier()
            .expect("preflight local Device");
        let status = database
            .get(&selected)
            .and_then(|object| object.audit_reporter())
            .expect("preflight Reporter")
            .status_internal();
        let initial = database
            .get_mut(&device)
            .and_then(|object| object.device_authority_internal())
            .and_then(|authority| authority.provisioned_audit_recipient().cloned())
            .expect("preflight recipient");
        routes.validate_initial(&initial)?;
        // A confirmed reporter has no route to a station at a broadcast or
        // group form on another network (#2732).
        let confirmed = status.configuration().confirmed;
        status.set_configured(
            routes
                .resolve(&initial)
                .is_some_and(|route| route.serves(confirmed)),
        );
        let failures = Arc::new(AuditFailureQueue::default());
        let routers = Arc::clone(routes.routers());
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
            db: bacnet_server::caller_waits::HeldDatabase::new(Arc::clone(&db)),
            selected,
            device,
            runtime: Arc::downgrade(&runtime),
            broadcast,
            egress,
            notifications: Arc::downgrade(notifications),
            slots: Arc::new(slots::Slots::new()),
            max_apdu,
            failures,
            routers,
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
        self.slots.available()
    }

    /// Whether a stop or drop of the session has sealed admission.
    #[cfg(test)]
    pub(crate) fn sealed(&self) -> bool {
        self.slots.is_closed()
    }

    pub(crate) fn close(&self) {
        if let Some(runtime) = self.runtime.upgrade() {
            runtime.seal();
        }
        self.slots.close();
    }

    /// An audited operation goes to one station: a direct unicast on the
    /// link, or one station on another network, by the rule a routed
    /// binding meets (#2371), which the record names by that network and
    /// MAC.
    fn one_station(
        &self,
        runtime: &SourceRecipient,
        network: u16,
        mac: &[u8],
    ) -> Result<(), Error> {
        let unicast = if network != 0 {
            routers::routed_unicast(network, mac)
        } else if runtime.routes.is_bip() {
            let (ip, port) = decode_bip_mac(mac)?;
            let ip = Ipv4Addr::from(ip);
            !(ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip == self.broadcast
                || port == 0)
        } else {
            // Another link: a direct unicast MAC there (#2357).
            runtime.routes.link().unicast(mac)
        };
        if unicast {
            Ok(())
        } else {
            Err(Error::invalid_argument(one_station_rule(runtime)))
        }
    }

    /// Report an operation carried out on this device as a whole, such as
    /// an accepted ReinitializeDevice reset, backup or restore step (Clause
    /// 19.6, Table 19-5, #1564), with `source` as its requester. With no
    /// `source`, the operation is the application's own (a local reset,
    /// #1837): this device is the source and the record has no invoke ID.
    ///
    /// Clause 19.6.2 casts this Reporter as the source-side one; sending
    /// these target-side records through it, the session's only Reporter,
    /// is a local choice.
    ///
    /// The session's one Reporter filters it by its Audit_Level and
    /// Auditable_Operations. The record names this Device as its target and
    /// no object, property or value, carries the target timestamp, and has
    /// no Result: only an operation carried out is reported. It goes to the
    /// Device's recipient as the source records do, admitted before this
    /// returns. Called under the database guard.
    pub(crate) fn device_operation(
        self: &Arc<Self>,
        db: &mut ObjectDatabase,
        operation: AuditOperation,
        source: Option<&bacnet_objects::device::AuditWriteSource>,
    ) {
        if self.slots.is_closed() {
            return;
        }
        let Some(runtime) = self
            .runtime
            .upgrade()
            .filter(|runtime| runtime.owner.is_active())
        else {
            return;
        };
        let Some(reporter) = db
            .get(&self.selected)
            .and_then(|object| object.audit_reporter())
        else {
            return;
        };
        let level = match reporter.read_property(PropertyIdentifier::AUDIT_LEVEL, None) {
            Ok(PropertyValue::Enumerated(level)) => AuditLevel::from_raw(level),
            _ => return,
        };
        let operations =
            match reporter.read_property(PropertyIdentifier::AUDITABLE_OPERATIONS, None) {
                Ok(PropertyValue::BitString { unused_bits, data }) => {
                    match AuditOperationFlags::from_bacnet(unused_bits, &data) {
                        Ok(operations) => operations,
                        Err(_) => return,
                    }
                }
                _ => return,
            };
        if level == AuditLevel::NONE || !operations.contains(operation) {
            return;
        }
        let status = reporter.status_internal();
        let confirmed = reporter.confirmed_internal();
        let route = runtime
            .recipient_route(db)
            .filter(|route| route.serves(confirmed));
        status.set_configured(route.is_some());
        let (Some(route), Some(owner)) = (route, self.notifications.upgrade()) else {
            return;
        };
        let device = self.device;
        if db.local_device().identifier() != Some(device) {
            status.set_configured(false);
            return;
        }
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
                peer: route.peer(),
                route: route.clone(),
                max_apdu: u32::from(self.max_apdu),
            })
        });
        let completion = delivery::Completion::new(Arc::clone(&status), status.begin_delivery());
        let record = BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: Some(timestamp),
            source_device: source.map_or(BACnetRecipient::Device(device), |source| {
                source.device.clone()
            }),
            source_object: None,
            operation,
            source_comment: None,
            target_comment: None,
            invoke_id: source.map(|source| source.invoke_id),
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Device(device),
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        };
        delivery::admit(self, &owner, confirmed, route, record, completion, failure);
    }
}

/// What an audited operation to anything but one station is refused with.
fn one_station_rule(runtime: &SourceRecipient) -> &'static str {
    if runtime.routes.is_bip() {
        "audited operation requires direct B/IP IPv4 unicast or a routed station"
    } else {
        "audited operation requires direct unicast on the link or a routed station"
    }
}

#[cfg(test)]
#[path = "source_read_failure_admission_tests.rs"]
mod failure_admission_tests;
