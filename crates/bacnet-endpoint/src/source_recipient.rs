//! Source runtime's Device mutation owner and immutable route facts: direct
//! on IPv4 B/IP or any other link (#2357), or behind a router (#2371).
use super::routers::{Hop, SourceRoute, SourceRouters, routed_unicast};
use super::*;
use bacnet_network::network_number::LocalNetworkNumber;
use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::{AuditOwnership, EventSequence};
use bacnet_objects::device::{AuditRecipientChangeSink, AuditWriteSource};
use bacnet_transport::port::GroupDestinations;
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use std::collections::HashMap;
use std::net::SocketAddrV4;

/// Which MACs on the session's link can be a source Audit recipient: a
/// direct unicast destination there.
#[derive(Clone, Debug)]
pub(crate) enum SourceLink {
    /// IPv4 B/IP, by the link's broadcast endpoint.
    Bip(SocketAddrV4),
    /// Any other link (B/IPv6, BACnet/SC, MS/TP and the rest), by its own
    /// MAC and group-destination rule (#2357).
    Other {
        /// This node's MAC, in the link's format; empty while it is learned
        /// only at start (B/IPv6's VMAC), when the length and self checks
        /// wait for it.
        local: MacAddr,
        /// The transport's group destinations.
        groups: GroupDestinations,
    },
}

impl SourceLink {
    /// Whether `mac` is a direct unicast destination on this link.
    ///
    /// - IPv4 B/IP: a unicast address and nonzero port, and never the link's
    ///   broadcast address, at any port, as the server's Device bindings
    ///   refuse it (#2348).
    /// - Any other link: a MAC as long as this node's, not this node's own,
    ///   not a group destination by the transport's rule, not the all-ones
    ///   broadcast of a one-octet (MS/TP's 255) or six-octet (SC's broadcast
    ///   VMAC, Ethernet's broadcast) link, and not a six-octet all-zero MAC,
    ///   which names no device: SC reserves that VMAC for one not yet known
    ///   (Annex AB.1.5.2, H.7.3), and it is no Ethernet station either.
    pub(crate) fn unicast(&self, mac: &[u8]) -> bool {
        match self {
            Self::Bip(broadcast) => {
                bacnet_server::server::valid_bip_audit_address(&BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(mac),
                }) && mac[..4] != broadcast.ip().octets()
            }
            Self::Other { local, groups } => {
                let reserved = match mac.len() {
                    1 => mac == [0xFF],
                    6 => mac == [0xFF; 6] || mac == [0; 6],
                    _ => false,
                };
                !mac.is_empty()
                    && (local.is_empty() || (mac.len() == local.len() && mac != local.as_slice()))
                    && !groups.contains(mac)
                    && !reserved
            }
        }
    }
}

pub(crate) struct SourceRoutes {
    devices: HashMap<ObjectIdentifier, SourceRoute>,
    link: SourceLink,
    local_network: LocalNetworkNumber,
    routers: Arc<SourceRouters>,
}
impl SourceRoutes {
    /// IPv4 B/IP routes, with `broadcast` the link's broadcast endpoint.
    pub(crate) fn new(
        bindings: &[(ObjectIdentifier, SocketAddrV4)],
        broadcast: SocketAddrV4,
    ) -> Result<Self, Error> {
        let bindings: Vec<_> = bindings
            .iter()
            .map(|(device, address)| {
                (
                    *device,
                    MacAddr::from_slice(&bacnet_transport::bvll::encode_bip_mac(
                        address.ip().octets(),
                        address.port(),
                    )),
                )
            })
            .collect();
        Self::for_link(&bindings, SourceLink::Bip(broadcast))
    }
    /// Routes on `link`, each binding a Device to a MAC there.
    pub(crate) fn for_link(
        bindings: &[(ObjectIdentifier, MacAddr)],
        link: SourceLink,
    ) -> Result<Self, Error> {
        let mut routes = Self {
            devices: HashMap::new(),
            link,
            local_network: LocalNetworkNumber::default(),
            routers: Arc::default(),
        };
        for (device, mac) in bindings {
            if !routes.valid_address(mac) {
                return Err(denied());
            }
            routes.bind(*device, SourceRoute::Direct(mac.clone()))?;
        }
        Ok(routes)
    }
    /// Add routed bindings, each a Device behind a router by its network
    /// number and its MAC there (#2371). Refused with SERVICES /
    /// SERVICE_REQUEST_DENIED unless [`routed_unicast`]: network 0, the
    /// global network 65535, an empty MAC or one past 18 octets.
    pub(crate) fn bind_routed(
        mut self,
        bindings: &[(ObjectIdentifier, u16, MacAddr)],
    ) -> Result<Self, Error> {
        for (device, network, mac) in bindings {
            if !routed_unicast(*network, mac) {
                return Err(denied());
            }
            self.routers.track(*network);
            self.bind(
                *device,
                SourceRoute::Routed {
                    network: *network,
                    mac: mac.clone(),
                },
            )?;
        }
        Ok(self)
    }
    fn bind(&mut self, device: ObjectIdentifier, route: SourceRoute) -> Result<(), Error> {
        if device.object_type() != ObjectType::DEVICE
            || device.instance_number() == ObjectIdentifier::MAX_INSTANCE
        {
            return Err(Error::Encoding(
                "source Audit binding requires a concrete Device".into(),
            ));
        }
        if self.devices.insert(device, route).is_some() {
            return Err(Error::Encoding(
                "duplicate source Audit Device binding".into(),
            ));
        }
        Ok(())
    }
    /// The routers learned for the routed bindings' networks.
    pub(crate) fn routers(&self) -> &Arc<SourceRouters> {
        &self.routers
    }
    /// Whether these are IPv4 B/IP routes.
    pub(crate) fn is_bip(&self) -> bool {
        matches!(self.link, SourceLink::Bip(_))
    }
    /// The link's unicast rule.
    pub(crate) fn link(&self) -> &SourceLink {
        &self.link
    }
    /// Take the started link's facts (B/IP's broadcast endpoint, another
    /// link's own MAC) and the layer's number slot, which the session's
    /// Number owner publishes into.
    pub(crate) fn finalize(&mut self, link: SourceLink, local_network: LocalNetworkNumber) {
        self.routers.finalize(link.clone(), local_network.clone());
        self.link = link;
        self.local_network = local_network;
    }
    /// A direct unicast MAC on the link ([`SourceLink::unicast`]).
    fn valid_address(&self, mac: &[u8]) -> bool {
        self.link.unicast(mac)
    }
    /// The route a recipient's records take, if it has one. An address
    /// naming this network by its number, once the number is known, is a
    /// station on this link like a local address (#1403), so its records go
    /// there with no DNET; so does a routed binding to it. Each call reads
    /// the number as it is then, so a number learned after startup applies
    /// from the next record. A Device bound behind a router on another
    /// network goes there through it (#2371); an address on another network
    /// has no route.
    pub(crate) fn resolve(&self, recipient: &BACnetRecipient) -> Option<SourceRoute> {
        let here = |network: u16| network == 0 || Some(network) == self.local_network.get();
        let mac = match recipient {
            BACnetRecipient::Device(device) => match self.devices.get(device)? {
                SourceRoute::Direct(mac) => mac,
                SourceRoute::Routed { network, mac } if here(*network) => mac,
                routed => return Some(routed.clone()),
            },
            BACnetRecipient::Address(address) if here(address.network_number) => {
                &address.mac_address
            }
            _ => return None,
        };
        self.valid_address(mac)
            .then(|| SourceRoute::Direct(mac.clone()))
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
    pub(crate) routes: SourceRoutes,
    pub(super) notifications: Weak<NotificationTransactions>,
    pub(super) egress: EndpointEgress,
    pub(super) max_apdu: u16,
    pub(super) failures: Arc<AuditFailureQueue<SourceRoute>>,
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

    /// The route the Device's current Audit_Notification_Recipient resolves
    /// to now, if any.
    pub(crate) fn recipient_route(&self, db: &ObjectDatabase) -> Option<SourceRoute> {
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
                                    route.peer(),
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
                let routers = Arc::clone(self.routes.routers());
                let deadline = tokio::time::Instant::now() + delivery::DEADLINE;
                let first_completion = delivery::Completion::new(Arc::clone(&self.status), token);
                let second_completion = delivery::Completion::new(Arc::clone(&self.status), token);
                let mut attempts = attempts.into_iter();
                let first = attempts.next().unwrap();
                let second = attempts.next().unwrap();
                Ok(async move {
                    let run =
                        |(target, permit, reserved, bytes): (Option<SourceRoute>, _, _, _),
                         completion: delivery::Completion| {
                            let egress = egress.clone();
                            let routers = Arc::clone(&routers);
                            async move {
                                let _permit: tokio::sync::OwnedSemaphorePermit = permit;
                                let (sent, hop) = match target {
                                    Some(route) => delivery::admit_encoded(
                                        &egress, &routers, &route, bytes, confirmed, deadline,
                                    ),
                                    None => (
                                        delivery::admit_global_broadcast(&egress, bytes, deadline),
                                        Hop::none(),
                                    ),
                                };
                                completion.finish(
                                    delivery::finish_send(sent, hop, reserved, deadline).await,
                                );
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
