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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_type_is_network_port() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        assert_eq!(
            np.object_identifier().object_type(),
            ObjectType::NETWORK_PORT
        );
        assert_eq!(np.object_identifier().instance_number(), 1);
    }

    #[test]
    fn read_object_name() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("NP-1".to_string()));
    }

    #[test]
    fn read_object_type() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::NETWORK_PORT.to_raw())
        );
    }

    #[test]
    fn read_network_type() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::NETWORK_TYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // IPv4
    }

    #[test]
    fn read_network_number_default() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::NETWORK_NUMBER, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(0));
    }

    #[test]
    fn read_max_apdu_length() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(1476));
    }

    #[test]
    fn read_link_speed_default() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::LINK_SPEED, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(0.0));
    }

    #[test]
    fn read_changes_pending_default() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::CHANGES_PENDING, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn read_command_default() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::COMMAND_NP, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0)); // idle
    }

    #[test]
    fn read_ip_address_default() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::IP_ADDRESS, None)
            .unwrap();
        assert_eq!(val, PropertyValue::OctetString(vec![0, 0, 0, 0]));
    }

    #[test]
    fn read_ip_default_gateway_default() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::IP_DEFAULT_GATEWAY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::OctetString(vec![0, 0, 0, 0]));
    }

    #[test]
    fn read_ip_subnet_mask_default() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::IP_SUBNET_MASK, None)
            .unwrap();
        assert_eq!(val, PropertyValue::OctetString(vec![255, 255, 255, 0]));
    }

    #[test]
    fn read_udp_port_default() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let val = np
            .read_property(PropertyIdentifier::BACNET_IP_UDP_PORT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(0xBAC0));
    }

    #[test]
    fn write_command() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        np.write_property(
            PropertyIdentifier::COMMAND_NP,
            None,
            PropertyValue::Enumerated(1), // discardChanges
            None,
        )
        .unwrap();
        let val = np
            .read_property(PropertyIdentifier::COMMAND_NP, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(1));
    }

    #[test]
    fn write_command_wrong_type() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let result = np.write_property(
            PropertyIdentifier::COMMAND_NP,
            None,
            PropertyValue::Unsigned(1),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn write_ip_address_sets_changes_pending() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        assert_eq!(
            np.read_property(PropertyIdentifier::CHANGES_PENDING, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );

        np.write_property(
            PropertyIdentifier::IP_ADDRESS,
            None,
            PropertyValue::OctetString(vec![192, 168, 1, 100]),
            None,
        )
        .unwrap();

        assert_eq!(
            np.read_property(PropertyIdentifier::IP_ADDRESS, None)
                .unwrap(),
            PropertyValue::OctetString(vec![192, 168, 1, 100])
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::CHANGES_PENDING, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
    }

    #[test]
    fn write_ip_default_gateway_sets_changes_pending() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        np.write_property(
            PropertyIdentifier::IP_DEFAULT_GATEWAY,
            None,
            PropertyValue::OctetString(vec![192, 168, 1, 1]),
            None,
        )
        .unwrap();

        assert_eq!(
            np.read_property(PropertyIdentifier::IP_DEFAULT_GATEWAY, None)
                .unwrap(),
            PropertyValue::OctetString(vec![192, 168, 1, 1])
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::CHANGES_PENDING, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
    }

    #[test]
    fn write_ip_subnet_mask_sets_changes_pending() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        np.write_property(
            PropertyIdentifier::IP_SUBNET_MASK,
            None,
            PropertyValue::OctetString(vec![255, 255, 0, 0]),
            None,
        )
        .unwrap();

        assert_eq!(
            np.read_property(PropertyIdentifier::IP_SUBNET_MASK, None)
                .unwrap(),
            PropertyValue::OctetString(vec![255, 255, 0, 0])
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::CHANGES_PENDING, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
    }

    #[test]
    fn write_udp_port_sets_changes_pending() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        np.write_property(
            PropertyIdentifier::BACNET_IP_UDP_PORT,
            None,
            PropertyValue::Unsigned(47809),
            None,
        )
        .unwrap();

        assert_eq!(
            np.read_property(PropertyIdentifier::BACNET_IP_UDP_PORT, None)
                .unwrap(),
            PropertyValue::Unsigned(47809)
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::CHANGES_PENDING, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
    }

    #[test]
    fn write_udp_port_out_of_range() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let result = np.write_property(
            PropertyIdentifier::BACNET_IP_UDP_PORT,
            None,
            PropertyValue::Unsigned(70000),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn write_udp_port_wrong_type() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let result = np.write_property(
            PropertyIdentifier::BACNET_IP_UDP_PORT,
            None,
            PropertyValue::Real(47808.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn write_network_number() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        np.write_property(
            PropertyIdentifier::NETWORK_NUMBER,
            None,
            PropertyValue::Unsigned(5),
            None,
        )
        .unwrap();
        assert_eq!(
            np.read_property(PropertyIdentifier::NETWORK_NUMBER, None)
                .unwrap(),
            PropertyValue::Unsigned(5)
        );
    }

    #[test]
    fn write_mac_address() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let mac = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
        np.write_property(
            PropertyIdentifier::MAC_ADDRESS,
            None,
            PropertyValue::OctetString(mac.clone()),
            None,
        )
        .unwrap();
        assert_eq!(
            np.read_property(PropertyIdentifier::MAC_ADDRESS, None)
                .unwrap(),
            PropertyValue::OctetString(mac)
        );
    }

    #[test]
    fn write_out_of_service() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        np.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        assert_eq!(
            np.read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
    }

    #[test]
    fn write_description() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        np.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("Main Ethernet port".to_string()),
            None,
        )
        .unwrap();
        assert_eq!(
            np.read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("Main Ethernet port".to_string())
        );
    }

    #[test]
    fn write_read_only_property_denied() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        // LINK_SPEED is read-only
        let result = np.write_property(
            PropertyIdentifier::LINK_SPEED,
            None,
            PropertyValue::Real(100_000_000.0),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn read_unknown_property() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let result = np.read_property(PropertyIdentifier::PRESENT_VALUE, None);
        assert!(result.is_err());
    }

    #[test]
    fn property_list_complete() {
        let np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        let props = np.property_list();
        assert!(props.contains(&PropertyIdentifier::OBJECT_IDENTIFIER));
        assert!(props.contains(&PropertyIdentifier::OBJECT_NAME));
        assert!(props.contains(&PropertyIdentifier::OBJECT_TYPE));
        assert!(props.contains(&PropertyIdentifier::NETWORK_TYPE));
        assert!(props.contains(&PropertyIdentifier::NETWORK_NUMBER));
        assert!(props.contains(&PropertyIdentifier::MAC_ADDRESS));
        assert!(props.contains(&PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED));
        assert!(props.contains(&PropertyIdentifier::LINK_SPEED));
        assert!(props.contains(&PropertyIdentifier::CHANGES_PENDING));
        assert!(props.contains(&PropertyIdentifier::COMMAND_NP));
        assert!(props.contains(&PropertyIdentifier::IP_ADDRESS));
        assert!(props.contains(&PropertyIdentifier::IP_DEFAULT_GATEWAY));
        assert!(props.contains(&PropertyIdentifier::IP_SUBNET_MASK));
        assert!(props.contains(&PropertyIdentifier::BACNET_IP_UDP_PORT));
    }

    #[test]
    fn setter_methods_work() {
        let mut np = NetworkPortObject::new(1, "NP-1", 0).unwrap();
        np.set_ip_address(vec![10, 0, 0, 1]);
        np.set_ip_default_gateway(vec![10, 0, 0, 254]);
        np.set_ip_subnet_mask(vec![255, 255, 255, 0]);
        np.set_mac_address(MacAddr::from_slice(&[0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E]));
        np.set_network_number(7);
        np.set_link_speed(100_000_000.0);
        np.set_udp_port(47808);
        np.set_description("Test port");

        assert_eq!(
            np.read_property(PropertyIdentifier::IP_ADDRESS, None)
                .unwrap(),
            PropertyValue::OctetString(vec![10, 0, 0, 1])
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::IP_DEFAULT_GATEWAY, None)
                .unwrap(),
            PropertyValue::OctetString(vec![10, 0, 0, 254])
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::NETWORK_NUMBER, None)
                .unwrap(),
            PropertyValue::Unsigned(7)
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::LINK_SPEED, None)
                .unwrap(),
            PropertyValue::Real(100_000_000.0)
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::BACNET_IP_UDP_PORT, None)
                .unwrap(),
            PropertyValue::Unsigned(47808)
        );
    }

    #[test]
    fn mstp_network_type() {
        let np = NetworkPortObject::new(2, "NP-MSTP", 2).unwrap();
        assert_eq!(
            np.read_property(PropertyIdentifier::NETWORK_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(2) // MS/TP
        );
    }

    #[test]
    fn full_network_config_scenario() {
        let mut np = NetworkPortObject::new(1, "Ethernet-1", 0).unwrap();

        // Configure the port
        np.set_ip_address(vec![192, 168, 1, 100]);
        np.set_ip_default_gateway(vec![192, 168, 1, 1]);
        np.set_ip_subnet_mask(vec![255, 255, 255, 0]);
        np.set_mac_address(MacAddr::from_slice(&[0x00, 0x50, 0x56, 0xAB, 0xCD, 0xEF]));
        np.set_network_number(1);
        np.set_link_speed(1_000_000_000.0); // 1 Gbps
        np.set_udp_port(0xBAC0);

        // Verify all reads
        assert_eq!(np.object_name(), "Ethernet-1");
        assert_eq!(
            np.read_property(PropertyIdentifier::IP_ADDRESS, None)
                .unwrap(),
            PropertyValue::OctetString(vec![192, 168, 1, 100])
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::LINK_SPEED, None)
                .unwrap(),
            PropertyValue::Real(1_000_000_000.0)
        );
        assert_eq!(
            np.read_property(PropertyIdentifier::MAC_ADDRESS, None)
                .unwrap(),
            PropertyValue::OctetString(vec![0x00, 0x50, 0x56, 0xAB, 0xCD, 0xEF])
        );

        // Write IP via property write (triggers changes_pending)
        np.write_property(
            PropertyIdentifier::IP_ADDRESS,
            None,
            PropertyValue::OctetString(vec![10, 0, 0, 50]),
            None,
        )
        .unwrap();
        assert_eq!(
            np.read_property(PropertyIdentifier::CHANGES_PENDING, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );

        // Discard changes via command
        np.write_property(
            PropertyIdentifier::COMMAND_NP,
            None,
            PropertyValue::Enumerated(1), // discardChanges
            None,
        )
        .unwrap();
        assert_eq!(
            np.read_property(PropertyIdentifier::COMMAND_NP, None)
                .unwrap(),
            PropertyValue::Enumerated(1)
        );
    }
}
