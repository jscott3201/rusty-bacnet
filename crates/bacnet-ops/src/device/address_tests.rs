use super::*;

/// Every address form reads back from its text, as the ambiguity error
/// writes its candidates.
#[test]
fn an_address_reads_back_from_its_text() {
    let cases: [(u16, &[u8], &str); 5] = [
        (0, &[10, 0, 0, 1, 0xBA, 0xC0], "10.0.0.1:47808"),
        (0, &[0x00, 0x04, 0xD2], "bip6:0x0004D2"),
        (0, &[5], "mstp:5"),
        (2, &[0x05], "2:0x05"),
        (2001, &[10, 0, 0, 9, 0xBA, 0xC0], "2001:0x0A000009BAC0"),
    ];
    for (network, mac, text) in cases {
        let address = DeviceAddress::new(network, mac).unwrap();
        assert_eq!(address.to_string(), text);
        assert_eq!(text.parse::<DeviceAddress>().unwrap(), address, "{text}");
        let candidate = bacnet_types::error::Error::AmbiguousDevice {
            instance: 1,
            candidates: vec![BACnetAddress::from(&address)].into(),
        };
        assert!(candidate.to_string().contains(text), "{candidate}");
    }
    // A bare IPv4 address is port 47808; six hex octets are a MAC as written.
    let bare: DeviceAddress = "10.0.0.1".parse().unwrap();
    assert_eq!(bare.to_string(), "10.0.0.1:47808");
    let sc: DeviceAddress = "02:00:00:00:00:01".parse().unwrap();
    assert_eq!(sc.mac(), [2, 0, 0, 0, 0, 1]);
    // The grammar has no local form for other lengths, so no address
    // takes one, and every address reads back.
    for length in [2, 4, 5, 7, 18] {
        assert!(DeviceAddress::local(&vec![1; length]).is_err(), "{length}");
        assert!(DeviceAddress::new(2, &vec![1; length]).is_ok(), "{length}");
    }
    // An SC VMAC shows as hex on an SC link, and reads back either way.
    assert_eq!(sc.styled(AddressStyle::Hex), "02:00:00:00:00:01");
    assert_eq!(sc.styled(AddressStyle::Bip), sc.to_string());
    for text in [sc.styled(AddressStyle::Hex), sc.to_string()] {
        assert_eq!(text.parse::<DeviceAddress>().unwrap(), sc, "{text}");
    }
    let routed = DeviceAddress::new(2, &[5]).unwrap();
    assert_eq!(routed.styled(AddressStyle::Hex), "2:0x05");
}

/// An instance, a B/IPv6 socket address and a broadcast are no address.
#[test]
fn only_a_station_is_an_address() {
    for text in [
        "1234",
        "2:1234",
        "[fe80::1]",
        "mstp:255",
        "65535:0x05",
        "nonsense",
    ] {
        assert!(text.parse::<DeviceAddress>().is_err(), "{text}");
    }
    assert!(DeviceAddress::new(0, &[]).is_err());
    assert!(DeviceAddress::new(0, &[0xFF]).is_err());
    assert!(DeviceAddress::new(1, &[0; 19]).is_err());
    assert!(DeviceAddress::new(1, &[0; 18]).is_ok());
}

/// Serde writes the text and reads it back, for every MAC length an
/// address takes.
#[test]
fn serde_uses_the_text() {
    let address = DeviceAddress::new(2, &[5]).unwrap();
    let json = serde_json::to_string(&address).unwrap();
    assert_eq!(json, "\"2:0x05\"");
    assert_eq!(
        serde_json::from_str::<DeviceAddress>(&json).unwrap(),
        address
    );
    assert!(serde_json::from_str::<DeviceAddress>("\"1234\"").is_err());
    for network in [0, 7] {
        for length in 1..=BACnetAddress::MAX_MAC_LEN {
            let Ok(address) = DeviceAddress::new(network, &vec![0x2A; length]) else {
                continue;
            };
            let json = serde_json::to_string(&address).unwrap();
            let back: DeviceAddress = serde_json::from_str(&json).unwrap();
            assert_eq!(back, address, "{json}");
        }
    }
}
