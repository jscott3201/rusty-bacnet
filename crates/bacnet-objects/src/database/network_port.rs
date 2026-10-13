//! Explicit receiving-port reservation and publication for one owned B/IP link.
use super::*;
use crate::network_port::{BipPortConfig, NetworkPortObject};
use bacnet_types::bip_port::BipPortMode;
use std::sync::Weak;

pub(super) struct NetworkPortRegistration {
    oid: ObjectIdentifier,
    lease: Weak<()>,
    configured: BipPortConfig,
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
        let configured = authority
            .configuration_internal()
            .cloned()
            .ok_or_else(|| invalid("registration requires an IPV4 port"))?;
        if configured.ip_address != ip || configured.udp_port != udp {
            return Err(invalid(
                "Network Port configuration differs from transport interface/UDP",
            ));
        }
        let lease = Arc::new(());
        authority.reserve_internal(&lease)?;
        self.network_port = Some(NetworkPortRegistration {
            oid,
            lease: Arc::downgrade(&lease),
            configured: configured.clone(),
            published: false,
        });
        Ok((configured, lease))
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
        let registration = self
            .network_port
            .as_ref()
            .filter(|port| port.oid == oid)
            .ok_or_else(|| invalid("missing Network Port reservation"))?;
        let lease = registration
            .lease
            .upgrade()
            .ok_or_else(|| invalid("Network Port owner expired"))?;
        if registration.published
            || registration.configured.ip_address != ip
            || udp == 0
            || (registration.configured.udp_port != 0 && registration.configured.udp_port != udp)
        {
            return Err(invalid("post-bind Network Port identity mismatch"));
        }
        self.builtin_network_port_mut(&oid)
            .ok_or_else(|| invalid("selected Network Port authority disappeared"))?
            .reconcile_internal(&lease, ip, udp, capacity, mode)?;
        self.network_port
            .as_mut()
            .expect("validated registration")
            .published = true;
        Ok(())
    }

    /// Explicit published receiving port, never a selection by database order.
    #[doc(hidden)]
    pub fn registered_bip_port_internal(&self) -> Option<ObjectIdentifier> {
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
        if self.registered_bip_port_internal() != Some(oid) {
            return None;
        }
        Some(
            self.builtin_network_port_mut(&oid)?
                .network_number_internal(announcement),
        )
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
                class: ErrorClass::OBJECT.to_raw().into(),
                code: ErrorCode::OBJECT_DELETION_NOT_PERMITTED.to_raw().into(),
            });
        }
        Ok(())
    }
}
fn invalid(message: &str) -> Error {
    Error::Encoding(message.into())
}
