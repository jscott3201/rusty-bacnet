//! Trusted single-port attachment; network writes never receive this authority.
use super::*;
use alloc::sync::Arc;
use bacnet_types::bip_port::BipPortMode;

impl NetworkPortObject {
    pub(crate) fn network_number_internal(
        &mut self,
        announcement: Option<(u16, u8)>,
    ) -> NetworkNumber {
        if let Some((number, flag)) = announcement
            && self.network_number.observe(number, flag)
                == bacnet_types::network_number::Observation::ConfiguredConflict
        {
            log_debug!(
                configured = self.network_number.snapshot().0,
                announced = number,
                "local Network Number configuration conflict"
            );
        }
        self.network_number
    }

    /// Borrow the complete configured B/IP snapshot.
    pub(crate) fn configuration_internal(&self) -> Option<&BipPortConfig> {
        match &self.ip {
            Some(IpConfig::V4(config)) => Some(config),
            _ => None,
        }
    }

    /// Borrow the complete configured B/IPv6 snapshot.
    pub(crate) fn bip6_configuration_internal(&self) -> Option<&Bip6PortConfig> {
        match &self.ip {
            Some(IpConfig::V6(config)) => Some(config),
            _ => None,
        }
    }

    /// The configured address and UDP port, B/IP or B/IPv6.
    pub(crate) fn configured_address_internal(&self) -> Option<PortAddress> {
        self.ip.as_ref().map(IpConfig::address)
    }

    /// Reserve the built-in object for a live owner. Until the owner
    /// publishes, the port reads as NORMAL with its configured number.
    ///
    /// Refused, with one text, by why the port can't take it (#1833): a
    /// port with no IP configuration is no B/IP port (`Unsupported`); one
    /// another owner holds is a one-holder resource taken (`LimitReached`);
    /// one the application left Out_Of_Service is refused as the database
    /// it started the owner with (`InvalidConfig`).
    pub(crate) fn reserve_internal(&mut self, lease: &Arc<()>) -> Result<(), Error> {
        const UNAVAILABLE: &str = "Network Port is unavailable for B/IP registration";
        let Some(config) = self.ip.as_ref() else {
            return Err(Error::unsupported(UNAVAILABLE));
        };
        if self.is_bound() {
            return Err(Error::limit_reached(UNAVAILABLE));
        }
        if self.out_of_service {
            return Err(Error::invalid_config(UNAVAILABLE));
        }
        self.network_number =
            NetworkNumber::configured(config.network_number()).expect("validated Network Number");
        self.mode = bip_mode::LiveMode::Normal;
        self.pending = Default::default();
        self.multicast = Default::default();
        self.activation = false;
        self.binding = Arc::downgrade(lease);
        Ok(())
    }

    /// What a publication replaces, as the reservation leaves it.
    pub(crate) fn reserved_internal(&self) -> Reserved {
        Reserved {
            ip: self.ip.clone(),
            mac_address: self.mac_address.clone(),
            apdu_length: self.apdu_length,
        }
    }

    /// Put the port back as its reservation left it, after an owner that
    /// published went without starting (#2752), so the next reservation
    /// matches the configured address and UDP port again.
    pub(crate) fn withdraw_internal(&mut self, reserved: Reserved) {
        if let Some(config) = &reserved.ip {
            self.network_number = NetworkNumber::configured(config.network_number())
                .expect("validated Network Number");
        }
        self.ip = reserved.ip;
        self.mac_address = reserved.mac_address;
        self.apdu_length = reserved.apdu_length;
        self.mode = bip_mode::LiveMode::Normal;
        self.pending = Default::default();
        self.multicast = Default::default();
        self.activation = false;
    }

    /// Publish the matching owner's actual bind, supported capacity and
    /// B/IP or B/IPv6 mode. A BBMD's tables stay the owner's: the object
    /// reads them live.
    pub(crate) fn reconcile_internal(
        &mut self,
        lease: &Arc<()>,
        bind: PortBind,
        capacity: u32,
        mode: BipPortMode,
    ) -> Result<(), Error> {
        if !self
            .binding
            .upgrade()
            .is_some_and(|current| Arc::ptr_eq(&current, lease))
        {
            return Err(Error::invalid_argument(
                "Network Port registration owner mismatch",
            ));
        }
        let config = self
            .ip
            .as_ref()
            .ok_or_else(|| Error::unsupported("not B/IP"))?
            .with_bind(bind, capacity)?;
        config.validate(self.oid.instance_number())?;
        let mode = bip_mode::LiveMode::from_port(mode)?;
        self.network_number =
            NetworkNumber::configured(config.network_number()).expect("validated Network Number");
        self.apdu_length = capacity;
        self.mac_address = config.mac();
        self.ip = Some(config);
        self.mode = mode;
        self.pending = Default::default();
        self.multicast.clear_pending();
        Ok(())
    }

    /// Lend the B/IP multicast group of the owner that published, or none
    /// for a transport that can't use multicast. Any written group is
    /// dropped.
    pub(crate) fn set_multicast_internal(
        &mut self,
        view: Option<Arc<dyn bacnet_types::bip_port::IpMulticast>>,
    ) {
        self.multicast = super::multicast::Multicast::lent(view);
    }
}

impl NetworkPortObject {
    /// Say whether this port's owner activates written values: it answers
    /// ReinitializeDevice ACTIVATE_CHANGES and WARMSTART.
    pub(crate) fn set_activation_internal(&mut self, activation: bool) {
        self.activation = activation;
    }

    pub(super) fn is_bound(&self) -> bool {
        self.binding.strong_count() != 0
    }
}

/// The configured bind, MAC address and APDU length a reserved port held
/// before its owner published, for [`NetworkPortObject::withdraw_internal`].
pub(crate) struct Reserved {
    ip: Option<IpConfig>,
    mac_address: MacAddr,
    apdu_length: u32,
}
