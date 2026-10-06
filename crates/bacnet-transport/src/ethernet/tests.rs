use super::*;
#[cfg(target_os = "linux")]
use crate::port::TransportPort;

#[test]
fn encode_decode_round_trip() {
    let dst = ETHERNET_BROADCAST;
    let src = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    let npdu = vec![0x01, 0x00, 0xAA, 0xBB];

    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);

    let decoded = decode_ethernet_frame(&buf).unwrap();
    assert_eq!(decoded.destination, dst);
    assert_eq!(decoded.source, src);
    assert_eq!(decoded.payload, npdu);
}

#[test]
fn ethernet_local_policy_marks_only_all_ff_as_group() {
    assert!(!is_ethernet_group(&[0x02, 0, 0, 0, 0, 1]));
    assert!(!is_ethernet_group(&[0x01, 0, 0x5e, 0, 0, 1]));
    assert!(is_ethernet_group(&ETHERNET_BROADCAST));
}

#[test]
fn llc_header_correct() {
    let dst = [0xFF; 6];
    let src = [0x00; 6];
    let npdu = vec![0xAA];
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    assert_eq!(buf[14], BACNET_LLC_DSAP);
    assert_eq!(buf[15], BACNET_LLC_SSAP);
    assert_eq!(buf[16], LLC_CONTROL_UI);
}

#[test]
fn length_field_correct() {
    let dst = [0xFF; 6];
    let src = [0x00; 6];
    let npdu = vec![0x01, 0x02, 0x03];
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    let length = u16::from_be_bytes([buf[12], buf[13]]);
    assert_eq!(length as usize, LLC_HEADER_LEN + npdu.len());
}

#[test]
fn rejects_short_frame() {
    assert!(decode_ethernet_frame(&[0; 10]).is_err());
}

#[test]
fn rejects_invalid_llc() {
    let mut buf = vec![0u8; 20];
    buf[14] = 0x00; // wrong DSAP
    buf[15] = 0x82;
    buf[16] = 0x03;
    buf[12] = 0x00;
    buf[13] = 0x04; // length = 4
    assert!(decode_ethernet_frame(&buf).is_err());
}

#[test]
fn rejects_truncated_payload() {
    let mut buf = vec![0u8; MIN_FRAME_LEN];
    buf[14] = BACNET_LLC_DSAP;
    buf[15] = BACNET_LLC_SSAP;
    buf[16] = LLC_CONTROL_UI;
    // Length claims more data than available
    buf[12] = 0x00;
    buf[13] = 0xFF;
    assert!(decode_ethernet_frame(&buf).is_err());
}

#[test]
fn rejects_ethertype_as_length() {
    let mut buf = vec![0u8; 20];
    buf[12] = 0x08;
    buf[13] = 0x00; // length = 2048 > 1500
    assert!(decode_ethernet_frame(&buf).is_err());
}

#[test]
fn rejects_length_1501() {
    let mut buf = vec![0u8; 20];
    buf[12] = (1501u16 >> 8) as u8;
    buf[13] = (1501u16 & 0xFF) as u8;
    assert!(decode_ethernet_frame(&buf).is_err());
}

#[test]
fn accepts_length_1500() {
    let mut buf = vec![0u8; 14 + 1500];
    buf[12] = (1500u16 >> 8) as u8;
    buf[13] = (1500u16 & 0xFF) as u8;
    buf[14] = BACNET_LLC_DSAP;
    buf[15] = BACNET_LLC_SSAP;
    buf[16] = LLC_CONTROL_UI;
    let result = decode_ethernet_frame(&buf);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().payload.len(), 1500 - LLC_HEADER_LEN);
}

#[test]
fn encode_pads_small_frame_to_minimum() {
    let dst = [0xFF; 6];
    let src = [0x00; 6];
    // 1-byte NPDU: frame would be 14 + 3 + 1 = 18 bytes without padding
    let npdu = vec![0xAA];
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    // Must be padded to 60 bytes (14 header + 46 payload)
    assert_eq!(buf.len(), 60);
    // Verify padding is zeros
    for &b in &buf[18..60] {
        assert_eq!(b, 0x00);
    }
}

#[test]
fn encode_does_not_pad_large_frame() {
    let dst = [0xFF; 6];
    let src = [0x00; 6];
    // 50-byte NPDU: frame is 14 + 3 + 50 = 67 bytes, above 60-byte minimum
    let npdu = vec![0xBB; 50];
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    assert_eq!(buf.len(), 67); // no padding needed
}

#[test]
fn padded_frame_decodes_correctly() {
    let dst = [0xFF; 6];
    let src = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    let npdu = vec![0x01]; // tiny payload
    let mut buf = BytesMut::new();
    encode_ethernet_frame(&mut buf, &dst, &src, &npdu);
    // Should be padded to 60 bytes
    assert_eq!(buf.len(), 60);
    // Decode should extract only the declared payload (not padding)
    let decoded = decode_ethernet_frame(&buf).unwrap();
    assert_eq!(decoded.payload, npdu);
    assert_eq!(decoded.source, src);
}

#[cfg(target_os = "linux")]
#[test]
fn ethernet_transport_new() {
    let t = EthernetTransport::new("eth0");
    assert_eq!(t.local_mac(), &[0; 6]); // not started yet
}

/// Group MACs: the all-ones broadcast and multicast MACs (IPv4 and IPv6
/// mapped, a bridge group, a locally administered group).
const GROUP_MACS: [[u8; 6]; 5] = [
    ETHERNET_BROADCAST,
    [0x01, 0x00, 0x5E, 0x00, 0x00, 0x01],
    [0x33, 0x33, 0x00, 0x00, 0x00, 0x01],
    [0x01, 0x80, 0xC2, 0x00, 0x00, 0x00],
    [0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
];

/// Individual MACs, a locally administered one and all-but-the-group-bit
/// included.
const INDIVIDUAL_MACS: [[u8; 6]; 3] = [
    [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
    [0x02, 0x00, 0x00, 0x00, 0x00, 0x01],
    [0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
];

/// #1493: the group bit, the low bit of the first octet, marks every group
/// MAC, so a multicast MAC is a group destination as the broadcast is.
#[test]
fn the_group_bit_marks_every_group_mac() {
    for mac in GROUP_MACS {
        assert!(is_group_mac(&mac), "{mac:02x?}");
    }
    for mac in INDIVIDUAL_MACS {
        assert!(!is_group_mac(&mac), "{mac:02x?}");
    }
    for length in [0, 1, 5, 7] {
        assert!(!is_group_mac(&vec![0xFF; length]), "{length} octets");
    }
}

/// The transport's live and owned rules are the group bit, while
/// `is_broadcast_mac` keeps to the all-ones broadcast.
#[cfg(target_os = "linux")]
#[test]
fn ethernet_group_destinations_are_every_group_mac() {
    let transport = EthernetTransport::new("unused");
    let owned = transport.group_destinations();
    for mac in GROUP_MACS {
        assert!(transport.is_group_destination(&mac), "{mac:02x?}");
        assert!(owned.contains(&mac), "{mac:02x?}");
        assert_eq!(transport.is_broadcast_mac(&mac), mac == ETHERNET_BROADCAST);
    }
    for mac in INDIVIDUAL_MACS {
        assert!(!transport.is_group_destination(&mac), "{mac:02x?}");
        assert!(!owned.contains(&mac), "{mac:02x?}");
    }
}

#[test]
fn ethernet_destination_policy_precedes_all_llc_controls() {
    let local = [2, 0, 0, 0, 0, 1];
    for control in [LLC_CONTROL_UI, LLC_CONTROL_XID_CMD, LLC_CONTROL_TEST_CMD] {
        for (destination, admitted) in [
            (local, true),
            (ETHERNET_BROADCAST, true),
            ([2, 0, 0, 0, 0, 2], false),
            ([1, 0, 0x5e, 0, 0, 1], false),
        ] {
            let mut frame = destination.to_vec();
            frame.extend_from_slice(&[2, 0, 0, 0, 0, 3, 0, 3, 0x82, 0x82, control]);
            assert_eq!(accepts_ethernet_destination(&frame, &local), admitted);
        }
    }
    for length in 0..6 {
        assert!(!accepts_ethernet_destination(&local[..length], &local));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn ethernet_number_capability_delegates_without_configured_authority() {
    use crate::{any::AnyTransport, mstp::LoopbackSerial};
    let transport = EthernetTransport::new("unused");
    assert!(transport.supports_local_nonrouter_number_controls());
    let any: AnyTransport<LoopbackSerial> = AnyTransport::Ethernet(transport);
    assert!(any.supports_local_nonrouter_number_controls());
    assert!(any.bip_port().is_none());
}

/// A raw frame to `destination` from `source` with LLC `control`, carrying
/// `info` after the LLC header, unpadded.
fn llc_frame(destination: [u8; 6], source: [u8; 6], control: u8, info: &[u8]) -> Vec<u8> {
    let mut frame = destination.to_vec();
    frame.extend_from_slice(&source);
    frame.extend_from_slice(&((LLC_HEADER_LEN + info.len()) as u16).to_be_bytes());
    frame.extend_from_slice(&[BACNET_LLC_DSAP, BACNET_LLC_SSAP, control]);
    frame.extend_from_slice(info);
    frame
}

/// An NPDU carrying Who-Is.
const WHO_IS: [u8; 4] = [0x01, 0x00, 0x10, 0x08];

/// #1492: a UI frame, an XID and a TEST from a group source MAC, sent to
/// this station or to the broadcast, are dropped and counted before an LLC
/// command is answered or a frame decoded, so the receive loop hands nothing
/// up and sends nothing back. The same frames from a station are answered or
/// decoded as before.
#[test]
fn a_frame_from_a_group_source_is_dropped_before_any_answer_or_decode() {
    use super::ingress::{classify_frame, FrameIngress};
    let local = [2, 0, 0, 0, 0, 1];
    let commands = [LLC_CONTROL_UI, LLC_CONTROL_XID_CMD, LLC_CONTROL_TEST_CMD];
    for source in GROUP_MACS {
        for destination in [local, ETHERNET_BROADCAST] {
            for control in commands {
                let frame = llc_frame(destination, source, control, &WHO_IS);
                assert_eq!(
                    classify_frame(&frame, &local),
                    FrameIngress::GroupSource { source },
                    "{source:02x?} to {destination:02x?}, control {control:#04x}"
                );
            }
        }
    }

    let station = INDIVIDUAL_MACS[0];
    let from_station = |control| llc_frame(local, station, control, &WHO_IS);
    assert_eq!(
        classify_frame(&from_station(LLC_CONTROL_XID_CMD), &local),
        FrameIngress::Xid { source: station }
    );
    assert_eq!(
        classify_frame(&from_station(LLC_CONTROL_TEST_CMD), &local),
        FrameIngress::Test {
            source: station,
            data: &WHO_IS
        }
    );
    let ui = from_station(LLC_CONTROL_UI);
    assert_eq!(classify_frame(&ui, &local), FrameIngress::Decode);
    assert_eq!(decode_ethernet_frame(&ui).unwrap().payload, WHO_IS[..]);

    // Destination admission still comes first, and this station's own
    // frames, and frames too short to name a source, are ignored.
    for control in commands {
        let elsewhere = llc_frame([2, 0, 0, 0, 0, 2], GROUP_MACS[1], control, &WHO_IS);
        assert_eq!(classify_frame(&elsewhere, &local), FrameIngress::Ignore);
        let own = llc_frame(local, local, control, &WHO_IS);
        assert_eq!(classify_frame(&own, &local), FrameIngress::Ignore);
    }
    let frame = llc_frame(local, GROUP_MACS[0], LLC_CONTROL_UI, &WHO_IS);
    for length in 0..12 {
        assert_eq!(
            classify_frame(&frame[..length], &local),
            FrameIngress::Ignore,
            "{length} octets"
        );
    }
}
