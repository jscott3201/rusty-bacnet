use super::*;

#[test]
fn parse_address_ipv4() {
    let mac = parse_address("192.168.1.100:47808").unwrap();
    assert_eq!(mac.len(), 6);
    assert_eq!(&mac[..4], &[192, 168, 1, 100]);
    assert_eq!(u16::from_be_bytes([mac[4], mac[5]]), 47808);
}

/// A B/IPv6 node is named by its VMAC (Clause U.1.3): a device instance's
/// 22 bits, high octet first (Clause H.7.2), or any three octets in hex.
#[test]
fn parse_address_bip6_vmac_forms() {
    assert_eq!(parse_address("bip6:1234").unwrap(), [0x00, 0x04, 0xD2]);
    assert_eq!(parse_address("bip6:0").unwrap(), [0, 0, 0]);
    assert_eq!(parse_address("bip6:4194303").unwrap(), [0x3F, 0xFF, 0xFF]);
    assert_eq!(parse_address("bip6:0x40ABCD").unwrap(), [0x40, 0xAB, 0xCD]);
    assert_eq!(parse_address("bip6:0x40abcd").unwrap(), [0x40, 0xAB, 0xCD]);
    assert_eq!(parse_address("bip6:0X40ABCD").unwrap(), [0x40, 0xAB, 0xCD]);
    // What resolve_vmac and local_address() print reads back, and so does
    // the raw hex form.
    let printed =
        bacnet_transport::bip6::format_vmac_text(bacnet_transport::bip6::Bip6Vmac::new([
            0x00, 0x04, 0xD2,
        ]));
    assert_eq!(printed, "bip6:0x0004D2");
    assert_eq!(
        parse_address(&printed).unwrap(),
        parse_address("00:04:d2").unwrap()
    );
}

#[test]
fn parse_address_refuses_bad_bip6_forms() {
    Python::initialize();
    for (address, digits) in [
        ("bip6:4194304", "4194304"),
        ("bip6:4294967295", "4294967295"),
        ("bip6:99999999999999999999", "99999999999999999999"),
    ] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("ValueError: B/IPv6 device instance must be in 0..=4194303, not {digits}"),
            "{address}"
        );
    }
    for address in [
        "bip6:",
        "bip6:-1",
        "bip6:12a",
        "bip6:0x",
        "bip6:0x0004D",
        "bip6:0x0004D2F",
        "bip6:0xGG0000",
        "bip6: 1234",
    ] {
        let err = parse_address(address).unwrap_err().to_string();
        assert!(err.starts_with("ValueError: '"), "{address}: {err}");
        assert!(err.contains("bip6:0x<6 hex digits>"), "{address}: {err}");
    }
}

/// A client on any transport but B/IPv6 refuses a `bip6:` VMAC up front,
/// rather than handing its transport a 3-octet MAC it can't send to.
#[test]
fn only_an_ipv6_client_takes_bip6_vmacs() {
    Python::initialize();
    let ipv6 = PeerAddresses::for_transport("ipv6");
    assert_eq!(ipv6.parse("bip6:1234").unwrap(), [0x00, 0x04, 0xD2]);
    for transport in ["bip", "sc", "mstp"] {
        let peers = PeerAddresses::for_transport(transport);
        let err = peers.parse("bip6:1234").unwrap_err().to_string();
        assert!(
            err.contains("bip6: addresses need transport='ipv6'"),
            "{err}"
        );
        // Other forms parse as before.
        assert_eq!(peers.parse("00:04:d2").unwrap(), [0x00, 0x04, 0xD2]);
        assert_eq!(peers.parse("7").unwrap(), [7]);
    }
}

/// A bracketed IPv6 address is where a node is, not its MAC: it is refused
/// with a pointer to the VMAC forms and `resolve_vmac`.
#[test]
fn parse_address_refuses_bracketed_ipv6() {
    Python::initialize();
    for address in [
        "[::1]:47808",
        "[fe80::1%3]:47808",
        "[::1]",
        "[::1",
        "[invalid]:47808",
    ] {
        let err = parse_address(address).unwrap_err().to_string();
        assert!(err.starts_with("ValueError: "), "{address}: {err}");
        assert!(err.contains("'bip6:<device instance>'"), "{address}: {err}");
        assert!(err.contains("resolve_vmac"), "{address}: {err}");
    }
}

#[test]
fn resolve_vmac_takes_a_bracketed_address() {
    Python::initialize();
    let peer = parse_bip6_address("[::1]:47808").unwrap();
    assert_eq!(*peer.ip(), std::net::Ipv6Addr::LOCALHOST);
    assert_eq!(peer.port(), 47808);
    let zoned = parse_bip6_address("[fe80::1%3]:47809").unwrap();
    assert_eq!((zoned.scope_id(), zoned.port()), (3, 47809));
    for address in ["::1", "bip6:1234", "[::1]", "[::1]:x", "[nope]:47808"] {
        let err = parse_bip6_address(address).unwrap_err().to_string();
        assert!(err.starts_with("ValueError: "), "{address}: {err}");
        assert!(err.contains("'[ip]:port'"), "{address}: {err}");
    }
}

#[test]
fn parse_address_hex_mac() {
    let mac = parse_address("01:02:03:04:05:06").unwrap();
    assert_eq!(mac, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn parse_address_mstp_peer_boundaries() {
    for mac in [0, 127, 128, 254] {
        assert_eq!(parse_address(&mac.to_string()).unwrap(), vec![mac as u8]);
        assert_eq!(
            parse_address(&format!("mstp:{mac}")).unwrap(),
            vec![mac as u8]
        );
    }
}

#[test]
fn parse_address_rejects_mstp_broadcast_and_out_of_range_peers() {
    Python::initialize();
    for address in ["255", "mstp:255"] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ValueError: MS/TP peer address 255 is broadcast, not a unicast peer"
        );
    }
    for address in ["256", "mstp:256", "99999999999999999999"] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ValueError: MS/TP peer address must be in 0..=254"
        );
    }
}

#[test]
fn parse_address_rejects_malformed_and_negative_mstp_peers_explicitly() {
    Python::initialize();
    for address in ["mstp:", "mstp:not-a-number"] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ValueError: MS/TP peer address must be a decimal integer in 0..=254"
        );
    }
    for address in ["-1", "mstp:-1"] {
        let err = parse_address(address).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ValueError: MS/TP peer address must be in 0..=254"
        );
    }
}

#[test]
fn parse_address_rejects_garbage() {
    assert!(parse_address("not_an_address").is_err());
}
