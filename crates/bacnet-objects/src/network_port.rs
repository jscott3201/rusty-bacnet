//! NetworkPort object (type 56) per ASHRAE 135-2020 Clause 12.56.
//!
//! Represents a physical or virtual network port on a BACnet device,
//! exposing network configuration (IP address, subnet, gateway, etc.)
//! and link status information.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bacnet_types::MacAddr;
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet Network Port object.
///
/// Models a network interface on the device. Key properties include
/// the network type (IPv4, IPv6, MS/TP, etc.), link speed, MAC address,
/// and IP configuration parameters.
pub struct NetworkPortObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
    /// Network type: 0=IPv4, 1=IPv6, 2=MSTP, etc.
    network_type: u32,
    /// The BACnet network number this port is connected to.
    network_number: u32,
    /// MAC address of this port.
    mac_address: MacAddr,
    /// Maximum APDU length accepted on this port.
    max_apdu_length_accepted: u32,
    /// Link speed in bits per second.
    link_speed: f32,
    /// Whether uncommitted configuration changes are pending.
    changes_pending: bool,
    /// NetworkPortCommand: 0=idle, 1=discardChanges, 2=renewFdRegistration, etc.
    command: u32,
    /// IP address (4 bytes for IPv4).
    ip_address: Vec<u8>,
    /// Default gateway IP address.
    ip_default_gateway: Vec<u8>,
    /// Subnet mask.
    ip_subnet_mask: Vec<u8>,
    /// BACnet/IP UDP port number.
    ip_udp_port: u16,
}

impl NetworkPortObject {
    /// Create a new Network Port object.
    ///
    /// `network_type` specifies the port type: 0=IPv4, 1=IPv6, 2=MSTP, etc.
    pub fn new(instance: u32, name: impl Into<String>, network_type: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
            network_type,
            network_number: 0,
            mac_address: MacAddr::new(),
            max_apdu_length_accepted: 1476,
            link_speed: 0.0,
            changes_pending: false,
            command: 0,
            ip_address: vec![0, 0, 0, 0],
            ip_default_gateway: vec![0, 0, 0, 0],
            ip_subnet_mask: vec![255, 255, 255, 0],
            ip_udp_port: 0xBAC0,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the IP address (4 bytes for IPv4).
    pub fn set_ip_address(&mut self, addr: Vec<u8>) {
        self.ip_address = addr;
    }

    /// Set the default gateway IP address.
    pub fn set_ip_default_gateway(&mut self, gw: Vec<u8>) {
        self.ip_default_gateway = gw;
    }

    /// Set the subnet mask.
    pub fn set_ip_subnet_mask(&mut self, mask: Vec<u8>) {
        self.ip_subnet_mask = mask;
    }

    /// Set the MAC address.
    pub fn set_mac_address(&mut self, mac: MacAddr) {
        self.mac_address = mac;
    }

    /// Set the network number.
    pub fn set_network_number(&mut self, num: u32) {
        self.network_number = num;
    }

    /// Set the link speed in bits per second.
    pub fn set_link_speed(&mut self, speed: f32) {
        self.link_speed = speed;
    }

    /// Set the BACnet/IP UDP port.
    pub fn set_udp_port(&mut self, port: u16) {
        self.ip_udp_port = port;
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::NETWORK_PORT.to_raw()))
            }
            p if p == PropertyIdentifier::NETWORK_TYPE => {
                Ok(PropertyValue::Enumerated(self.network_type))
            }
            p if p == PropertyIdentifier::NETWORK_NUMBER => {
                Ok(PropertyValue::Unsigned(self.network_number as u64))
            }
            p if p == PropertyIdentifier::MAC_ADDRESS => {
                Ok(PropertyValue::OctetString(self.mac_address.to_vec()))
            }
            p if p == PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED => Ok(PropertyValue::Unsigned(
                self.max_apdu_length_accepted as u64,
            )),
            p if p == PropertyIdentifier::LINK_SPEED => Ok(PropertyValue::Real(self.link_speed)),
            p if p == PropertyIdentifier::CHANGES_PENDING => {
                Ok(PropertyValue::Boolean(self.changes_pending))
            }
            p if p == PropertyIdentifier::COMMAND_NP => Ok(PropertyValue::Enumerated(self.command)),
            p if p == PropertyIdentifier::IP_ADDRESS => {
                Ok(PropertyValue::OctetString(self.ip_address.clone()))
            }
            p if p == PropertyIdentifier::IP_DEFAULT_GATEWAY => {
                Ok(PropertyValue::OctetString(self.ip_default_gateway.clone()))
            }
            p if p == PropertyIdentifier::IP_SUBNET_MASK => {
                Ok(PropertyValue::OctetString(self.ip_subnet_mask.clone()))
            }
            p if p == PropertyIdentifier::BACNET_IP_UDP_PORT => {
                Ok(PropertyValue::Unsigned(self.ip_udp_port as u64))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if property == PropertyIdentifier::COMMAND_NP {
            if let PropertyValue::Enumerated(v) = value {
                self.command = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::IP_ADDRESS {
            if let PropertyValue::OctetString(v) = value {
                self.ip_address = v;
                self.changes_pending = true;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::IP_DEFAULT_GATEWAY {
            if let PropertyValue::OctetString(v) = value {
                self.ip_default_gateway = v;
                self.changes_pending = true;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::IP_SUBNET_MASK {
            if let PropertyValue::OctetString(v) = value {
                self.ip_subnet_mask = v;
                self.changes_pending = true;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::BACNET_IP_UDP_PORT {
            if let PropertyValue::Unsigned(v) = value {
                if v > u16::MAX as u64 {
                    return Err(common::value_out_of_range_error());
                }
                self.ip_udp_port = v as u16;
                self.changes_pending = true;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::NETWORK_NUMBER {
            if let PropertyValue::Unsigned(v) = value {
                self.network_number = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        if property == PropertyIdentifier::MAC_ADDRESS {
            if let PropertyValue::OctetString(v) = value {
                self.mac_address = v.into();
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::NETWORK_TYPE,
            PropertyIdentifier::NETWORK_NUMBER,
            PropertyIdentifier::MAC_ADDRESS,
            PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
            PropertyIdentifier::LINK_SPEED,
            PropertyIdentifier::CHANGES_PENDING,
            PropertyIdentifier::COMMAND_NP,
            PropertyIdentifier::IP_ADDRESS,
            PropertyIdentifier::IP_DEFAULT_GATEWAY,
            PropertyIdentifier::IP_SUBNET_MASK,
            PropertyIdentifier::BACNET_IP_UDP_PORT,
        ];
        Cow::Borrowed(PROPS)
    }

    /// NetworkPort is not createable or deleteable at runtime.
    fn is_createable(&self) -> bool {
        false
    }
    fn is_deleteable(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests;
