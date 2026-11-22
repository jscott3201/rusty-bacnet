//! The remote broadcast forms (#2709) and the link-native text of a
//! refused router (#2722).
use super::*;

#[test]
fn only_the_all_ones_forms_of_one_three_and_six_octets_are_remote_broadcasts() {
    use RemoteBroadcast::*;
    let cases: [(&[u8], bool, Option<RemoteBroadcast>); 14] = [
        (&[0xFF], false, Some(MstpBroadcast)),
        // Marked ARCNET: 255 is a station and 0 the broadcast.
        (&[0xFF], true, None),
        (&[0x00], true, Some(ArcnetBroadcast)),
        (&[0x00], false, None),
        (&[0xFE], false, None),
        (&[0xFF, 0xFF, 0xFF], false, Some(NoVmac)),
        (&[0xFF, 0xFF, 0xFF], true, Some(NoVmac)),
        (&[0x3F, 0xFF, 0xFF], false, None),
        (&[0xFF; 6], false, Some(SixOctetBroadcast)),
        (&[0xFF; 6], true, Some(SixOctetBroadcast)),
        (&[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0], false, None),
        // Two and seven octets (LonTalk) and other lengths: one station.
        (&[0xFF, 0xFF], false, None),
        (&[0xFF; 7], false, None),
        (&[0xFF; 18], false, None),
    ];
    for (mac, arcnet, form) in cases {
        assert_eq!(
            RemoteBroadcast::of(mac, arcnet),
            form,
            "{mac:02X?} arcnet {arcnet}"
        );
        let checked = check_remote_station(7, mac, arcnet);
        assert_eq!(checked.is_err(), form.is_some(), "{mac:02X?}: {checked:?}");
    }
}

/// The refusal names the station as the target grammar writes it, the form,
/// and that a remote broadcast takes only an unconfirmed request.
#[test]
fn a_remote_broadcast_refusal_says_which_form_and_what_to_use() {
    let Err(Error::Encoding(text)) = check_remote_station(7, &[0xFF], false) else {
        panic!("refused");
    };
    assert!(
        text.contains("7:0xFF on network 7 is station 255"),
        "{text}"
    );
    assert!(text.contains("mark the station ARCNET"), "{text}");
    assert!(text.contains("only an unconfirmed request"), "{text}");
    let Err(Error::Encoding(text)) = check_remote_station(9, &[0xFF; 6], false) else {
        panic!("refused");
    };
    assert!(
        text.contains("255.255.255.255:65535 on BACnet/IP"),
        "{text}"
    );
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
