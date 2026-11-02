//! `NetworkPortEntry`: the fields each constructor fills in, and the checks
//! that keep a declared entry describing one configured port (#2183).
use super::*;
use crate::network_port::NetworkPortObject;
use bacnet_types::MacAddr;

const LAN: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 10);
const LAN6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 0x10);

fn read(port: &NetworkPortObject, property: P) -> PropertyValue {
    port.read_property(property, None).unwrap()
}

/// The `Error::Encoding` message of building `entry`.
fn build_error(entry: &NetworkPortEntry) -> String {
    match entry.build_object() {
        Err(Error::Encoding(message)) => message,
        Err(other) => panic!("expected an encoding error for {entry:?}, got {other:?}"),
        Ok(_) => panic!("expected an encoding error for {entry:?}, got a port"),
    }
}

#[test]
fn a_bip_entry_derives_its_mac_from_the_address_and_port() {
    let entry = NetworkPortEntry::bip(1, 10, LAN, 0xBAC0);
    assert_eq!(entry.network_type, NetworkType::IPV4.to_raw());
    assert_eq!(entry.network_number, 10);
    assert_eq!(entry.mac.as_slice(), &[192, 168, 1, 10, 0xBA, 0xC0]);
    assert_eq!(entry.ip, LAN.octets());
    assert_eq!(entry.ipv6, [0; 16]);
    assert_eq!(entry.udp_port, 0xBAC0);

    let port = entry.build_object().unwrap();
    assert_eq!(
        read(&port, P::NETWORK_TYPE),
        PropertyValue::Enumerated(NetworkType::IPV4.to_raw())
    );
    assert_eq!(read(&port, P::NETWORK_NUMBER), PropertyValue::Unsigned(10));
    assert_eq!(
        read(&port, P::MAC_ADDRESS),
        PropertyValue::OctetString(entry.mac.to_vec())
    );
    assert_eq!(read(&port, P::APDU_LENGTH), PropertyValue::Unsigned(1476));
    // An unbound UDP port zero is still a valid declaration.
    NetworkPortEntry::bip(1, 10, LAN, 0).build_object().unwrap();
}

#[test]
fn a_bip6_entry_carries_a_zero_vmac_until_one_is_published() {
    let entry = NetworkPortEntry::bip6(2, 20, LAN6, 47808);
    assert_eq!(entry.network_type, NetworkType::IPV6.to_raw());
    assert_eq!(entry.mac.as_slice(), &[0, 0, 0]);
    assert_eq!(entry.ip, [0; 4]);
    assert_eq!(entry.ipv6, LAN6.octets());
    assert_eq!(entry.vmac().unwrap(), [0, 0, 0]);

    let known = NetworkPortEntry::bip6_with_vmac(2, 20, LAN6, 47808, [1, 2, 3]);
    assert_eq!(known.vmac().unwrap(), [1, 2, 3]);
    let port = known.build_object().unwrap();
    assert_eq!(
        read(&port, P::NETWORK_TYPE),
        PropertyValue::Enumerated(NetworkType::IPV6.to_raw())
    );
    assert_eq!(read(&port, P::NETWORK_NUMBER), PropertyValue::Unsigned(20));
    assert_eq!(
        read(&port, P::MAC_ADDRESS),
        PropertyValue::OctetString(vec![1, 2, 3])
    );
}

#[test]
fn an_sc_entry_is_virtual_and_has_no_socket() {
    let entry = NetworkPortEntry::sc(3, 30, [1, 2, 3, 4, 5, 6]);
    assert_eq!(entry.network_type, NetworkType::VIRTUAL.to_raw());
    assert_eq!(entry.mac.as_slice(), &[1, 2, 3, 4, 5, 6]);
    assert_eq!((entry.ip, entry.ipv6, entry.udp_port), ([0; 4], [0; 16], 0));
    let port = entry.build_object().unwrap();
    assert_eq!(
        read(&port, P::NETWORK_TYPE),
        PropertyValue::Enumerated(NetworkType::VIRTUAL.to_raw())
    );
    assert_eq!(read(&port, P::NETWORK_NUMBER), PropertyValue::Unsigned(30));
    assert_eq!(
        read(&port, P::OBJECT_NAME),
        PropertyValue::CharacterString("port-3".into())
    );
}

#[test]
fn a_bip_entry_whose_fields_disagree_is_refused() {
    let mut moved = NetworkPortEntry::bip(1, 10, LAN, 47808);
    moved.udp_port = 47809;
    assert!(build_error(&moved).contains("B/IP MAC"));

    let mut readdressed = NetworkPortEntry::bip(1, 10, LAN, 47808);
    readdressed.ip = [10, 0, 0, 1];
    assert!(build_error(&readdressed).contains("B/IP MAC"));

    let mut dual = NetworkPortEntry::bip(1, 10, LAN, 47808);
    dual.ipv6 = LAN6.octets();
    assert!(build_error(&dual).contains("no IPv6"));
}

#[test]
fn a_bip6_entry_needs_a_three_octet_vmac_and_no_ipv4_address() {
    let mut dual = NetworkPortEntry::bip6(2, 20, LAN6, 47808);
    dual.ip = LAN.octets();
    assert!(build_error(&dual).contains("no IPv4"));
    assert!(dual.vmac().is_err());

    for mac in [&[1, 2][..], &[1, 2, 3, 4, 5, 6][..]] {
        let mut entry = NetworkPortEntry::bip6(2, 20, LAN6, 47808);
        entry.mac = MacAddr::from_slice(mac);
        assert!(build_error(&entry).contains("3-octet VMAC"), "{mac:?}");
    }
}

#[test]
fn a_network_number_past_sixteen_bits_is_refused_for_every_type() {
    for entry in [
        NetworkPortEntry::bip(1, 65_536, LAN, 47808),
        NetworkPortEntry::bip6(2, 65_536, LAN6, 47808),
        NetworkPortEntry::sc(3, u32::MAX, [1; 6]),
    ] {
        assert!(build_error(&entry).contains("network number"), "{entry:?}");
    }
    // 65535 fits in 16 bits but is no configurable network.
    assert!(
        NetworkPortEntry::sc(3, 65_535, [1; 6])
            .build_object()
            .is_err()
    );
    NetworkPortEntry::sc(3, 65_534, [1; 6])
        .build_object()
        .unwrap();
}

#[test]
fn other_link_types_build_only_where_the_port_object_takes_them() {
    let mut ethernet = NetworkPortEntry::sc(4, 40, [2; 6]);
    ethernet.network_type = NetworkType::ETHERNET.to_raw();
    let port = ethernet.build_object().unwrap();
    assert_eq!(
        read(&port, P::NETWORK_TYPE),
        PropertyValue::Enumerated(NetworkType::ETHERNET.to_raw())
    );

    // MS/TP has no generic profile here.
    let mut mstp = NetworkPortEntry::sc(5, 50, [3; 6]);
    mstp.network_type = NetworkType::MSTP.to_raw();
    mstp.mac = MacAddr::from_slice(&[3]);
    assert!(mstp.build_object().is_err());
}
