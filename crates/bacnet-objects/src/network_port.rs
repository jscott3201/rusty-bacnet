//! Configured Network Port snapshots and selected live state (135-2020 Clause 12.56).
//!
//! The B/IP and B/IPv6 constructors supply the flat application-level IPV4
//! and IPV6 profiles (`ip_config`). Configuration is read-only over BACnet:
//! this object has no socket or NIC discovery. Reconstruct it to change
//! local configuration. An explicit B/IP or B/IPv6 owner may publish its
//! actual bind and mode, and learn Network_Number/Quality, without changing
//! configured provenance. In FOREIGN or BBMD mode the object serves that
//! mode's properties, read live from the transport (see `bip_mode`), for
//! either IP version. A BBMD's BDT and registration setting, and a foreign
//! device's BBMD and time-to-live, are writable when the transport can
//! change them, and wait for activation (`pending`), as is a B/IP port's
//! multicast group (`multicast`).
//! Other snapshots expose only the common application rows;
//! they do not claim a complete transport-specific SC, Ethernet or MS/TP profile.

use bacnet_types::enums::{NetworkType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bacnet_types::MacAddr;
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

mod bip6_config;
mod bip_config;
mod bip_mode;
mod ip_config;
mod metadata;
mod multicast;
mod pending;
mod registration;
use bacnet_types::network_number::NetworkNumber;
pub(crate) use bip6_config::Bip6Bind;
pub use bip6_config::Bip6PortConfig;
pub use bip_config::BipPortConfig;
use ip_config::IpConfig;
pub(crate) use ip_config::{PortAddress, PortBind};

/// A declared application-port snapshot, optionally associated with an owned link.
pub struct NetworkPortObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: Reliability,
    network_type: NetworkType,
    network_number: NetworkNumber,
    mac_address: MacAddr,
    apdu_length: u32,
    /// The B/IP or B/IPv6 profile; `None` for the other network types.
    ip: Option<IpConfig>,
    /// The mode the last published owner reported; NORMAL until one
    /// publishes. Like the bind, it stays after that owner stops, until a
    /// new registration reserves the port.
    mode: bip_mode::LiveMode,
    /// Values written to the mode's properties, waiting for activation.
    pending: pending::Pending,
    /// Whether the owner activates written values on ReinitializeDevice;
    /// without it the pending-activation rows refuse writes.
    activation: bool,
    /// The B/IP multicast group the owner lends, and a written one waiting.
    multicast: multicast::Multicast,
    binding: std::sync::Weak<()>,
}

impl NetworkPortObject {
    /// Complete IPV4 snapshot, in NORMAL mode until a registered owner
    /// publishes another. Instance is the declared local Port ID (local
    /// policy: 1..=255); UDP port zero remains valid unbound configuration.
    pub fn new_bip(
        instance: u32,
        name: impl Into<String>,
        config: BipPortConfig,
    ) -> Result<Self, Error> {
        Self::new_ip(instance, name.into(), IpConfig::V4(config))
    }

    /// Complete IPV6 (B/IPv6, Annex U) snapshot, in NORMAL mode until a
    /// registered owner publishes another. The instance policy is B/IP's:
    /// the declared local Port ID, 1..=255. MAC_Address is the configured
    /// VMAC until an owner publishes the one it uses.
    pub fn new_bip6(
        instance: u32,
        name: impl Into<String>,
        config: Bip6PortConfig,
    ) -> Result<Self, Error> {
        Self::new_ip(instance, name.into(), IpConfig::V6(config))
    }

    fn new_ip(instance: u32, name: String, config: IpConfig) -> Result<Self, Error> {
        config.validate(instance)?;
        Ok(Self {
            oid: ObjectIdentifier::new(ObjectType::NETWORK_PORT, instance)?,
            name,
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            network_type: config.network_type(),
            network_number: NetworkNumber::configured(config.network_number())
                .expect("validated Network Number"),
            mac_address: config.mac(),
            apdu_length: config.apdu_length(),
            ip: Some(config),
            mode: Default::default(),
            pending: Default::default(),
            activation: false,
            multicast: Default::default(),
            binding: std::sync::Weak::new(),
        })
    }

    /// Common application rows for Ethernet or VIRTUAL configuration. This is not a
    /// complete link-specific profile. In particular it exposes no IP fields.
    /// IPV4 must use `new_bip` and IPV6 `new_bip6`; the flat B/IP instance
    /// policy does not apply here.
    pub fn new_non_bip(
        instance: u32,
        name: impl Into<String>,
        network_type: NetworkType,
        network_number: u16,
        mac_address: MacAddr,
        apdu_length: u32,
    ) -> Result<Self, Error> {
        if !matches!(network_type, NetworkType::ETHERNET | NetworkType::VIRTUAL)
            || network_number == u16::MAX
            || apdu_length < 50
        {
            return Err(common::value_out_of_range_error());
        }
        Ok(Self {
            oid: ObjectIdentifier::new(ObjectType::NETWORK_PORT, instance)?,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            network_type,
            network_number: NetworkNumber::configured(network_number)
                .expect("validated Network Number"),
            mac_address,
            apdu_length,
            ip: None,
            mode: Default::default(),
            pending: Default::default(),
            activation: false,
            multicast: Default::default(),
            binding: std::sync::Weak::new(),
        })
    }

    /// Set the optional descriptive label without altering port configuration.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for NetworkPortObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        use PropertyIdentifier as P;
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            P::OBJECT_TYPE => Ok(PropertyValue::Enumerated(ObjectType::NETWORK_PORT.to_raw())),
            P::NETWORK_TYPE => Ok(PropertyValue::Enumerated(self.network_type.to_raw())),
            P::PROTOCOL_LEVEL => Ok(PropertyValue::Enumerated(2)),
            P::NETWORK_NUMBER => Ok(PropertyValue::Unsigned(
                self.network_number.snapshot().0.into(),
            )),
            P::NETWORK_NUMBER_QUALITY => Ok(PropertyValue::Enumerated(
                self.network_number.snapshot().1.into(),
            )),
            P::MAC_ADDRESS => Ok(PropertyValue::OctetString(self.mac_address.to_vec())),
            P::APDU_LENGTH => Ok(PropertyValue::Unsigned(self.apdu_length.into())),
            // Optional unknown rate, not a measured NIC speed (errata item 23).
            P::LINK_SPEED => Ok(PropertyValue::Real(0.0)),
            P::CHANGES_PENDING => Ok(PropertyValue::Boolean(self.changes_pending())),
            _ => {
                let config = self
                    .ip
                    .as_ref()
                    .ok_or_else(common::unknown_property_error)?;
                if property == config.mode_property() {
                    return Ok(PropertyValue::Enumerated(self.mode.ip_mode().to_raw()));
                }
                if pending::nat_row(property) && !self.serves_nat() {
                    return Err(common::unknown_property_error());
                }
                if let Some(result) = self.read_multicast(property) {
                    return result;
                }
                config
                    .read(property, array_index)
                    .or_else(|| self.pending_value(property))
                    .or_else(|| self.mode.read(property))
                    .unwrap_or_else(|| Err(common::unknown_property_error()))
            }
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::OUT_OF_SERVICE && self.is_bound() {
            return Err(common::write_access_denied_error());
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) = self.write_pending(property, array_index, &value) {
            return result;
        }
        if let Some(result) = self.write_multicast(property, array_index, &value) {
            return result;
        }
        // Other configuration writes would need an activation owner (12.56)
        // this snapshot doesn't have.
        if self
            .property_metadata()
            .iter()
            .any(|row| row.property_identifier == property)
        {
            Err(common::write_access_denied_error())
        } else {
            Err(common::unknown_property_error())
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }
    fn is_createable(&self) -> bool {
        false
    }
    fn is_deleteable(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod bip6_tests;
#[cfg(test)]
mod foreign_pending_tests;
#[cfg(test)]
mod mode_tests;
#[cfg(test)]
mod multicast_tests;
#[cfg(test)]
mod nat_pending_tests;
#[cfg(test)]
mod pending_tests;
#[cfg(test)]
mod tests;
