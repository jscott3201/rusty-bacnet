//! Independent literal values/refusals for the identical Q/R representation.
use bacnet_encoding::npdu::{decode_npdu, NpduAddress, NpduDecodeError};
use bacnet_npdu::{AddressField, Malformed, NetworkMessageType};
use bacnet_types::MacAddr;
use bytes::Bytes;

#[test]
fn routed_proprietary_fields_and_payload_sharing() {
    let data = Bytes::from(vec![
        1, 0xaf, 0, 2, 1, 0x42, 0, 3, 1, 0x24, 7, 0x80, 0x12, 0x34, 0xaa,
    ]);
    let npdu = decode_npdu(data.clone()).unwrap();
    assert!(npdu.expecting_reply);
    assert_eq!(npdu.priority.to_raw(), 3);
    assert_eq!(
        npdu.destination,
        Some(NpduAddress {
            network: 2,
            mac_address: MacAddr::from_slice(&[0x42])
        })
    );
    assert_eq!(
        npdu.source,
        Some(NpduAddress {
            network: 3,
            mac_address: MacAddr::from_slice(&[0x24])
        })
    );
    assert_eq!(npdu.hop_count, 7);
    assert_eq!(npdu.message_type, Some(NetworkMessageType::from_raw(0x80)));
    assert!(npdu.is_network_message());
    assert_eq!(npdu.vendor_id, Some(0x1234));
    assert_eq!(npdu.payload.as_ref(), &[0xaa]);
    assert_eq!(npdu.payload.as_ptr(), data[14..].as_ptr());
}

#[test]
fn independent_first_fault_and_truncation_literals() {
    use Malformed::{EmptySource, ReservedNetwork, Truncated, Version};
    for (frame, fault) in [
        (vec![], Truncated),
        (vec![2], Truncated),
        (vec![2, 0xff], Version),
        (vec![1, 0x20, 0], Truncated),
        (vec![1, 0x20, 0, 0, 255], ReservedNetwork),
        (vec![1, 0x20, 0, 2, 1], Truncated),
        (vec![1, 0x20, 0, 2, 0], Truncated),
        (vec![1, 0x08, 0, 0, 255], ReservedNetwork),
        (vec![1, 0x08, 255, 255, 0], ReservedNetwork),
        (vec![1, 0x08, 0, 3, 0], EmptySource),
        (vec![1, 0x08, 0, 3, 1], Truncated),
        (vec![1, 0x80], Truncated),
        (vec![1, 0x80, 0x80], Truncated),
        (vec![1, 0x80, 0x80, 0x12], Truncated),
    ] {
        assert_eq!(
            decode_npdu(Bytes::from(frame.clone())),
            Err(NpduDecodeError::Malformed(fault)),
            "{frame:?}"
        );
    }
    let routed = [1, 0xaf, 0, 2, 1, 0x42, 0, 3, 1, 0x24, 7, 0x80, 0x12, 0x34];
    for end in 0..routed.len() {
        assert_eq!(
            decode_npdu(Bytes::copy_from_slice(&routed[..end])),
            Err(NpduDecodeError::Malformed(Truncated)),
            "prefix {end}"
        );
    }
    assert!(decode_npdu(Bytes::copy_from_slice(&routed)).is_ok());
}

#[test]
fn independent_overlong_context_precedes_missing_octets() {
    for (frame, field, dnet) in [
        (vec![1, 0x20, 0, 2, 19], AddressField::Destination, Some(2)),
        (vec![1, 0x08, 0, 3, 255], AddressField::Source, None),
        (
            vec![1, 0x28, 0, 2, 0, 0, 3, 19],
            AddressField::Source,
            Some(2),
        ),
    ] {
        let length = *frame.last().unwrap();
        assert_eq!(
            decode_npdu(Bytes::from(frame)),
            Err(NpduDecodeError::AddressTooLong {
                field,
                length,
                dnet,
                source: None
            })
        );
    }
    let mut frame = vec![1, 0x28, 0, 2, 19];
    frame.extend([0x55; 19]);
    frame.extend([0, 3, 1, 0x24]);
    assert_eq!(
        decode_npdu(Bytes::from(frame.clone())),
        Err(NpduDecodeError::AddressTooLong {
            field: AddressField::Destination,
            length: 19,
            dnet: Some(2),
            source: Some(NpduAddress {
                network: 3,
                mac_address: MacAddr::from_slice(&[0x24])
            }),
        })
    );
    for tail in [
        &[0, 0, 1, 0x24][..],
        &[255, 255, 1, 0x24],
        &[0, 3, 0],
        &[0, 3, 19],
        &[0, 3, 1],
    ] {
        frame.truncate(24);
        frame.extend_from_slice(tail);
        assert_eq!(
            decode_npdu(Bytes::from(frame.clone())),
            Err(NpduDecodeError::AddressTooLong {
                field: AddressField::Destination,
                length: 19,
                dnet: Some(2),
                source: None
            })
        );
    }
}
