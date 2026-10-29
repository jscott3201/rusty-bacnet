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
    buf[14] = 0x0B; // wrong DSAP
    buf[15] = 0x82;
    buf[16] = 0x03;
    buf[12] = 0x00;
    buf[13] = 0x04; // length = 4

    // The octets print upper case and zero-padded, as before #1776 (#1803).
    match decode_ethernet_frame(&buf) {
        Err(Error::Decoding(refused)) => assert_eq!(
            refused.message().to_string(),
            "invalid DSAP: expected 0x82, got 0x0B"
        ),
        other => panic!("expected a decoding error, got {other:?}"),
    }
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

/// #1883: an interface that doesn't exist, or whose name the kernel can't
/// hold, is a clear `NotFound` naming it, not a panic.
#[cfg(target_os = "linux")]
#[test]
fn a_missing_interface_is_a_clear_error() {
    for name in [
        "bacnet-none0",
        "an-interface-name-far-too-long",
        "nul\0name",
        "",
    ] {
        match super::transport::interface_link(name) {
            Err(Error::Transport(error)) => {
                assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{name:?}");
                assert!(error.to_string().contains(&format!("{name:?}")), "{error}");
            }
            other => panic!("{name:?}: {other:?}"),
        }
    }
    // A lookup that fails for want of a resource is not a missing interface.
    use nix::errno::Errno;
    for (errno, kind) in [
        (Errno::ENODEV, std::io::ErrorKind::NotFound),
        (Errno::ENXIO, std::io::ErrorKind::NotFound),
        (Errno::EINVAL, std::io::ErrorKind::NotFound),
        (Errno::EMFILE, std::io::Error::from(Errno::EMFILE).kind()),
        (Errno::ENOMEM, std::io::ErrorKind::OutOfMemory),
    ] {
        let Error::Transport(error) = super::transport::lookup_error("eth9", "index", errno) else {
            panic!("{errno}");
        };
        assert_eq!(error.kind(), kind, "{errno}");
        assert!(error.to_string().contains("\"eth9\""), "{error}");
    }
    assert_ne!(
        std::io::Error::from(Errno::EMFILE).kind(),
        std::io::ErrorKind::NotFound
    );
}

/// #1883: the drain after the bind empties the queue, a datagram longer
/// than its buffer included, and stops at an empty one. A Unix datagram
/// pair stands in for the packet socket: delivery is synchronous, and a
/// short read discards the rest of a datagram as it does a frame.
#[cfg(target_os = "linux")]
#[test]
fn the_drain_drops_every_queued_frame_and_stops() {
    let (sender, receiver) =
        socket2::Socket::pair(socket2::Domain::UNIX, socket2::Type::DGRAM, None).unwrap();
    receiver.set_nonblocking(true).unwrap();
    assert_eq!(super::transport::drain_queued_frames(&receiver), 0);
    for frame in [&[1u8; 60][..], &[2; 1514], &[3; 17]] {
        sender.send(frame).unwrap();
    }
    assert_eq!(super::transport::drain_queued_frames(&receiver), 3);
    assert_eq!(super::transport::drain_queued_frames(&receiver), 0);
}

/// #1883: the loopback interface, present on every Linux host and
/// container, resolves to its own index with a six-octet address, and the
/// address's protocol leaves the socket's in place on bind.
#[cfg(target_os = "linux")]
#[test]
fn the_loopback_link_resolves_by_index() {
    let link = super::transport::interface_link("lo").unwrap();
    let index = nix::net::if_::if_nametoindex("lo").unwrap();
    assert_eq!(link.ifindex(), index as usize);
    assert_eq!(link.halen(), 6);
    assert_eq!(link.protocol(), 0);
}

/// #1883: starting on a missing interface fails before any socket opens,
/// so it needs no raw-socket permission, and leaves the transport unstarted.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn starting_on_a_missing_interface_fails_cleanly() {
    let mut transport = EthernetTransport::new("bacnet-none0");
    let error = transport.start().await.unwrap_err();
    assert!(error.to_string().contains("bacnet-none0"), "{error}");
    assert_eq!(transport.local_mac(), &[0; 6]);
    assert!(matches!(
        transport.send_broadcast(&[1, 0]).await,
        Err(Error::Transport(e)) if e.kind() == std::io::ErrorKind::NotConnected
    ));
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
    use crate::any::AnyTransport;
    let transport = EthernetTransport::new("unused");
    assert!(transport.supports_local_nonrouter_number_controls());
    let any: AnyTransport = AnyTransport::Ethernet(transport);
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
    use super::ingress::{FrameIngress, classify_frame};
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

/// #1918: a frame goes out whole or the send is an error. A short send is
/// `WriteZero`, an interrupted call is retried, and any other error passes
/// through unchanged.
#[test]
fn a_short_send_is_an_error_and_an_interrupted_one_is_retried() {
    use super::egress::send_whole_frame;
    use std::io::{Error, ErrorKind};
    let frame = [0x5Au8; 60];

    assert!(send_whole_frame(&frame, |f| Ok(f.len())).is_ok());

    let short = send_whole_frame(&frame, |f| Ok(f.len() - 1)).unwrap_err();
    assert_eq!(short.kind(), ErrorKind::WriteZero);
    assert!(short.to_string().contains("59 of 60"), "{short}");
    let nothing = send_whole_frame(&frame, |_| Ok(0)).unwrap_err();
    assert_eq!(nothing.kind(), ErrorKind::WriteZero);

    let mut calls = 0;
    let retried = send_whole_frame(&frame, |f| {
        calls += 1;
        if calls < 3 {
            Err(Error::from(ErrorKind::Interrupted))
        } else {
            Ok(f.len())
        }
    });
    assert!(retried.is_ok());
    assert_eq!(calls, 3);

    let failed = send_whole_frame(&frame, |_| Err(Error::other("no buffer space"))).unwrap_err();
    assert_eq!(failed.to_string(), "no buffer space");
}

/// #1918: an XID or TEST reply that fails to go out, whole or in part, is
/// counted, and its warning is throttled; one that goes out isn't counted.
#[tokio::test(start_paused = true)]
async fn a_failed_llc_reply_is_counted_and_its_warning_throttled() {
    use super::egress::{LlcReply, ReplyFailures};
    use crate::warn_throttle::WARNING_INTERVAL;
    use std::io::{Error, ErrorKind};
    let failures = ReplyFailures::default();
    let local = [2, 0, 0, 0, 0, 1];
    let station = INDIVIDUAL_MACS[0];
    let xid = build_xid_response(&local, &station);
    let test = build_test_response(&local, &station, &WHO_IS);
    let refused = |_: &[u8]| Err(Error::from(ErrorKind::OutOfMemory));

    assert_eq!(
        failures.send_reply(LlcReply::Xid, &station, &xid, |f| Ok(f.len())),
        None
    );
    assert_eq!(failures.count(), 0, "a reply that went out isn't counted");

    assert_eq!(
        failures.send_reply(LlcReply::Xid, &station, &xid, refused),
        Some(1),
        "the first failure warns"
    );
    assert_eq!(
        failures.send_reply(LlcReply::Test, &station, &test, |f| Ok(f.len() / 2)),
        None,
        "a short send fails too, and logs at debug inside the interval"
    );
    assert_eq!(failures.count(), 2);

    tokio::time::advance(WARNING_INTERVAL).await;
    assert_eq!(
        failures.send_reply(LlcReply::Test, &station, &test, refused),
        Some(2),
        "the next warning reports the one held back, and this one"
    );
    assert_eq!(failures.count(), 3);
}

/// #1950: a burst of frames dropped on a full NPDU channel warns once per
/// interval, with the drops since the last warning.
#[tokio::test(start_paused = true)]
async fn the_channel_full_warning_goes_out_at_most_once_per_interval() {
    use super::ingress::npdu_channel_full;
    use crate::warn_throttle::{WARNING_INTERVAL, WarnThrottle};
    let throttle = WarnThrottle::default();
    assert_eq!(npdu_channel_full(&throttle), Some(1));
    for _ in 0..5 {
        assert_eq!(
            npdu_channel_full(&throttle),
            None,
            "debug inside the interval"
        );
    }
    tokio::time::advance(WARNING_INTERVAL).await;
    assert_eq!(npdu_channel_full(&throttle), Some(6));
}
