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
