//! Source runtime's Device mutation owner and immutable direct B/IP route facts.
use super::*;
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::{AuditOwnership, EventSequence};
use bacnet_objects::device::{AuditRecipientChangeSink, AuditWriteSource};
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use std::collections::HashMap;
use std::net::SocketAddrV4;

pub(crate) struct SourceRoutes {
    devices: HashMap<ObjectIdentifier, MacAddr>,
    broadcast: SocketAddrV4,
    local_network: LocalNetworkNumber,
}
impl SourceRoutes {
    pub(crate) fn new(
        bindings: &[(ObjectIdentifier, SocketAddrV4)],
        broadcast: SocketAddrV4,
    ) -> Result<Self, Error> {
        let mut routes = Self {
            devices: HashMap::new(),
            broadcast,
            local_network: LocalNetworkNumber::default(),
        };
        for (device, address) in bindings {
            if device.object_type() != ObjectType::DEVICE
                || device.instance_number() == ObjectIdentifier::MAX_INSTANCE
            {
                return Err(Error::Encoding(
                    "source Audit binding requires a concrete Device".into(),
                ));
            }
            let mac = MacAddr::from_slice(&bacnet_transport::bvll::encode_bip_mac(
                address.ip().octets(),
                address.port(),
            ));
            if !routes.valid_address(&mac) {
                return Err(denied());
            }
            if routes.devices.insert(*device, mac).is_some() {
                return Err(Error::Encoding(
                    "duplicate source Audit Device binding".into(),
                ));
            }
        }
        Ok(routes)
    }
    /// Take the started link's broadcast endpoint and the layer's number
    /// slot, which the session's Number owner publishes into.
    pub(crate) fn finalize(&mut self, broadcast: SocketAddrV4, local_network: LocalNetworkNumber) {
        self.broadcast = broadcast;
        self.local_network = local_network;
    }
    fn valid_address(&self, mac: &[u8]) -> bool {
        bacnet_server::server::valid_bip_audit_address(&BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(mac),
        }) && !(mac[..4] == self.broadcast.ip().octets()
            && mac[4..] == self.broadcast.port().to_be_bytes())
    }
    /// The direct B/IP MAC a recipient is sent to, if it has one. An address
    /// naming this network by its number, once the number is known, is a
    /// station on this link like a local address (#1403), so its records go
    /// there with no DNET. Each call reads the number as it is then, so a
    /// number learned after startup applies from the next record. An address
    /// on any other network has no route.
    pub(crate) fn resolve(&self, recipient: &BACnetRecipient) -> Option<MacAddr> {
        let here = |network: u16| network == 0 || Some(network) == self.local_network.get();
        let mac = match recipient {
            BACnetRecipient::Device(device) => self.devices.get(device)?,
            BACnetRecipient::Address(address) if here(address.network_number) => {
                &address.mac_address
            }
            _ => return None,
        };
        self.valid_address(mac).then(|| mac.clone())
    }
    /// Whether `recipient` is an Address on a network numbered 1 to 65534
    /// that is not the session's number in force, with a MAC this source
    /// could send to: no route now, but one whenever that number is the
    /// session's (#1461). One provisioned so starts unresolved, reporting
    /// CONFIGURATION_ERROR as a Device with no binding does; a session
    /// without a registered port learns its number only after it starts.
    /// One the number moved away from no longer holds up a recipient change:
    /// it gets no copy of the change record, since nothing reaches it.
    pub(crate) fn awaits_local_number(&self, recipient: &BACnetRecipient) -> bool {
        let BACnetRecipient::Address(address) = recipient else {
            return false;
        };
        (1..=0xFFFE).contains(&address.network_number)
            && Some(address.network_number) != self.local_network.get()
            && self.valid_address(&address.mac_address)
    }
    /// Refuse a provisioned Address recipient with no route, unless the
    /// session's number may yet name its network (#1461).
    pub(crate) fn validate_initial(&self, recipient: &BACnetRecipient) -> Result<(), Error> {
        if matches!(recipient, BACnetRecipient::Address(_))
            && self.resolve(recipient).is_none()
            && !self.awaits_local_number(recipient)
        {
            Err(denied())
        } else {
            Ok(())
        }
    }
}
fn denied() -> Error {
    Error::protocol(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
}

pub(crate) struct SourceRecipient {
    pub(crate) owner: Arc<AuditOwnership>,
    pub(super) device: ObjectIdentifier,
    pub(super) status: Arc<AuditReporterStatus>,
    pub(super) sequence: Arc<EventSequence>,
    pub(super) routes: SourceRoutes,
    pub(super) notifications: Weak<NotificationTransactions>,
    pub(super) egress: EndpointEgress,
    pub(super) max_apdu: u16,
    pub(super) failures: Arc<AuditFailureQueue<MacAddr>>,
}
impl SourceRecipient {
    pub(crate) fn write_local(
        &self,
        db: &mut ObjectDatabase,
        recipient: Option<BACnetRecipient>,
    ) -> Result<(), Error> {
        let value = match recipient {
            None => PropertyValue::Null,
            Some(recipient) => {
                let mut bytes = bytes::BytesMut::new();
                // A MAC past BACnetAddress::MAX_MAC_LEN does not encode; the
                // Device refuses it the same way when a write carries one.
                bacnet_encoding::constructed::encode_recipient(&mut bytes, &recipient).map_err(
                    |_| {
                        Error::Remote(RemoteError::Error {
                            class: ErrorClass::PROPERTY,
                            code: ErrorCode::INVALID_DATA_ENCODING,
                        })
                    },
                )?;
                PropertyValue::ApplicationData(bytes.to_vec())
            }
        };
        db.get_mut(&self.device)
            .and_then(|object| object.device_authority_internal())
            .filter(|authority| authority.object_identifier() == self.device)
            .ok_or_else(denied)?
            .write_audit_recipient(None, value, None, None)
    }

    /// The direct MAC the Device's current Audit_Notification_Recipient
    /// resolves to now, if any.
    pub(crate) fn recipient_route(&self, db: &ObjectDatabase) -> Option<MacAddr> {
        let value = db
            .get(&self.device)?
            .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
            .ok()?;
        let PropertyValue::ApplicationData(bytes) = value else {
            return None;
        };
        let (recipient, _) = bacnet_encoding::constructed::decode_recipient(&bytes, 0).ok()?;
        self.routes.resolve(&recipient)
    }

    /// The session's number changed: whether the recipient resolves may
    /// have changed with it, so the Reporter's CONFIGURATION_ERROR follows
    /// the route in force now, not at the next audited operation (#1461).
    pub(crate) fn number_changed(&self, db: &ObjectDatabase) {
        self.status
            .set_configured(self.recipient_route(db).is_some());
    }

    pub(crate) fn seal(&self) {
        match self.notifications.upgrade() {
            Some(notifications) => {
                notifications.seal_audit_owner(&self.owner);
            }
            _ => {
                self.owner.seal();
            }
        }
    }
    pub(crate) fn uninstall(self: &Arc<Self>, db: &mut ObjectDatabase) {
        let sink: Arc<dyn AuditRecipientChangeSink> = self.clone();
        if let Some(mut authority) = db
            .get_mut(&self.device)
            .and_then(|object| object.device_authority_internal())
        {
            authority.uninstall_audit_recipient(&sink);
        }
        db.release_audit_internal(&self.owner);
    }
    fn commit_recipient(
        &self,
        current: &mut BACnetRecipient,
        new: BACnetRecipient,
        source: Option<&AuditWriteSource>,
        timestamp: BACnetTimeStamp,
    ) -> Result<(), Error> {
        let notifications = self.notifications.upgrade().ok_or_else(denied)?;
        notifications.commit_audit(|| {
            if !self.owner.is_active() {
                return Err(denied());
            }
            // An old Address the session's number no longer, or not yet,
            // names has no route, but does not refuse the change (#1461):
            // the record goes by global broadcast in its place. Any other
            // old recipient must resolve.
            let old_route = match self.routes.resolve(current) {
                Some(route) => Some(route),
                None if self.routes.awaits_local_number(current) => None,
                None => return Err(denied()),
            };
            let new_route = self.routes.resolve(&new).ok_or_else(denied)?;
            let mut old_value = bytes::BytesMut::new();
            let mut new_value = bytes::BytesMut::new();
            bacnet_encoding::constructed::encode_recipient(&mut old_value, current)?;
            bacnet_encoding::constructed::encode_recipient(&mut new_value, &new)?;
            let record = BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: Some(timestamp),
                source_device: source.map_or(BACnetRecipient::Device(self.device), |source| {
                    source.device.clone()
                }),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: source.map(|source| source.invoke_id),
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(self.device),
                target_object: Some(self.device),
                target_property: Some(AuditPropertyReference {
                    property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    property_array_index: None,
                }),
                target_priority: None,
                target_value: Some(new_value.to_vec()),
                current_value: Some(old_value.to_vec()),
                result: None,
            };
            // Clause 12.11.66: the change's record reaches both recipients,
            // or goes by global broadcast. With no route to the old one, the
            // new recipient gets its copy and an unconfirmed global
            // broadcast (`None`) stands in for the old one's.
            let targets = match old_route {
                Some(old_route) => [Some(old_route), Some(new_route)],
                None => [Some(new_route), None],
            };
            self.status.commit_recipient_change(|confirmed, token| {
                let mut attempts = Vec::with_capacity(2);
                for target in targets {
                    let permit = notifications.try_admit_audit().map_err(|_| denied())?;
                    let reservation = match &target {
                        Some(route) if confirmed => Some(
                            notifications
                                .reserve(
                                    CanonicalPeer::direct(route),
                                    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                                )
                                .map_err(|_| denied())?,
                        ),
                        _ => None,
                    };
                    let invoke = reservation
                        .as_ref()
                        .map_or(0, |(operation, _)| operation.invoke_id());
                    let encoded =
                        delivery::encode(&record, reservation.is_some(), self.max_apdu, invoke)
                            .ok_or_else(denied)?;
                    attempts.push((target, permit, reservation, encoded));
                }
                *current = new;
                let egress = self.egress.clone();
                let deadline = tokio::time::Instant::now() + delivery::DEADLINE;
                let first_completion = delivery::Completion::new(Arc::clone(&self.status), token);
                let second_completion = delivery::Completion::new(Arc::clone(&self.status), token);
                let mut attempts = attempts.into_iter();
                let first = attempts.next().unwrap();
                let second = attempts.next().unwrap();
                Ok(async move {
                    let run = |(target, permit, reserved, bytes): (Option<MacAddr>, _, _, _),
                               completion: delivery::Completion| {
                        let egress = egress.clone();
                        async move {
                            let _permit: tokio::sync::OwnedSemaphorePermit = permit;
                            let sent = match target {
                                Some(route) => delivery::admit_encoded(
                                    &egress, route, bytes, confirmed, deadline,
                                ),
                                None => delivery::admit_global_broadcast(&egress, bytes, deadline),
                            };
                            completion
                                .finish(delivery::finish_send(sent, reserved, deadline).await);
                        }
                    };
                    tokio::join!(run(first, first_completion), run(second, second_completion));
                })
            })
        })?;
        self.failures.recipient_changed();
        Ok(())
    }
}
impl AuditRecipientChangeSink for SourceRecipient {
    fn is_active(&self) -> bool {
        self.owner.is_active()
    }
    fn change(
        &self,
        current: &mut BACnetRecipient,
        new: BACnetRecipient,
        source: Option<&AuditWriteSource>,
        clock: Option<ClockFrame>,
    ) -> Result<(), Error> {
        match clock.filter(|frame| frame.is_valid_actual_datetime()) {
            Some(frame) => self.commit_recipient(
                current,
                new,
                source,
                BACnetTimeStamp::DateTime {
                    date: frame.local_date,
                    time: frame.local_time,
                },
            ),
            None => self.sequence.transaction(|number| {
                self.commit_recipient(
                    current,
                    new,
                    source,
                    BACnetTimeStamp::SequenceNumber(number),
                )
            }),
        }
    }
}
