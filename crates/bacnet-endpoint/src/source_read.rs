//! Session-private source READ ownership. No discovery cache or persistent outbox.
use std::net::Ipv4Addr;
use std::sync::{Arc, Weak};

use bacnet_client::EndpointRequester;
use bacnet_endpoint_core::endpoint_ingress::{EndpointApduDestination, EndpointEgress};
use bacnet_objects::audit::AuditReporterStatus;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::__endpoint_NotificationTransactions as NotificationTransactions;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_transport::port::DataAttribute;
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAddress, BACnetAuditNotification, BACnetRecipient,
};
use bacnet_types::enums::{AuditLevel, AuditOperation, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use tokio::sync::{oneshot, RwLock, Semaphore};

use crate::bip::StaticSourceAuditRecipient;

#[path = "source_read_delivery.rs"]
mod delivery;

pub(crate) struct SourceRead {
    db: Arc<RwLock<ObjectDatabase>>,
    selected: ObjectIdentifier,
    recipient: StaticSourceAuditRecipient,
    broadcast: Ipv4Addr,
    egress: EndpointEgress,
    notifications: Weak<NotificationTransactions>,
    operations: Arc<Semaphore>,
    max_apdu: u16,
}

impl SourceRead {
    pub(crate) fn new(
        db: Arc<RwLock<ObjectDatabase>>,
        selected: ObjectIdentifier,
        recipient: StaticSourceAuditRecipient,
        broadcast: Ipv4Addr,
        egress: EndpointEgress,
        notifications: &Arc<NotificationTransactions>,
        max_apdu: u16,
    ) -> Arc<Self> {
        if let Some(reporter) = db
            .try_read()
            .expect("unshared startup database")
            .get(&selected)
            .and_then(|object| object.audit_reporter_internal())
        {
            reporter.status_internal().set_configured(true);
        }
        Arc::new(Self {
            db,
            selected,
            recipient,
            broadcast,
            egress,
            notifications: Arc::downgrade(notifications),
            operations: Arc::new(Semaphore::new(64)),
            max_apdu,
        })
    }

    #[cfg(test)]
    pub(crate) fn available_operations(&self) -> usize {
        self.operations.available_permits()
    }

    pub(crate) fn close(&self) {
        self.operations.close();
    }

    pub(crate) async fn read(
        &self,
        requester: &EndpointRequester,
        destination: EndpointApduDestination,
        attributes: Vec<DataAttribute>,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        // Bound every retained operation, including time before lease acquisition
        // and after dispatch releases the request lease. Never spawn permit waiters.
        let permit = Arc::clone(&self.operations)
            .try_acquire_owned()
            .map_err(|_| Error::Encoding("source READ admission is closed or full".into()))?;
        let mut db = self.db.write().await;
        if self.operations.is_closed() {
            return Err(Error::Encoding("endpoint shutdown".into()));
        }
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
                "source READ does not support Monitored_Objects".into(),
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
        if level == AuditLevel::NONE
            || !operations.contains(AuditOperation::READ)
            || (level == AuditLevel::AUDIT_CONFIG && property == PropertyIdentifier::PRESENT_VALUE)
        {
            drop(db);
            drop(permit);
            return requester
                .read_property_with_destination(destination, attributes, object, property, index)
                .await;
        }
        let status = reporter.status_internal();
        let confirmed = reporter.confirmed_internal();
        let EndpointApduDestination::Direct { destination_mac } = &destination else {
            return Err(Error::Encoding(
                "audited READ requires direct B/IP IPv4 unicast".into(),
            ));
        };
        let (ip, port) = decode_bip_mac(destination_mac)?;
        let ip = Ipv4Addr::from(ip);
        if ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_broadcast()
            || ip == self.broadcast
            || port == 0
        {
            return Err(Error::Encoding(
                "audited READ requires direct B/IP IPv4 unicast".into(),
            ));
        }
        let devices = db.find_by_type(ObjectType::DEVICE);
        if devices.len() != 1 {
            status.set_configured(false);
            return Err(Error::Encoding("source Device is unavailable".into()));
        }
        let operation = requester.prepare_read_property(
            destination.clone(),
            attributes,
            object,
            property,
            index,
        )?;
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
        let mut notification = BACnetAuditNotification {
            source_timestamp: Some(timestamp),
            target_timestamp: None,
            source_device: BACnetRecipient::Device(devices[0]),
            source_object: None,
            operation: AuditOperation::READ,
            source_comment: None,
            target_comment: None,
            invoke_id: Some(operation.invoke_id()),
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: destination_mac.clone(),
            }),
            target_object: Some(object),
            target_property: Some(AuditPropertyReference {
                property_identifier: property,
                property_array_index: index.map(u64::from),
            }),
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        };
        status.set_configured(true);
        drop(db);
        let owner = self
            .notifications
            .upgrade()
            .ok_or_else(|| Error::Encoding("endpoint shutdown".into()))?;
        let weak_owner = Arc::downgrade(&owner);
        let egress = self.egress.clone();
        let recipient = self.recipient;
        let max_apdu = self.max_apdu;
        let (reply, response) = oneshot::channel();
        // Spawn/close are serialized by the same worker owner. A rejected spawn
        // drops the operation, permit and reply. The worker never retains owner.
        owner.spawn(async move {
            let _permit = permit;
            let outcome = operation.execute().await;
            if outcome.attempted {
                notification.result = delivery::result(&outcome.result);
                if let Some(owner) = weak_owner.upgrade() {
                    delivery::admit(
                        &owner,
                        egress,
                        recipient,
                        max_apdu,
                        confirmed,
                        notification,
                        status,
                    );
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
