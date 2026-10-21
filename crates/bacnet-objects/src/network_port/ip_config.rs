//! The IP profile of a B/IP or B/IPv6 Network Port, and the rows each one
//! adds (Table 12-71, and the informative Table 12-72).
//!
//! Everything else the two share: the mode (BACnet_IP_Mode or
//! BACnet_IPv6_Mode, read from the same published mode), the BBMD and
//! foreign-device rows (`bip_mode`), the values waiting for activation
//! (`pending`), and registration (`registration`).

use bacnet_types::enums::{NetworkType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bacnet_types::MacAddr;

use super::{Bip6Bind, Bip6PortConfig, BipPortConfig};
use crate::common;

/// A port's configured IP profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum IpConfig {
    /// B/IP (Annex J).
    V4(BipPortConfig),
    /// B/IPv6 (Annex U).
    V6(Bip6PortConfig),
}

/// The address and UDP port a configuration declares, or a transport is
/// bound to, as registration compares them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PortAddress {
    /// An IPv4 address and UDP port.
    V4([u8; 4], u16),
    /// An IPv6 address and UDP port.
    V6([u8; 16], u16),
}

impl PortAddress {
    /// The UDP port.
    pub(crate) fn udp(self) -> u16 {
        match self {
            Self::V4(_, udp) | Self::V6(_, udp) => udp,
        }
    }

    /// Whether `self` and `other` are of one IP version.
    pub(crate) fn same_family(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::V4(..), Self::V4(..)) | (Self::V6(..), Self::V6(..))
        )
    }

    /// Whether `self` names the same address as `other`, any port.
    pub(crate) fn same_ip(self, other: Self) -> bool {
        match (self, other) {
            (Self::V4(a, _), Self::V4(b, _)) => a == b,
            (Self::V6(a, _), Self::V6(b, _)) => a == b,
            _ => false,
        }
    }
}

/// What an owner publishes about its bind, for either family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PortBind {
    /// A B/IP bind: the address and UDP port.
    V4([u8; 4], u16),
    /// A B/IPv6 bind.
    V6(Bip6Bind),
}

impl PortBind {
    pub(crate) fn address(self) -> PortAddress {
        match self {
            Self::V4(ip, udp) => PortAddress::V4(ip, udp),
            Self::V6(bind) => PortAddress::V6(bind.ip, bind.udp),
        }
    }
}

impl IpConfig {
    pub(super) fn network_type(&self) -> NetworkType {
        match self {
            Self::V4(_) => NetworkType::IPV4,
            Self::V6(_) => NetworkType::IPV6,
        }
    }

    pub(super) fn network_number(&self) -> u16 {
        match self {
            Self::V4(config) => config.network_number,
            Self::V6(config) => config.network_number,
        }
    }

    pub(super) fn apdu_length(&self) -> u32 {
        match self {
            Self::V4(config) => config.apdu_length,
            Self::V6(config) => config.apdu_length,
        }
    }

    pub(super) fn validate(&self, instance: u32) -> Result<(), Error> {
        match self {
            Self::V4(config) => config.validate(instance),
            Self::V6(config) => config.validate(instance),
        }
    }

    /// MAC_Address (Clause 12.56.15): a B/IP port's address and UDP port,
    /// a B/IPv6 port's VMAC.
    pub(super) fn mac(&self) -> MacAddr {
        match self {
            Self::V4(config) => {
                let mut mac = MacAddr::from_slice(&config.ip_address);
                mac.extend_from_slice(&config.udp_port.to_be_bytes());
                mac
            }
            Self::V6(config) => MacAddr::from_slice(&config.vmac),
        }
    }

    pub(super) fn address(&self) -> PortAddress {
        match self {
            Self::V4(config) => PortAddress::V4(config.ip_address, config.udp_port),
            Self::V6(config) => PortAddress::V6(config.ip_address, config.udp_port),
        }
    }

    /// This configuration with an owner's actual bind and capacity, or an
    /// error when the bind is for the other family.
    pub(super) fn with_bind(&self, bind: PortBind, capacity: u32) -> Result<Self, Error> {
        Ok(match (self, bind) {
            (Self::V4(config), PortBind::V4(ip, udp)) => Self::V4(BipPortConfig {
                ip_address: ip,
                udp_port: udp,
                apdu_length: capacity,
                ..config.clone()
            }),
            (Self::V6(config), PortBind::V6(bind)) => Self::V6(Bip6PortConfig {
                ip_address: bind.ip,
                udp_port: bind.udp,
                vmac: bind.vmac,
                multicast_address: bind.multicast,
                apdu_length: capacity,
                ..config.clone()
            }),
            _ => {
                return Err(Error::Encoding(
                    "Network Port bind is for the other IP version".into(),
                ))
            }
        })
    }

    /// Whether this is an IPv4 profile, the only one with NAT rows (Table
    /// 12-71, footnotes 9 and 10).
    pub(super) fn is_ipv4(&self) -> bool {
        matches!(self, Self::V4(_))
    }

    /// The property naming the port's mode: BACnet_IP_Mode or
    /// BACnet_IPv6_Mode (Clause 12.56.21, 12.56.39).
    pub(super) fn mode_property(&self) -> P {
        match self {
            Self::V4(_) => P::BACNET_IP_MODE,
            Self::V6(_) => P::BACNET_IPV6_MODE,
        }
    }

    /// Read a row this profile adds, other than the mode; `None` for any
    /// other property.
    pub(super) fn read(
        &self,
        property: P,
        array_index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        let octets = |value: &[u8]| Some(Ok(PropertyValue::OctetString(value.to_vec())));
        match self {
            Self::V4(config) => match property {
                P::IP_ADDRESS => octets(&config.ip_address),
                P::IP_SUBNET_MASK => octets(&config.subnet_mask),
                P::IP_DEFAULT_GATEWAY => octets(&config.default_gateway),
                P::BACNET_IP_UDP_PORT => Some(Ok(PropertyValue::Unsigned(config.udp_port.into()))),
                P::IP_DNS_SERVER => Some(address_array(&config.dns_servers, array_index)),
                _ => None,
            },
            Self::V6(config) => match property {
                P::IPV6_ADDRESS => octets(&config.ip_address),
                P::IPV6_PREFIX_LENGTH => {
                    Some(Ok(PropertyValue::Unsigned(config.prefix_length.into())))
                }
                P::BACNET_IPV6_UDP_PORT => {
                    Some(Ok(PropertyValue::Unsigned(config.udp_port.into())))
                }
                P::IPV6_DEFAULT_GATEWAY => octets(&config.default_gateway),
                P::BACNET_IPV6_MULTICAST_ADDRESS => octets(&config.multicast_with_port()),
                P::IPV6_DNS_SERVER => Some(address_array(&config.dns_servers, array_index)),
                _ => None,
            },
        }
    }
}

/// A BACnetARRAY of addresses, each an OCTET STRING: IP_DNS_Server or
/// IPv6_DNS_Server.
fn address_array<const N: usize>(
    addresses: &[[u8; N]],
    array_index: Option<u32>,
) -> Result<PropertyValue, Error> {
    common::read_array(
        addresses
            .iter()
            .map(|address| PropertyValue::OctetString(address.to_vec()))
            .collect(),
        array_index,
    )
}
