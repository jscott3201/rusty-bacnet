//! The transport's BVLL adapter: owned frames, encoding into `BytesMut`, MAC
//! helpers and the codec error mapping. The codec's own vectors live in
//! `bacnet-bip-codec`.

use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, Bvll, BvllError};
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::*;

fn encode(message: &Bvll<'_>) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, message).expect("valid BVLL encoding");
    buf
}

#[test]
fn owned_frame_reads_back_through_the_codec() {
    // A Who-Is broadcast NPDU (Original-Broadcast-NPDU, J.2.12).
    let npdu = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
    let wire = encode(&Bvll::OriginalBroadcastNpdu(&npdu));
    assert_eq!(&wire[..4], &[0x81, 0x0B, 0x00, 0x0C]);

    let message = decode_bvll(&wire).unwrap();
    assert_eq!(message.function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
    assert_eq!(message.payload, npdu[..]);
    assert_eq!(message.npdu().unwrap(), npdu[..]);
    assert_eq!(message.origin(), None);
    assert_eq!(
        message.message().unwrap(),
        Bvll::OriginalBroadcastNpdu(&npdu)
    );
}

#[test]
fn forwarded_npdu_keeps_its_origin_apart_from_the_npdu() {
    let origin = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 100), 0xBAC0);
    let npdu = [0x01, 0x00, 0x55];
    let wire = encode(&Bvll::ForwardedNpdu {
        origin,
        npdu: &npdu,
    });
    assert_eq!(&wire[4..10], &[192, 168, 1, 100, 0xBA, 0xC0]);

    let message = decode_bvll(&wire).unwrap();
    assert_eq!(message.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(message.payload.len(), 6 + npdu.len());
    assert_eq!(message.npdu().unwrap(), npdu[..]);
    assert_eq!(message.origin(), Some(origin));
}

#[test]
fn management_payloads_are_read_only_on_request() {
    // A Register-Foreign-Device with a 3-octet TTL is a frame, so the
    // transport can still NAK it, but not a message.
    let wire = raw_frame(BvlcFunction::REGISTER_FOREIGN_DEVICE, &[0, 60, 0]);
    let message = decode_bvll(&wire).unwrap();
    assert_eq!(message.function, BvlcFunction::REGISTER_FOREIGN_DEVICE);
    assert!(matches!(
        message.message(),
        Err(Error::Decoding(DecodeError { offset: 4, .. }))
    ));
    assert_eq!(message.npdu(), None);
}

#[test]
fn datagram_with_octets_past_its_length_is_refused() {
    // Annex J.2: the Length field counts the whole message. The decoder
    // before bacnet-bip-codec ignored the trailing octets and read the
    // shorter message; now the datagram is refused.
    let err = decode_bvll(&[0x81, 0x0A, 0x00, 0x06, 0x01, 0x00, 0xDE, 0xAD]).unwrap_err();
    assert!(
        matches!(&err, Error::Decoding(refused @ DecodeError { offset: 2, .. })
            if refused.message().to_string().contains("length 6")
                && refused.message().to_string().contains("8-octet")),
        "{err}"
    );
}

#[test]
fn header_errors_map_to_decoding_errors() {
    for (wire, offset) in [
        (&[0x81, 0x0A][..], 0),
        (&[0x82, 0x0A, 0x00, 0x04][..], 0),
        (&[0x81, 0x0A, 0x00, 0x64][..], 2),
        (&[0x81, 0x0A, 0x00, 0x03][..], 2),
    ] {
        match decode_bvll(wire) {
            Err(Error::Decoding(refused)) => assert_eq!(refused.offset, offset, "{wire:02X?}"),
            other => panic!("{wire:02X?}: {other:?}"),
        }
    }
    // A Forwarded-NPDU too short for its originating address.
    let short = raw_frame(BvlcFunction::FORWARDED_NPDU, &[192, 0, 2]);
    assert!(decode_bvll(&short).unwrap().message().is_err());
}

#[test]
fn error_mapping_covers_every_codec_error() {
    let decoding = [
        (BvllError::TooShort { length: 2 }, 0),
        (BvllError::NotBip { bvlc_type: 0x82 }, 0),
        (BvllError::UnknownFunction { function: 0x0C }, 1),
        (
            BvllError::LengthMismatch {
                declared: 6,
                received: 8,
            },
            2,
        ),
        (
            BvllError::PayloadLength {
                function: BvlcFunction::BVLC_RESULT,
                length: 3,
            },
            4,
        ),
        (
            BvllError::TableLength {
                length: 9,
                entry_length: 10,
            },
            0,
        ),
        (BvllError::AddressLength { length: 3 }, 0),
    ];
    let texts = [
        "BVLL datagram too short: 2 octets, header needs 4",
        "BVLC type 0x82 is not B/IP (0x81)",
        "unknown BVLC function 0x0C",
        "BVLC length 6 does not match the 8-octet datagram",
        "BVLC_RESULT cannot carry a 3-octet payload",
        "table of 9 octets is not a whole number of 10-octet entries",
        "B/IP address must be 6 octets, got 3",
    ];
    for ((error, offset), text) in decoding.into_iter().zip(texts) {
        match bvll_error(error) {
            Error::Decoding(refused) => {
                assert_eq!(refused.offset, offset, "{error:?}");
                assert_eq!(refused.message().to_string(), text, "{error:?}");
            }
            other => panic!("{error:?} mapped to {other:?}"),
        }
        // The hex octets read as the codec's own text does (#1803).
        if matches!(
            error,
            BvllError::NotBip { .. } | BvllError::UnknownFunction { .. }
        ) {
            assert_eq!(text, error.to_string());
        }
    }
    let encoding = [
        BvllError::TooLong { length: 65536 },
        BvllError::UndefinedResultCode { code: 0x70 },
        BvllError::BufferExhausted {
            needed: 6,
            available: 0,
        },
    ];
    for error in encoding {
        assert!(
            matches!(bvll_error(error), Error::Encoding(ref message) if *message == error.to_string()),
            "{error:?}"
        );
    }
}

#[test]
fn encode_refusals_leave_the_buffer_alone() {
    let mut buf = BytesMut::from(&b"keep"[..]);
    let npdu = vec![0; u16::MAX as usize - 3];
    assert!(matches!(
        encode_bvll(&mut buf, &Bvll::OriginalUnicastNpdu(&npdu)),
        Err(Error::Encoding(_))
    ));
    assert!(matches!(
        encode_bvll(
            &mut buf,
            &Bvll::BvlcResult(BvlcResultCode::from_raw(0x0070))
        ),
        Err(Error::Encoding(_))
    ));
    assert_eq!(&buf[..], b"keep");

    // The largest NPDU that fits the Length field encodes.
    encode_bvll(&mut buf, &Bvll::OriginalUnicastNpdu(&npdu[1..])).unwrap();
    assert_eq!(buf.len(), 4 + u16::MAX as usize);
}

#[test]
fn table_messages_encode_from_typed_rows() {
    let rows = [bacnet_bip_codec::BdtEntry::new(
        SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 0xBAC0),
        Ipv4Addr::new(255, 255, 255, 0),
    )];
    let wire = encode(&Bvll::WriteBroadcastDistributionTable(
        BdtEntries::from_entries(&rows),
    ));
    assert_eq!(
        &wire[..],
        &[
            0x81, 0x01, 0x00, 0x0E, 192, 0, 2, 10, 0xBA, 0xC0, 255, 255, 255, 0
        ]
    );
}

#[test]
fn bip_mac_round_trip() {
    let mac = encode_bip_mac([10, 0, 1, 42], 0xBAC0);
    assert_eq!(mac, [10, 0, 1, 42, 0xBA, 0xC0]);
    assert_eq!(decode_bip_mac(&mac).unwrap(), ([10, 0, 1, 42], 0xBAC0));
}

#[test]
fn bip_mac_invalid_length() {
    for mac in [&[1, 2, 3][..], &[], &[0; 7]] {
        let err = decode_bip_mac(mac).unwrap_err();
        assert!(
            matches!(&err, Error::Decoding(refused @ DecodeError { offset: 0, .. })
                if refused.message().to_string().contains("must be 6 octets")),
            "{err}"
        );
    }
}
