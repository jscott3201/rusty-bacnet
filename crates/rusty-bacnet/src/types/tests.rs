use super::*;

#[test]
fn parse_address_ipv4() {
    let mac = parse_address("192.168.1.100:47808").unwrap();
    assert_eq!(mac.len(), 6);
    assert_eq!(&mac[..4], &[192, 168, 1, 100]);
    assert_eq!(u16::from_be_bytes([mac[4], mac[5]]), 47808);
}

#[test]
fn parse_address_ipv6() {
    let mac = parse_address("[::1]:47808").unwrap();
    assert_eq!(mac.len(), 18);
    // ::1 → 15 zero bytes + 0x01
    assert_eq!(mac[15], 1);
    assert_eq!(u16::from_be_bytes([mac[16], mac[17]]), 47808);
}

#[test]
fn parse_address_ipv6_full() {
    let mac = parse_address("[fe80::1]:47808").unwrap();
    assert_eq!(mac.len(), 18);
    assert_eq!(mac[0], 0xfe);
    assert_eq!(mac[1], 0x80);
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

#[test]
fn parse_address_ipv6_missing_bracket() {
    assert!(parse_address("[::1").is_err());
}
