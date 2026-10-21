//! ReadProperty request and ACK through a sink and back, with no allocator
//! (#1614): hand-written octets, whole-value refusal, the borrowed value, and
//! (with alloc) the octets the `BytesMut` primitives write.

use super::*;
use bacnet_types::enums::ObjectType;

/// `[0]` analog input 1 and `[1]` Present_Value (85) (Clause 15.5.1.1, tags
/// per Clause 20.2.14 and 20.2.4).
const REQUEST: [u8; 7] = [0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55];

/// The request's members, then `[3]` around an application-tagged REAL 72.5.
const ACK: [u8; 14] = [
    0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44, 0x42, 0x91, 0x00, 0x00, 0x3F,
];

fn request(index: Option<u32>) -> ReadPropertyRequest {
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: index,
    }
}

fn ack(value: &[u8]) -> ReadPropertyAckRef<'_> {
    ReadPropertyAckRef {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value,
    }
}

/// `encode` into a slice of `room` octets: the octets written, or the
/// refusal with the slice and its storage checked unchanged.
fn into_slice(
    room: usize,
    encode: impl FnOnce(&mut &mut [u8]) -> Result<usize, SinkFull>,
) -> Result<([u8; 32], usize), SinkFull> {
    let mut storage = [0xA5; 32];
    let mut tail = &mut storage[..room];
    match encode(&mut tail) {
        Ok(written) => {
            assert_eq!(tail.len(), room - written);
            Ok((storage, written))
        }
        Err(refused) => {
            assert_eq!(tail.len(), room);
            assert_eq!(storage, [0xA5; 32]);
            Err(refused)
        }
    }
}

#[test]
fn the_request_and_ack_encode_to_their_octets_and_decode_back() {
    let (storage, written) = into_slice(32, |s| request(None).encode_to_sink(s)).unwrap();
    assert_eq!(storage[..written], REQUEST);
    assert_eq!(
        ReadPropertyRequest::decode(&REQUEST).unwrap(),
        request(None)
    );

    let (storage, written) = into_slice(32, |s| request(Some(8)).encode_to_sink(s)).unwrap();
    assert_eq!(
        storage[..written],
        [&REQUEST[..], &[0x29, 0x08]].concat()[..]
    );

    let value = &ACK[8..13];
    let (storage, written) = into_slice(32, |s| ack(value).encode_to_sink(s)).unwrap();
    assert_eq!(storage[..written], ACK);
    let decoded = ReadPropertyAckRef::decode(&ACK).unwrap();
    assert_eq!(decoded, ack(value));
    // The value borrows the ACK's octets.
    assert!(core::ptr::eq(decoded.property_value, &ACK[8..13]));
}

#[test]
fn a_value_that_does_not_fit_is_not_written_at_all() {
    for room in 0..REQUEST.len() {
        assert_eq!(
            into_slice(room, |s| request(None).encode_to_sink(s)),
            Err(SinkFull {
                needed: REQUEST.len(),
                available: room
            })
        );
    }
    let value = &ACK[8..13];
    for room in 0..ACK.len() {
        assert_eq!(
            into_slice(room, |s| ack(value).encode_to_sink(s)),
            Err(SinkFull {
                needed: ACK.len(),
                available: room
            })
        );
    }
}

/// The widest members fill the staging buffers exactly.
#[test]
fn the_widest_members_fit_their_staging() {
    let widest = ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::from_raw(1023), 4_194_303).unwrap(),
        property_identifier: PropertyIdentifier::from_raw(u32::MAX),
        property_array_index: Some(u32::MAX),
    };
    let (storage, written) = into_slice(32, |s| widest.encode_to_sink(s)).unwrap();
    assert_eq!(written, 15);
    assert_eq!(
        ReadPropertyRequest::decode(&storage[..written]).unwrap(),
        widest
    );

    let ack = ReadPropertyAckRef {
        object_identifier: widest.object_identifier,
        property_identifier: widest.property_identifier,
        property_array_index: widest.property_array_index,
        property_value: &[0x00],
    };
    let (storage, written) = into_slice(32, |s| ack.encode_to_sink(s)).unwrap();
    assert_eq!(written, 18);
    assert_eq!(
        ReadPropertyAckRef::decode(&storage[..written]).unwrap(),
        ack
    );
}

#[test]
fn malformed_acks_are_refused() {
    let text = |data: &[u8]| ReadPropertyAckRef::decode(data).unwrap_err().to_string();
    assert_eq!(
        text(&REQUEST),
        "decoding error at offset 7: tag decode: offset beyond buffer length"
    );
    assert_eq!(
        text(&[&REQUEST[..], &[0x39, 0x01]].concat()),
        "decoding error at offset 7: ReadPropertyACK property-value: expected opening tag [3]"
    );
    assert_eq!(
        text(&[&ACK[..], &[0x00]].concat()),
        "decoding error at offset 14: ReadPropertyACK has trailing data"
    );
    assert_eq!(
        text(&ACK[..13]),
        "decoding error at offset 8: missing closing tag 3"
    );
}

/// The sink encoders write what the `BytesMut` primitives wrote for each
/// member before they did, and the owned ACK reads what the borrowed one
/// does.
#[cfg(feature = "alloc")]
#[test]
fn the_sink_encoders_write_the_hosted_octets() {
    use bytes::BytesMut;

    for index in [None, Some(0), Some(255), Some(256), Some(u32::MAX)] {
        let request = request(index);
        let mut hosted = BytesMut::new();
        primitives::encode_ctx_object_id(&mut hosted, 0, &request.object_identifier);
        primitives::encode_ctx_unsigned(
            &mut hosted,
            1,
            request.property_identifier.to_raw().into(),
        );
        if let Some(index) = index {
            primitives::encode_ctx_unsigned(&mut hosted, 2, index.into());
        }
        let mut sink = BytesMut::new();
        request.encode(&mut sink);
        assert_eq!(sink, hosted, "{request:?}");

        let value = [0x75, 0x05, 0x00, b't', b'e', b'x', b't'];
        let owned = ReadPropertyACK {
            object_identifier: request.object_identifier,
            property_identifier: request.property_identifier,
            property_array_index: index,
            property_value: value.to_vec(),
        };
        tags::encode_opening_tag(&mut hosted, 3);
        hosted.extend_from_slice(&value);
        tags::encode_closing_tag(&mut hosted, 3);
        let mut sink = BytesMut::new();
        owned.encode(&mut sink);
        assert_eq!(sink, hosted, "{owned:?}");
        assert_eq!(ReadPropertyACK::decode(&sink).unwrap(), owned);
        assert_eq!(
            ReadPropertyAckRef::decode(&sink).unwrap(),
            owned.as_borrowed()
        );
    }
}
