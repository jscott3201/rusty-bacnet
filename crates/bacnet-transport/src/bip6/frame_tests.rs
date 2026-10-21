//! The transport's B/IPv6 encoding and decoding, through the codec: wire
//! layouts and rejections.

use std::net::Ipv6Addr;

use bytes::BytesMut;

use super::frame::{encoded, raw_frame};
use super::*;

fn vmac(octets: [u8; 3]) -> Bip6Vmac {
    Bip6Vmac::new(octets)
}

/// `message` on the wire.
fn wire(message: Bvll6<'_>) -> BytesMut {
    encoded(&message).expect("a valid B/IPv6 message")
}

#[test]
fn encode_original_unicast() {
    let src_vmac = vmac([0x01, 0x02, 0x03]);
    let dst_vmac = vmac([0x0A, 0x0B, 0x0C]);
    let npdu = vec![0x01, 0x00, 0xAA];
    let mut buf = BytesMut::new();
    encode_bvll6(
        &mut buf,
        &Bvll6::OriginalUnicastNpdu {
            source: src_vmac,
            destination: dst_vmac,
            npdu: &npdu,
        },
    )
    .expect("valid BVLC6 unicast encoding");
    assert_eq!(buf[0], BVLC_TYPE_BIP6);
    assert_eq!(buf[1], Bvlc6Function::ORIGINAL_UNICAST_NPDU.to_raw());
    let len = u16::from_be_bytes([buf[2], buf[3]]);
    assert_eq!(len as usize, HEADER_LENGTH + 2 * VMAC_LENGTH + npdu.len());
    assert_eq!(buf[4..7], src_vmac.octets());
    assert_eq!(buf[7..10], dst_vmac.octets());
    assert_eq!(&buf[10..], &npdu[..]);
}

#[test]
fn encode_original_broadcast() {
    let source = vmac([0x01; 3]);
    let npdu = vec![0xBB];
    let mut buf = BytesMut::new();
    encode_bvll6(
        &mut buf,
        &Bvll6::OriginalBroadcastNpdu {
            source,
            npdu: &npdu,
        },
    )
    .expect("valid BVLC6 broadcast encoding");
    assert_eq!(buf[1], Bvlc6Function::ORIGINAL_BROADCAST_NPDU.to_raw());
}

#[test]
fn encode_bvlc6_oversized_payload_errors() {
    let source = vmac([0x01; 3]);
    let npdu = vec![0; u16::MAX as usize - (HEADER_LENGTH + VMAC_LENGTH) + 1];
    let mut buf = BytesMut::new();
    assert!(encode_bvll6(
        &mut buf,
        &Bvll6::OriginalBroadcastNpdu {
            source,
            npdu: &npdu
        }
    )
    .is_err());
    assert!(buf.is_empty());
}

#[test]
fn encode_bvlc6_unicast_oversized_payload_errors() {
    let npdu = vec![0; u16::MAX as usize - (HEADER_LENGTH + 2 * VMAC_LENGTH) + 1];
    let mut buf = BytesMut::new();
    let message = Bvll6::OriginalUnicastNpdu {
        source: vmac([0x01, 0x02, 0x03]),
        destination: vmac([0x0A, 0x0B, 0x0C]),
        npdu: &npdu,
    };
    assert!(encode_bvll6(&mut buf, &message).is_err());
}

#[test]
fn decode_round_trip_unicast() {
    let src_vmac = vmac([0x01, 0x02, 0x03]);
    let dst_vmac = vmac([0x0A, 0x0B, 0x0C]);
    let npdu = vec![0x01, 0x00, 0xAA, 0xBB];
    let buf = wire(Bvll6::OriginalUnicastNpdu {
        source: src_vmac,
        destination: dst_vmac,
        npdu: &npdu,
    });
    let decoded = decode_bvll6(&buf).unwrap();
    assert_eq!(decoded.function(), Bvlc6Function::ORIGINAL_UNICAST_NPDU);
    assert_eq!(decoded.source_vmac(), src_vmac);
    assert_eq!(decoded.destination_vmac(), Some(dst_vmac));
    assert_eq!(decoded.npdu(), Some(&npdu[..]));
}

#[test]
fn decode_rejects_short_frame() {
    assert!(decode_bvll6(&[0x82, 0x01]).is_err());
}

#[test]
fn decode_rejects_wrong_type() {
    assert!(decode_bvll6(&[0x81, 0x01, 0x00, 0x07, 0, 0, 0]).is_err());
}

#[test]
fn function_round_trip() {
    for byte in 0x00..=0x0Cu8 {
        let f = Bvlc6Function::from_raw(byte);
        assert_eq!(f.to_raw(), byte);
    }
}

#[test]
fn encode_decode_virtual_address_resolution() {
    let source = vmac([0xAA, 0xBB, 0xCC]);
    let buf = wire(Bvll6::VirtualAddressResolution { source });

    // VAR is 7 bytes: type(1) + function(1) + length(2) + source_vmac(3)
    assert_eq!(buf.len(), 7);
    assert_eq!(buf[0], BVLC_TYPE_BIP6);
    assert_eq!(buf[1], Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION.to_raw());
    let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    assert_eq!(total_len, 7);
    assert_eq!(buf[4..7], source.octets());

    let frame = decode_bvll6(&buf).unwrap();
    assert_eq!(frame, Bvll6::VirtualAddressResolution { source });
    assert_eq!(frame.npdu(), None);
}

#[test]
fn encode_decode_virtual_address_resolution_ack() {
    let source = vmac([0x11, 0x22, 0x33]);
    let dest = vmac([0x44, 0x55, 0x66]);
    let buf = wire(Bvll6::VirtualAddressResolutionAck {
        source,
        destination: dest,
    });

    // VAR-ACK is 10 bytes: type(1)+function(1)+length(2)+source(3)+dest(3)
    assert_eq!(buf.len(), 10);
    assert_eq!(buf[0], BVLC_TYPE_BIP6);
    assert_eq!(
        buf[1],
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK.to_raw()
    );
    let total_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    assert_eq!(total_len, 10);
    assert_eq!(buf[4..7], source.octets());
    assert_eq!(buf[7..10], dest.octets());

    let frame = decode_bvll6(&buf).unwrap();
    assert_eq!(
        frame.function(),
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK
    );
    assert_eq!(frame.source_vmac(), source);
    assert_eq!(frame.destination_vmac(), Some(dest));
    assert_eq!(frame.npdu(), None);
}

#[test]
fn encode_decode_address_resolution() {
    let source = vmac([0x01, 0x02, 0x03]);
    let target = vmac([0x04, 0x05, 0x06]);
    let buf = wire(Bvll6::AddressResolution { source, target });

    assert_eq!(buf.len(), 10);
    let frame = decode_bvll6(&buf).unwrap();
    assert_eq!(frame, Bvll6::AddressResolution { source, target });
}

#[test]
fn encode_decode_address_resolution_ack() {
    let source = vmac([0x0A, 0x0B, 0x0C]);
    let dest = vmac([0x0D, 0x0E, 0x0F]);
    let buf = wire(Bvll6::AddressResolutionAck {
        source,
        destination: dest,
    });

    assert_eq!(buf.len(), 10);
    let frame = decode_bvll6(&buf).unwrap();
    assert_eq!(frame.function(), Bvlc6Function::ADDRESS_RESOLUTION_ACK);
    assert_eq!(frame.source_vmac(), source);
    assert_eq!(frame.destination_vmac(), Some(dest));
}

#[test]
fn fixed_length_bvlc6_frames_reject_surplus_bytes() {
    let mut var = wire(Bvll6::VirtualAddressResolution {
        source: vmac([1, 2, 3]),
    });
    var.extend_from_slice(&[0]);
    assert!(decode_bvll6(&var).is_err());

    let mut ar = wire(Bvll6::AddressResolution {
        source: vmac([1, 2, 3]),
        target: vmac([4, 5, 6]),
    });
    ar.extend_from_slice(&[0]);
    let ar_len = ar.len() as u16;
    ar[2..4].copy_from_slice(&ar_len.to_be_bytes());
    assert!(decode_bvll6(&ar).is_err());

    let source = vmac([1, 2, 3]);
    let entry = Bip6Address::new(Ipv6Addr::UNSPECIFIED, 0);
    for message in [
        Bvll6::BvlcResult {
            source,
            code: Bvlc6ResultCode::SUCCESSFUL_COMPLETION,
        },
        Bvll6::RegisterForeignDevice { source, ttl: 0 },
        Bvll6::DeleteForeignDeviceTableEntry { source, entry },
    ] {
        let mut frame = wire(message);
        assert!(decode_bvll6(&frame).is_ok());
        frame.extend_from_slice(&[0]);
        let length = frame.len() as u16;
        frame[2..4].copy_from_slice(&length.to_be_bytes());
        assert!(decode_bvll6(&frame).is_err(), "{message:?}");
    }
}

#[test]
fn bvlc6_declared_length_must_match_datagram() {
    let mut frame = wire(Bvll6::VirtualAddressResolution {
        source: vmac([1, 2, 3]),
    });
    frame[2..4].copy_from_slice(&6u16.to_be_bytes());
    assert!(decode_bvll6(&frame).is_err());
}

/// A Forwarded-NPDU from VMAC AA BB CC whose payload after the VMAC is
/// `payload`, whatever its length.
fn forwarded_payload(payload: &[u8]) -> BytesMut {
    let mut fields = vec![0xAA, 0xBB, 0xCC];
    fields.extend_from_slice(payload);
    raw_frame(Bvlc6Function::FORWARDED_NPDU.to_raw(), &fields)
}

#[test]
fn decode_forwarded_npdu_extracts_npdu() {
    let source_ip = Ipv6Addr::LOCALHOST;
    let source_port: u16 = 47808;
    let npdu_data = vec![0x01, 0x00, 0xFF, 0xEE];
    let mut payload = source_ip.octets().to_vec();
    payload.extend_from_slice(&source_port.to_be_bytes());
    payload.extend_from_slice(&npdu_data);

    let frame = forwarded_payload(&payload);
    let message = decode_bvll6(&frame).unwrap();
    let addr = message.origin().unwrap();
    assert_eq!(addr.ip, source_ip);
    assert_eq!(addr.port, source_port);
    assert_eq!(message.npdu(), Some(&npdu_data[..]));
}

/// Without a whole 18-octet address the codec refuses the frame. An address
/// with a 0- or 1-octet NPDU decodes, and the receive path drops it
/// (`group_delivery_tests::a_forwarded_npdu_too_short_for_an_npdu_is_dropped_uncounted`).
#[test]
fn decode_forwarded_npdu_needs_a_whole_address() {
    assert!(decode_bvll6(&forwarded_payload(&[0x01; 17])).is_err());
    assert!(decode_bvll6(&forwarded_payload(&[])).is_err());
    for npdu in [0, 1] {
        let frame = forwarded_payload(&vec![0x01; 18 + npdu]);
        let message = decode_bvll6(&frame).unwrap();
        assert_eq!(message.npdu().map(<[u8]>::len), Some(npdu));
    }
}

#[test]
fn forwarded_npdu_encode_decode_round_trip() {
    // Build a full ForwardedNpdu BVLC6 frame and decode it
    let originating_vmac = vmac([0xAA, 0xBB, 0xCC]);
    let source_ip = Ipv6Addr::LOCALHOST;
    let npdu = vec![0x01, 0x00, 0xDE, 0xAD];

    let buf = wire(Bvll6::ForwardedNpdu {
        original_source: originating_vmac,
        original_address: Bip6Address::new(source_ip, 47808),
        npdu: &npdu,
    });

    let frame = decode_bvll6(&buf).unwrap();
    assert_eq!(frame.function(), Bvlc6Function::FORWARDED_NPDU);
    assert_eq!(frame.source_vmac(), originating_vmac);
    assert_eq!(frame.origin().unwrap().ip, source_ip);
    assert_eq!(frame.npdu(), Some(&npdu[..]));
}

/// The function codes the transport re-exports name Annex U's twelve, with
/// the deleted 0x0B unknown and 0x0C Distribute-Broadcast-To-Network.
#[test]
fn bvlc6_function_codes_are_the_codecs() {
    let expected: &[(u8, &str)] = &[
        (0x00, "BVLC_RESULT"),
        (0x01, "ORIGINAL_UNICAST_NPDU"),
        (0x02, "ORIGINAL_BROADCAST_NPDU"),
        (0x03, "ADDRESS_RESOLUTION"),
        (0x04, "FORWARDED_ADDRESS_RESOLUTION"),
        (0x05, "ADDRESS_RESOLUTION_ACK"),
        (0x06, "VIRTUAL_ADDRESS_RESOLUTION"),
        (0x07, "VIRTUAL_ADDRESS_RESOLUTION_ACK"),
        (0x08, "FORWARDED_NPDU"),
        (0x09, "REGISTER_FOREIGN_DEVICE"),
        (0x0A, "DELETE_FOREIGN_DEVICE_TABLE_ENTRY"),
        (0x0C, "DISTRIBUTE_BROADCAST_TO_NETWORK"),
    ];
    for &(byte, name) in expected {
        let function = Bvlc6Function::from_raw(byte);
        assert_eq!(function.name(), Some(name), "0x{byte:02X}");
    }

    // Verify 0x0C is Distribute-Broadcast-To-Network (not the old SECURE_BVLL)
    assert_eq!(
        Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK.to_raw(),
        0x0C
    );
    // 0x0B is unknown since it's removed
    assert!(!Bvlc6Function::from_raw(0x0B).is_defined());
    assert!(matches!(
        decode_bvll6(&raw_frame(0x0B, &[1, 2, 3])),
        Err(bacnet_types::error::Error::Decoding(_))
    ));
}
