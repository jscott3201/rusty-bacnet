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
    // No local form for other lengths: written, but not read back.
    let odd = DeviceAddress::local(&[1, 2]).unwrap();
    assert_eq!(odd.to_string(), "0x0102");
    assert!(odd.to_string().parse::<DeviceAddress>().is_err());
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

/// Serde writes the text and reads it back.
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
}
