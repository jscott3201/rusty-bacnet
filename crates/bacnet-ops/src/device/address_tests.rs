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
    // A remote 0xFF is that MS/TP network's broadcast: a write there would
    // reach every station on it. So do the client's other remote broadcast
    // and group forms (#2709), here or on another network: the all-ones
    // VMAC, and six octets beginning with four of 0xFF, the B/IP limited
    // broadcast at any port.
    for network in [0, 5] {
        for mac in [
            &[0xFF][..],
            &[0xFF; 3],
            &[0xFF; 6],
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0],
        ] {
            let refused = DeviceAddress::new(network, mac).unwrap_err();
            assert!(refused.contains(", not a station"), "{refused}");
            // Only a remote 0xFF may be one station there (ARCNET's 255),
            // which the mark lets through, written in the grammar
            // (#2738).
            assert_eq!(
                refused.contains("5:0xFF:one-station"),
                network != 0 && mac == [0xFF],
                "{refused}"
            );
        }
    }
    // Two octets are LonTalk's, a remote network's only: a first octet of
    // 0x00 is a broadcast, of 0xFF a group.
    let refused = DeviceAddress::new(5, &[0x00, 0x05]).unwrap_err();
    assert_eq!(
        refused,
        "MAC 0x0005 is a LonTalk broadcast (subnet 5), not a station; a station at such an \
         address is written with the one-station mark: 5:0x0005:one-station"
    );
    assert!(DeviceAddress::new(5, &[0xFF, 0x07]).is_err());
    assert!("5:0xFFFFFFFFFFFF".parse::<DeviceAddress>().is_err());
    assert!("5:0xFFFFFFFFBAC0".parse::<DeviceAddress>().is_err());
    // Other one-, two- and six-octet MACs are stations.
    assert!(DeviceAddress::new(5, &[0x00]).is_ok());
    assert!(DeviceAddress::new(5, &[0x01, 0x05]).is_ok());
    assert!(DeviceAddress::new(5, &[0xFF, 0xFF, 0xFF, 0xFE, 0xBA, 0xC0]).is_ok());
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

/// The mark follows from the MAC: over every one- and two-octet MAC on a
/// few networks, the address is marked exactly when the unmarked
/// constructor refuses the MAC for a form the mark lifts, only `marked`
/// builds it, the text writes the suffix there and nowhere else, and every
/// address reads back as it was (#2738).
#[test]
fn the_mark_follows_the_mac_and_every_address_reads_back() {
    let one = (0..=0xFFu8).map(|a| vec![a]);
    let two = (0..=0xFFFFu16).map(|a| a.to_be_bytes().to_vec());
    for mac in one.chain(two) {
        let lifted = crate::target::lifted_by_one_station(&mac);
        for network in [1u16, 5, 65534] {
            let unmarked = DeviceAddress::new(network, &mac);
            let marked = DeviceAddress::marked(network, &mac);
            assert_eq!(marked.is_ok(), lifted, "{network}:{mac:02X?}");
            if lifted {
                // Only the marked constructor reaches it.
                assert!(unmarked.is_err(), "{network}:{mac:02X?}");
                let address = marked.unwrap();
                assert!(address.is_one_station());
                let text = address.to_string();
                assert!(text.ends_with(":one-station"), "{text}");
                assert_eq!(text.parse::<DeviceAddress>().unwrap(), address, "{text}");
                // The same text without the suffix is refused, naming it.
                let bare = text.trim_end_matches(":one-station");
                let refused = bare.parse::<DeviceAddress>().unwrap_err();
                assert!(refused.contains(&text), "{refused}");
                // Written and read once more: one spelling.
                let json = serde_json::to_string(&address).unwrap();
                assert_eq!(json, format!("\"{text}\""));
                assert_eq!(
                    serde_json::from_str::<DeviceAddress>(&json).unwrap(),
                    address
                );
            } else if let Ok(address) = unmarked {
                assert!(!address.is_one_station());
                let text = address.to_string();
                assert!(!text.contains("one-station"), "{text}");
                assert_eq!(text.parse::<DeviceAddress>().unwrap(), address, "{text}");
                // The suffix on a station it lifts nothing for is refused.
                assert!(
                    format!("{text}:one-station")
                        .parse::<DeviceAddress>()
                        .is_err()
                );
            }
        }
    }
}

/// The suffix lifts nothing on the local network, 0 and 65535, or at a
/// MAC of another length; `marked` says why.
#[test]
fn marked_refuses_where_the_mark_lifts_nothing() {
    for (network, mac, why) in [
        (0, &[0xFF][..], "local network"),
        (65535, &[0xFF][..], "every network"),
        (5, &[0x05][..], "an ordinary station"),
        (5, &[0x00][..], "an ordinary station"),
        (5, &[0x00, 0x00][..], "whole domain"),
        (5, &[0xFF, 0xFF][..], "LonTalk group 255"),
        (5, &[0xFF; 3][..], "VMAC 0xFFFFFF"),
        (5, &[0xFF; 6][..], "six-octet link"),
        (5, &[0x0A, 0, 0, 1, 0xBA, 0xC0][..], "an ordinary station"),
    ] {
        let error = DeviceAddress::marked(network, mac).unwrap_err();
        assert!(error.contains(why), "{network}:{mac:02X?}: {error}");
    }
    // The grammar's refusals read the same, from text.
    for text in [
        "5:0x00:one-station",
        "5:0x0000:one-station",
        "5:0x0A:one-station",
    ] {
        assert!(text.parse::<DeviceAddress>().is_err(), "{text}");
    }
    // A device instance has no marked form.
    assert!("5:1234:one-station".parse::<DeviceAddress>().is_err());
}

/// A discovered device never takes the mark: its address at a lifted form
/// is none, since an I-Am can't say what a router's network is (#2738).
#[test]
fn a_discovered_device_never_invents_the_mark() {
    use bacnet_client::discovery::DiscoveredDevice;
    use bacnet_types::enums::{ObjectType, Segmentation};
    use bacnet_types::primitives::ObjectIdentifier;
    let row = |source: &[u8]| DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 7).unwrap(),
        mac_address: MacAddr::from_slice(&[10, 0, 0, 1, 0xBA, 0xC0]),
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
        vendor_id: 0,
        last_seen: std::time::Instant::now(),
        last_announced: None,
        source_network: Some(5),
        source_address: Some(MacAddr::from_slice(source)),
    };
    assert!(DeviceAddress::of(&row(&[0x05])).is_some());
    for lifted in [&[0xFF][..], &[0x00, 0x12], &[0xFF, 0x07]] {
        assert!(DeviceAddress::of(&row(lifted)).is_none(), "{lifted:02X?}");
    }
}

/// Serde and the typed calls: a ledger line from before the suffix reads
/// as it did, one with the suffix reads marked, one at a lifted form
/// without it is refused as always, and the conversion to the client's
/// `Station` carries the mark, so a confirmed request goes.
#[test]
fn serde_and_the_station_conversion_carry_the_mark() {
    use bacnet_client::client::Station;
    let old = serde_json::from_str::<DeviceAddress>("\"2:0x05\"").unwrap();
    assert!(!old.is_one_station());
    assert_eq!(
        Station::from(&old),
        Station::remote(2, &[5]),
        "an ordinary station is unmarked"
    );
    let marked: DeviceAddress = serde_json::from_str("\"5:0xFF:one-station\"").unwrap();
    assert!(marked.is_one_station());
    assert_eq!(marked.network(), 5);
    assert_eq!(marked.mac(), [0xFF]);
    let station = Station::from(&marked);
    assert_eq!(station, Station::remote(5, &[0xFF]).one_station());
    assert!(station.is_one_station());
    station.check_confirmed().unwrap();
    let wide = DeviceAddress::marked(9, &[0x00, 0x12]).unwrap();
    Station::from(wide).check_confirmed().unwrap();
    // A line at a lifted form without the suffix is refused, as before.
    for line in ["\"5:0xFF\"", "\"9:0x0012\"", "\"5:0x05:one-station\""] {
        assert!(
            serde_json::from_str::<DeviceAddress>(line).is_err(),
            "{line}"
        );
    }
    let local = DeviceAddress::local(&[7]).unwrap();
    assert_eq!(Station::from(local), Station::local(&[7]));
}
