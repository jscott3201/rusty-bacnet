//! The B/IPv6 client builder: [`BACnetClient::bip6_builder`] and
//! [`Bip6ClientBuilder`].
use super::*;

#[cfg(feature = "ipv6")]
impl BACnetClient<Bip6Transport> {
    /// Create a BIP6-specific builder for BACnet/IPv6 transport.
    #[cfg_attr(docsrs, doc(cfg(feature = "ipv6")))]
    pub fn bip6_builder() -> Bip6ClientBuilder {
        Bip6ClientBuilder {
            config: ClientConfig::default(),
            options: ClientOptions::default(),
            interface: Ipv6Addr::UNSPECIFIED,
            device_instance: None,
            foreign_device: None,
        }
    }
}

/// BIP6-specific builder that constructs `Bip6Transport` from IPv6 interface/port/device-instance.
#[cfg(feature = "ipv6")]
#[cfg_attr(docsrs, doc(cfg(feature = "ipv6")))]
pub struct Bip6ClientBuilder {
    pub(super) config: ClientConfig,
    pub(super) options: ClientOptions,
    pub(super) interface: Ipv6Addr,
    pub(super) device_instance: Option<u32>,
    pub(super) foreign_device: Option<Bip6ForeignDeviceConfig>,
}

#[cfg(feature = "ipv6")]
impl Bip6ClientBuilder {
    /// Set the local IPv6 interface address.
    pub fn interface(mut self, ip: Ipv6Addr) -> Self {
        self.interface = ip;
        self
    }

    /// Set the UDP port (0 for ephemeral).
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Set the device instance for VMAC derivation (Annex U.5).
    pub fn device_instance(mut self, instance: u32) -> Self {
        self.device_instance = Some(instance);
        self
    }

    /// Set APDU timeout in milliseconds.
    pub fn apdu_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_timeout_ms = ms;
        self
    }

    /// Set the maximum APDU length this client accepts.
    pub fn max_apdu_length(mut self, len: u16) -> Self {
        self.config.max_apdu_length = len;
        self
    }

    /// Set the COV notification broadcast channel capacity.
    pub fn cov_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.cov_channel_capacity = capacity;
        self
    }
}
