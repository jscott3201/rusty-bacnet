//! [`encode_npdu`] stages the control information and appends it and the
//! payload (#2334): the octets are the view's encoding of the whole NPDU,
//! the longest control information fits the staging buffer, what the buffer
//! held stays, and a refused NPDU leaves the buffer as it was.

use super::*;

fn address(network: u16, length: usize) -> Option<NpduAddress> {
    Some(NpduAddress {
        network,
        mac_address: MacAddr::from_slice(&vec![0x42; length]),
    })
}

/// The longest control information: both addresses at the bound and a
/// proprietary message type, which carries a vendor ID.
fn longest() -> Npdu {
    Npdu {
        expecting_reply: true,
        priority: NetworkPriority::LifeSafety,
        destination: address(2000, MAX_MAC_LEN),
        source: address(3000, MAX_MAC_LEN),
        hop_count: 7,
        message_type: Some(NetworkMessageType::from_raw(0x80)),
        vendor_id: Some(999),
        payload: Bytes::from_static(&[0xDE, 0xAD, 0xBE, 0xEF]),
    }
}

fn samples() -> Vec<Npdu> {
    vec![
        Npdu::default(),
        Npdu {
            payload: Bytes::from(vec![0x55; 100]),
            ..Npdu::default()
        },
        Npdu {
            destination: address(0xFFFF, 0),
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Npdu::default()
        },
        Npdu {
            source: address(1, 1),
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK),
            payload: Bytes::from_static(&[0x03, 0xE8]),
            ..Npdu::default()
        },
        longest(),
    ]
}

#[test]
fn the_longest_control_information_fits_the_staging_buffer() {
    let npdu = longest();
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    assert_eq!(buf.len(), MAX_HEADER_LEN + npdu.payload.len());
    assert_eq!(decode_npdu(buf.freeze()).unwrap(), npdu);
}

#[test]
fn the_octets_are_the_views_encoding_of_the_whole_npdu() {
    for npdu in samples() {
        let view = npdu.view();
        let mut whole = vec![0; view.encoded_len()];
        view.encode_into(&mut whole).unwrap();

        let mut buf = BytesMut::new();
        encode_npdu(&mut buf, &npdu).unwrap();
        assert_eq!(&buf[..], &whole[..], "{npdu:?}");
    }
}

#[test]
fn encoding_appends_behind_what_the_buffer_holds() {
    for npdu in samples() {
        let mut alone = BytesMut::new();
        encode_npdu(&mut alone, &npdu).unwrap();

        let mut buf = BytesMut::from(&[0xAA, 0xBB, 0xCC][..]);
        encode_npdu(&mut buf, &npdu).unwrap();
        assert_eq!(&buf[..3], [0xAA, 0xBB, 0xCC]);
        assert_eq!(&buf[3..], &alone[..], "{npdu:?}");
    }
}

#[test]
fn a_refused_npdu_leaves_the_buffer_as_it_was() {
    let refused = [
        Npdu {
            destination: address(0, 1),
            ..longest()
        },
        Npdu {
            source: address(0xFFFF, 1),
            ..longest()
        },
        Npdu {
            source: address(3000, 0),
            ..longest()
        },
        Npdu {
            destination: address(2000, MAX_MAC_LEN + 1),
            ..longest()
        },
    ];
    for npdu in refused {
        let mut buf = BytesMut::from(&[0xAA, 0xBB][..]);
        encode_npdu(&mut buf, &npdu).unwrap_err();
        assert_eq!(&buf[..], [0xAA, 0xBB], "{npdu:?}");
    }
}
