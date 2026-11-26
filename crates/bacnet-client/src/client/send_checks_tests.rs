//! The remote broadcast forms (#2709) and the link-native text of a
//! refused router (#2722).
use super::*;

/// Each form `RemoteBroadcast` names (its own tests are bacnet-npdu's) is
/// refused, as the one-station mark has them (#2738), and a station's MAC
/// of each length goes.
#[test]
fn a_confirmed_request_refuses_exactly_the_remote_broadcast_forms() {
    let refused: [(&[u8], bool); 10] = [
        (&[0xFF], false),
        (&[0x00, 0x05], false),
        (&[0xFF, 0x07], false),
        (&[0x00, 0x00], true),
        (&[0xFF, 0xFF], true),
        (&[0xFF, 0xFF, 0xFF], false),
        (&[0xFF, 0xFF, 0xFF], true),
        (&[0xFF; 6], false),
        (&[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0], false),
        (&[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0], true),
    ];
    for (mac, one_station) in refused {
        let checked = check_remote_station(7, mac, one_station);
        assert!(checked.is_err(), "{mac:02X?} one_station {one_station}");
    }
    let stations: [(&[u8], bool); 11] = [
        (&[0xFF], true),
        (&[0x00], true),
        (&[0x00, 0x05], true),
        (&[0xFF, 0x07], true),
        (&[0x00], false),
        (&[0x01, 0x05], false),
        (&[0x3F, 0xFF, 0xFF], false),
        (&[0xFF, 0xFF, 0xFF, 0xFE, 0xBA, 0xC0], false),
        (&[192, 168, 1, 255, 0xBA, 0xC0], false),
        (&[0xFF; 7], false),
        (&[0xFF; 18], false),
    ];
    for (mac, one_station) in stations {
        let checked = check_remote_station(7, mac, one_station);
        assert!(
            checked.is_ok(),
            "{mac:02X?} one_station {one_station}: {checked:?}"
        );
    }
}

/// The refusal names the station as the target grammar writes it, the form,
/// and that a remote broadcast takes only an unconfirmed request.
#[test]
fn a_remote_broadcast_refusal_says_which_form_and_what_to_use() {
    let Err(Error::InvalidArgument(text)) = check_remote_station(7, &[0xFF], false) else {
        panic!("refused");
    };
    let text = text.to_string();
    assert!(
        text.contains("7:0xFF on network 7 is station 255"),
        "{text}"
    );
    assert!(
        text.contains("(ARCNET's station 255), mark it one station (one_station)"),
        "{text}"
    );
    assert!(!text.contains("two octets"), "{text}");
    assert!(text.contains("only an unconfirmed request"), "{text}");
    let decoded: [(u16, &[u8], &str); 3] = [
        (
            9,
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0],
            "9:0xFFFFFFFFBAC0 on network 9 is a broadcast on every six-octet link: \
             255.255.255.255:47808, the limited broadcast, on BACnet/IP",
        ),
        (
            4,
            &[0x00, 0x05],
            "4:0x0005 on network 4 is a LonTalk broadcast (subnet 5)",
        ),
        (
            4,
            &[0xFF, 0x07],
            "4:0xFF07 on network 4 is LonTalk group 7, a multicast",
        ),
    ];
    for (network, mac, expected) in decoded {
        let Err(Error::InvalidArgument(text)) = check_remote_station(network, mac, false) else {
            panic!("refused");
        };
        let text = text.to_string();
        assert!(text.contains(expected), "{text}");
        // A two-octet form's hint names a gateway's two-octet station, not
        // ARCNET; the six-octet form names no mark, which can't lift it.
        let two = mac.len() == 2;
        assert_eq!(
            text.contains("(a station a gateway numbers in two octets), mark it one station"),
            two,
            "{text}"
        );
        assert!(!text.contains("ARCNET"), "{text}");
    }
}

#[test]
fn a_router_is_written_as_its_link_writes_it() {
    let cases: [(Option<DataLink>, &[u8], &str); 7] = [
        (
            Some(DataLink::Bip),
            &[255, 255, 255, 255, 0xBA, 0xC0],
            "255.255.255.255:47808",
        ),
        (
            Some(DataLink::Bip),
            &[224, 0, 0, 1, 0xBA, 0xC0],
            "224.0.0.1:47808",
        ),
        (Some(DataLink::Mstp), &[0xFF], "MS/TP station 255"),
        (Some(DataLink::Bip6), &[0x00, 0x04, 0xD2], "VMAC 0x0004D2"),
        (
            Some(DataLink::Ethernet),
            &[0x01, 0x80, 0xC2, 0, 0, 0],
            "01:80:C2:00:00:00",
        ),
        (Some(DataLink::Sc), &[0xFF; 6], "FF:FF:FF:FF:FF:FF"),
        (None, &[0xFF], "FF"),
    ];
    for (link, mac, text) in cases {
        assert_eq!(link_text(link, mac), text);
    }
}
