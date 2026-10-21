//! Who-Is and I-Am through a sink and back, with no allocator (#1614):
//! hand-written octets, whole-value refusal, the static refusals, and (with
//! alloc) the octets the `BytesMut` primitives write.

use super::*;
use bacnet_types::enums::ObjectType;

/// A Who-Is for 1000 to 2000: `[0]` 1000 and `[1]` 2000, two octets each
/// (Clause 16.10.1.1, tags per Clause 20.2.4).
const WHO_IS: [u8; 6] = [0x0A, 0x03, 0xE8, 0x1A, 0x07, 0xD0];

/// An I-Am from device 1234: its object identifier, max APDU 480, no
/// segmentation (3) and vendor 261, each application-tagged.
const I_AM: [u8; 13] = [
    0xC4, 0x02, 0x00, 0x04, 0xD2, 0x22, 0x01, 0xE0, 0x91, 0x03, 0x22, 0x01, 0x05,
];

fn i_am() -> IAmRequest {
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap(),
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 261,
    }
}

fn who_is() -> WhoIsRequest {
    WhoIsRequest {
        range: Some(DeviceInstanceRange::new(1000, 2000).unwrap()),
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
fn requests_encode_to_their_octets_and_decode_back() {
    let (storage, written) = into_slice(32, |s| who_is().encode_to_sink(s)).unwrap();
    assert_eq!(storage[..written], WHO_IS);
    assert_eq!(WhoIsRequest::decode(&WHO_IS).unwrap(), who_is());

    let (storage, written) = into_slice(32, |s| i_am().encode_to_sink(s)).unwrap();
    assert_eq!(storage[..written], I_AM);
    assert_eq!(IAmRequest::decode(&I_AM).unwrap(), i_am());

    assert_eq!(
        into_slice(0, |s| WhoIsRequest::all().encode_to_sink(s))
            .unwrap()
            .1,
        0
    );
    assert_eq!(WhoIsRequest::decode(&[]).unwrap(), WhoIsRequest::all());
}

#[test]
fn a_request_that_does_not_fit_is_not_written_at_all() {
    for room in 0..WHO_IS.len() {
        assert_eq!(
            into_slice(room, |s| who_is().encode_to_sink(s)),
            Err(SinkFull {
                needed: WHO_IS.len(),
                available: room
            })
        );
    }
    for room in 0..I_AM.len() {
        assert_eq!(
            into_slice(room, |s| i_am().encode_to_sink(s)),
            Err(SinkFull {
                needed: I_AM.len(),
                available: room
            })
        );
    }
}

/// The widest values fill the staging buffers exactly.
#[test]
fn the_widest_requests_fit_their_staging() {
    let widest = IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::from_raw(1023), 4_194_303).unwrap(),
        max_apdu_length: u32::MAX,
        segmentation_supported: Segmentation::from_raw(u8::MAX),
        vendor_id: u16::MAX,
    };
    let (storage, written) = into_slice(32, |s| widest.encode_to_sink(s)).unwrap();
    assert_eq!(written, 15);
    assert_eq!(IAmRequest::decode(&storage[..written]).unwrap(), widest);

    // Limits past the highest instance only come from the wire.
    let wide = [0x0C, 0xFF, 0xFF, 0xFF, 0xFF, 0x1C, 0xFF, 0xFF, 0xFF, 0xFF];
    let decoded = WhoIsRequest::decode(&wide).unwrap();
    let (storage, written) = into_slice(32, |s| decoded.encode_to_sink(s)).unwrap();
    assert_eq!(storage[..written], wide);
}

#[test]
fn refusals_carry_static_labels() {
    let text = |data: &[u8]| WhoIsRequest::decode(data).unwrap_err().to_string();
    assert_eq!(
        text(&[0x09, 0x01]),
        "decoding error at offset 2: WhoIs low limit needs the high limit [1] with it"
    );
    assert_eq!(
        text(&[0x19, 0x0A]),
        "decoding error at offset 0: WhoIs high limit needs the low limit [0] before it"
    );
    assert_eq!(
        text(&[0x09, 0x0A, 0x19, 0x01]),
        "decoding error at offset 0: WhoIs low limit exceeds high limit"
    );
    assert_eq!(
        text(&[0x09, 0x01, 0x19, 0x0A, 0x29, 0x00]),
        "decoding error at offset 4: WhoIs: 2 trailing byte(s)"
    );
    assert!(matches!(
        WhoIsRequest::decode(&[0x09, 0x01]),
        Err(Error::Decoding(DecodeError {
            kind: DecodingKind::Missing,
            ..
        }))
    ));
    assert_eq!(
        IAmRequest::decode(&I_AM[..12]).unwrap_err().to_string(),
        "buffer too short: need 13 bytes, have 12"
    );
    assert_eq!(
        DeviceInstanceRange::new(20, 10).unwrap_err().to_string(),
        "value out of range: device instance range low limit 20 is above its high limit 10, \
         so it would match no device"
    );
    assert_eq!(
        DeviceInstanceRange::from_limits(Some(3), None)
            .unwrap_err()
            .to_string(),
        "value out of range: a device instance range needs both limits or neither: \
         low limit 3 was given without a high limit"
    );
}

/// The sink encoders write what the `BytesMut` primitives wrote for each
/// member before they did.
#[cfg(feature = "alloc")]
#[test]
fn the_sink_encoders_write_the_hosted_octets() {
    let range = |low, high| WhoIsRequest {
        range: Some(DeviceInstanceRange::new(low, high).unwrap()),
    };
    for request in [
        WhoIsRequest::all(),
        range(0, 0),
        range(255, 256),
        range(65_535, 65_536),
        range(1, 4_194_303),
    ] {
        let mut hosted = BytesMut::new();
        if let Some(range) = request.range {
            primitives::encode_ctx_unsigned(&mut hosted, 0, range.low().into());
            primitives::encode_ctx_unsigned(&mut hosted, 1, range.high().into());
        }
        let mut sink = BytesMut::new();
        request.encode(&mut sink);
        assert_eq!(sink, hosted, "{request:?}");
    }
    for (instance, max_apdu, segmentation, vendor) in [
        (0, 50, 0, 0),
        (1234, 480, 3, 261),
        (4_194_303, u32::MAX, 255, u16::MAX),
        (70_000, 65_536, 1, 256),
    ] {
        let request = IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
            max_apdu_length: max_apdu,
            segmentation_supported: Segmentation::from_raw(segmentation),
            vendor_id: vendor,
        };
        let mut hosted = BytesMut::new();
        primitives::encode_app_object_id(&mut hosted, &request.object_identifier);
        primitives::encode_app_unsigned(&mut hosted, request.max_apdu_length.into());
        primitives::encode_app_enumerated(&mut hosted, segmentation.into());
        primitives::encode_app_unsigned(&mut hosted, request.vendor_id.into());
        let mut sink = BytesMut::new();
        request.encode(&mut sink);
        assert_eq!(sink, hosted, "{request:?}");
    }
}
