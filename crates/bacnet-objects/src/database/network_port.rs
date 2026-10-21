//! Explicit receiving-port reservation and publication for one owned B/IP
//! or B/IPv6 link. Both IP versions share one registration path; only the
//! address types differ.
use super::*;
use crate::network_port::{
    Bip6Bind, Bip6PortConfig, BipPortConfig, NetworkPortObject, PortAddress, PortBind,
};
use bacnet_types::bip_port::BipPortMode;
use std::sync::Weak;

pub(super) struct NetworkPortRegistration {
    oid: ObjectIdentifier,
    lease: Weak<()>,
    /// The configured address and UDP port the reservation checked.
    configured: PortAddress,
    published: bool,
}

impl ObjectDatabase {
    fn builtin_network_port_mut(
        &mut self,
        oid: &ObjectIdentifier,
    ) -> Option<&mut NetworkPortObject> {
        // Reflect the stored dyn object, never the Box and never an adapter hook.
        self.objects
            .get_mut(oid)?
            .as_mut()
            .as_stored_any_mut(crate::traits::ObjectStorageAccess(()))
            .downcast_mut::<NetworkPortObject>()
            .filter(|port| port.object_identifier() == *oid)
    }

    /// Snapshot only the concrete built-in object; adapters cannot impersonate it.
    #[doc(hidden)]
    pub fn configured_bip_port_internal(
        &mut self,
        oid: &ObjectIdentifier,
    ) -> Option<BipPortConfig> {
        self.builtin_network_port_mut(oid)?
            .configuration_internal()
            .cloned()
    }

    /// Snapshot only the concrete built-in B/IPv6 object.
    fn configured_bip6_port_internal(&mut self, oid: &ObjectIdentifier) -> Option<Bip6PortConfig> {
        self.builtin_network_port_mut(oid)?
            .bip6_configuration_internal()
            .cloned()
    }

    /// Reserve exactly one concrete built-in B/IP object before bind, in any
    /// B/IP mode. The returned token must move into the transport before it
    /// can start.
    #[doc(hidden)]
    pub fn reserve_bip_port_internal(
        &mut self,
        oid: ObjectIdentifier,
        ip: [u8; 4],
        udp: u16,
    ) -> Result<(BipPortConfig, Arc<()>), Error> {
        let lease = self.reserve_port(oid, PortAddress::V4(ip, udp))?;
        let configured = self
            .configured_bip_port_internal(&oid)
            .expect("reserved B/IP port");
        Ok((configured, lease))
    }

    /// Reserve exactly one concrete built-in B/IPv6 object before bind, in
    /// any mode, as [`reserve_bip_port_internal`](Self::reserve_bip_port_internal)
    /// does for B/IP.
    #[doc(hidden)]
    pub fn reserve_bip6_port_internal(
        &mut self,
        oid: ObjectIdentifier,
        ip: [u8; 16],
        udp: u16,
    ) -> Result<(Bip6PortConfig, Arc<()>), Error> {
        let lease = self.reserve_port(oid, PortAddress::V6(ip, udp))?;
        let configured = self
            .configured_bip6_port_internal(&oid)
            .expect("reserved B/IPv6 port");
        Ok((configured, lease))
    }

    fn reserve_port(
        &mut self,
        oid: ObjectIdentifier,
        address: PortAddress,
    ) -> Result<Arc<()>, Error> {
        if oid.object_type() != ObjectType::NETWORK_PORT
            || !(1..=255).contains(&oid.instance_number())
        {
            return Err(invalid("registered Network Port instance must be 1..255"));
        }
        if self
            .network_port
            .as_ref()
            .is_some_and(|port| port.lease.strong_count() != 0)
        {
            return Err(invalid("database already has a live registered port owner"));
        }
        let authority = self
            .builtin_network_port_mut(&oid)
            .ok_or_else(|| invalid("registration requires the selected built-in Network Port"))?;
        let configured = match authority.configured_address_internal() {
            Some(configured) if configured == address => configured,
            Some(configured) if configured.same_family(address) => {
                return Err(invalid(
                    "Network Port configuration differs from transport interface/UDP",
                ))
            }
            _ => {
                return Err(invalid(match address {
                    PortAddress::V4(..) => "registration requires an IPV4 port",
                    PortAddress::V6(..) => "registration requires an IPV6 port",
                }))
            }
        };
        let lease = Arc::new(());
        authority.reserve_internal(&lease)?;
        self.network_port = Some(NetworkPortRegistration {
            oid,
            lease: Arc::downgrade(&lease),
            configured,
            published: false,
        });
        Ok(lease)
    }

    /// Reconcile one actual bind and its B/IP mode under the database write
    /// lock before publication. In BBMD mode `mode` must carry the started
    /// transport's tables, which the object then reads live.
    #[doc(hidden)]
    pub fn publish_bip_port_internal(
        &mut self,
        oid: ObjectIdentifier,
        ip: [u8; 4],
        udp: u16,
        capacity: u32,
        mode: BipPortMode,
    ) -> Result<(), Error> {
        self.publish_port(oid, PortBind::V4(ip, udp), capacity, mode)
    }

    /// Reconcile one actual B/IPv6 bind (address, UDP port), the VMAC it
    /// uses, the multicast group it broadcasts to and its mode, as
    /// [`publish_bip_port_internal`](Self::publish_bip_port_internal) does
    /// for B/IP.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)] // one bind, as the B/IP call takes it
    pub fn publish_bip6_port_internal(
        &mut self,
        oid: ObjectIdentifier,
        ip: [u8; 16],
        udp: u16,
        vmac: [u8; 3],
        multicast: [u8; 16],
        capacity: u32,
        mode: BipPortMode,
    ) -> Result<(), Error> {
        let bind = Bip6Bind {
            ip,
            udp,
            vmac,
            multicast,
        };
        self.publish_port(oid, PortBind::V6(bind), capacity, mode)
    }

    fn publish_port(
        &mut self,
        oid: ObjectIdentifier,
        bind: PortBind,
        capacity: u32,
        mode: BipPortMode,
    ) -> Result<(), Error> {
        let registration = self
            .network_port
            .as_ref()
            .filter(|port| port.oid == oid)
            .ok_or_else(|| invalid("missing Network Port reservation"))?;
        let lease = registration
            .lease
            .upgrade()
            .ok_or_else(|| invalid("Network Port owner expired"))?;
        let actual = bind.address();
        let configured = registration.configured;
        if registration.published
            || !configured.same_ip(actual)
            || actual.udp() == 0
            || (configured.udp() != 0 && configured.udp() != actual.udp())
        {
            return Err(invalid("post-bind Network Port identity mismatch"));
        }
        self.builtin_network_port_mut(&oid)
            .ok_or_else(|| invalid("selected Network Port authority disappeared"))?
            .reconcile_internal(&lease, bind, capacity, mode)?;
        self.network_port
            .as_mut()
            .expect("validated registration")
            .published = true;
        Ok(())
    }

    /// Explicit published receiving port, never a selection by database order.
    #[doc(hidden)]
    pub fn registered_port_internal(&self) -> Option<ObjectIdentifier> {
        self.network_port
            .as_ref()
            .filter(|port| port.published && port.lease.strong_count() != 0)
            .map(|port| port.oid)
    }

    /// Read or learn through the explicitly published receiving owner only.
    /// Callers hold the database write lock, so RP/RPM observe coherent pairs.
    #[doc(hidden)]
    pub fn network_number_internal(
        &mut self,
        oid: ObjectIdentifier,
        announcement: Option<(u16, u8)>,
    ) -> Option<bacnet_types::network_number::NetworkNumber> {
        if self.registered_port_internal() != Some(oid) {
            return None;
        }
        Some(
            self.builtin_network_port_mut(&oid)?
                .network_number_internal(announcement),
        )
    }

    /// Say whether the registered port's owner activates written values,
    /// answering ReinitializeDevice ACTIVATE_CHANGES and WARMSTART. Until it
    /// does, the pending-activation rows refuse writes.
    #[doc(hidden)]
    pub fn set_network_port_activation_internal(
        &mut self,
        oid: ObjectIdentifier,
        activation: bool,
    ) -> Result<(), Error> {
        self.builtin_network_port_mut(&oid)
            .ok_or_else(|| invalid("selected Network Port authority disappeared"))?
            .set_activation_internal(activation);
        Ok(())
    }

    /// Lend the registered B/IP port the multicast group of its owner, as
    /// the owner reports it after start ([`IpMulticast`]), or none for an
    /// owner that can't use multicast. The port then serves
    /// BACnet_IP_Multicast_Address (#941).
    ///
    /// [`IpMulticast`]: bacnet_types::bip_port::IpMulticast
    #[doc(hidden)]
    pub fn set_network_port_multicast_internal(
        &mut self,
        oid: ObjectIdentifier,
        multicast: Option<std::sync::Arc<dyn bacnet_types::bip_port::IpMulticast>>,
    ) -> Result<(), Error> {
        self.builtin_network_port_mut(&oid)
            .ok_or_else(|| invalid("selected Network Port authority disappeared"))?
            .set_multicast_internal(multicast);
        Ok(())
    }

    /// The Network Port objects, built-in ones only.
    fn network_port_oids(&self) -> Vec<ObjectIdentifier> {
        self.objects
            .keys()
            .filter(|oid| oid.object_type() == ObjectType::NETWORK_PORT)
            .copied()
            .collect()
    }

    /// Check, changing nothing, that every built-in Network Port's pending
    /// values would still be taken: what a ReinitializeDevice ACTIVATE_CHANGES
    /// or WARMSTART checks before its handler runs.
    pub fn check_network_port_changes(&mut self) -> Result<(), Error> {
        for oid in self.network_port_oids() {
            if let Some(port) = self.builtin_network_port_mut(&oid) {
                port.check_pending_changes()?;
            }
        }
        Ok(())
    }

    /// Put the values written to every built-in Network Port into effect and
    /// clear its Changes_Pending, as a ReinitializeDevice ACTIVATE_CHANGES
    /// or WARMSTART does (Clause 12.56.13). How many ports had changes.
    ///
    /// Each port's changes go into effect all or none, but not across
    /// ports: a port whose transport refuses its changes keeps them pending
    /// while the others take theirs, and the first such error is returned
    /// once every port has been tried.
    pub fn activate_network_port_changes(&mut self) -> Result<usize, Error> {
        let ports = self.network_port_oids();
        let mut activated = 0;
        let mut first_error = None;
        for oid in ports {
            let Some(port) = self.builtin_network_port_mut(&oid) else {
                continue;
            };
            match port.activate_changes() {
                Ok(true) => activated += 1,
                Ok(false) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(activated), Err)
    }

    pub(super) fn check_network_port_membership(
        &self,
        oid: &ObjectIdentifier,
    ) -> Result<(), Error> {
        if self
            .network_port
            .as_ref()
            .is_some_and(|port| port.oid == *oid && port.lease.strong_count() != 0)
        {
            return Err(Error::Protocol {
                class: ErrorClass::OBJECT,
                code: ErrorCode::OBJECT_DELETION_NOT_PERMITTED,
            });
        }
        Ok(())
    }
}
fn invalid(message: &str) -> Error {
    Error::Encoding(message.into())
}
