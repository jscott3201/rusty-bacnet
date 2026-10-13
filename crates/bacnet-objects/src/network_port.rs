//! Configured Network Port snapshots and selected live state (135-2020 Clause 12.56).
//!
//! The B/IP constructor supplies the flat application-level IPV4 profile.
//! Configuration is read-only over BACnet: this object has no pending
//! activation owner, socket, or NIC discovery. Reconstruct it to change local
//! configuration. An explicit B/IP owner may publish its actual bind and mode,
//! and learn Network_Number/Quality, without changing configured provenance.
//! In FOREIGN or BBMD mode the object serves that mode's properties, a
//! BBMD's tables read live from the transport (see `bip_mode`).
//! Non-B/IP snapshots expose only the common application rows;
//! they do not claim a complete transport-specific SC, Ethernet or MS/TP profile.

use bacnet_types::enums::{NetworkType, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bacnet_types::MacAddr;
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

mod bip_config;
mod bip_mode;
mod metadata;
mod registration;
use bacnet_types::network_number::NetworkNumber;
pub use bip_config::BipPortConfig;

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
    bip: Option<BipPortConfig>,
    /// The mode the last published owner reported; NORMAL until one
    /// publishes. Like the bind, it stays after that owner stops, until a
    /// new registration reserves the port.
    mode: bip_mode::LiveMode,
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
        config.validate(instance)?;
        let mut mac = MacAddr::from_slice(&config.ip_address);
        mac.extend_from_slice(&config.udp_port.to_be_bytes());
        Ok(Self {
            oid: ObjectIdentifier::new(ObjectType::NETWORK_PORT, instance)?,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            network_type: NetworkType::IPV4,
            network_number: NetworkNumber::configured(config.network_number)
                .expect("validated Network Number"),
            mac_address: mac,
            apdu_length: config.apdu_length,
            bip: Some(config),
            mode: Default::default(),
            binding: std::sync::Weak::new(),
        })
    }

    /// Common application rows for Ethernet or VIRTUAL configuration. This is not a
    /// complete link-specific profile. In particular it exposes no IPv4 fields.
    /// IPV4 must use `new_bip`; the flat B/IP instance policy does not apply here.
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
            bip: None,
            mode: Default::default(),
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
            P::CHANGES_PENDING => Ok(PropertyValue::Boolean(false)),
            _ => {
                let config = self
                    .bip
                    .as_ref()
                    .ok_or_else(common::unknown_property_error)?;
                match property {
                    P::BACNET_IP_MODE => {
                        Ok(PropertyValue::Enumerated(self.mode.ip_mode().to_raw()))
                    }
                    P::IP_ADDRESS => Ok(PropertyValue::OctetString(config.ip_address.to_vec())),
                    P::IP_SUBNET_MASK => {
                        Ok(PropertyValue::OctetString(config.subnet_mask.to_vec()))
                    }
                    P::IP_DEFAULT_GATEWAY => {
                        Ok(PropertyValue::OctetString(config.default_gateway.to_vec()))
                    }
                    P::BACNET_IP_UDP_PORT => Ok(PropertyValue::Unsigned(config.udp_port.into())),
                    P::IP_DNS_SERVER => match array_index {
                        None => Ok(PropertyValue::List(
                            config
                                .dns_servers
                                .iter()
                                .map(|ip| PropertyValue::OctetString(ip.to_vec()))
                                .collect(),
                        )),
                        Some(0) => Ok(PropertyValue::Unsigned(config.dns_servers.len() as u64)),
                        Some(index) => config
                            .dns_servers
                            .get(index as usize - 1)
                            .map(|ip| PropertyValue::OctetString(ip.to_vec()))
                            .ok_or_else(common::invalid_array_index_error),
                    },
                    _ => self
                        .mode
                        .read(property)
                        .unwrap_or_else(|| Err(common::unknown_property_error())),
                }
            }
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
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
        // Configuration writes require actual activation (12.56); this snapshot
        // deliberately provides no apply/Command path or inert pending state.
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
mod mode_tests;
#[cfg(test)]
mod tests;
