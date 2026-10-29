//! The allocation-free subset allocates nothing (#1614, #1646).
//!
//! A device without a heap encodes and decodes Who-Is, I-Am and ReadProperty
//! (request and ACK) into and out of its own buffers, frames them with the
//! APDU header, and reads the ACK's value in place. This runs that exchange
//! under a counting global allocator, one per test binary, in every profile:
//! a run without `alloc` shows the API compiles without it, and the count
//! shows the calls make no allocation, the refusals included, whose static
//! labels are formatted only when displayed.

use bacnet_encoding::apdu::header::{ApduHeader, ConfirmedRequestHeader, MaxApdu, MaxSegments};
use bacnet_encoding::primitives::{
    ApplicationValueRef, decode_application_value_ref, encode_app_character_string_to_sink,
    encode_app_real_to_sink,
};
use bacnet_encoding::sink::EncodeSink;
use bacnet_services::read_property::{ReadPropertyAckRef, ReadPropertyRequest};
use bacnet_services::who_is::{DeviceInstanceRange, IAmRequest, WhoIsRequest};
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, PropertyIdentifier, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::error::{DecodingKind, Error};
use bacnet_types::primitives::ObjectIdentifier;

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
use counting_alloc::counted;

const READ_PROPERTY: ConfirmedServiceChoice = ConfirmedServiceChoice::READ_PROPERTY;

/// A client's Who-Is, a device's I-Am, a ReadProperty for its name and the
/// ACK, each framed in an APDU in a fixed buffer and read back.
fn exchange() -> Result<(), Error> {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 1234)?;
    let mut storage = [0; 64];

    // Who-Is for device 1234 alone, unconfirmed.
    let who_is = WhoIsRequest {
        range: Some(DeviceInstanceRange::single(1234)?),
    };
    let mut parameters = [0; 16];
    let mut tail = &mut parameters[..];
    let length = who_is.encode_to_sink(&mut tail).unwrap();
    let mut out = &mut storage[..];
    let header = ApduHeader::UnconfirmedRequest(UnconfirmedServiceChoice::WHO_IS);
    let written = header
        .encode_with_parameters_to_sink(&mut out, &parameters[..length])
        .unwrap();
    let (read, rest) = ApduHeader::decode(&storage[..written])?;
    assert_eq!(read, header);
    assert_eq!(WhoIsRequest::decode(rest)?, who_is);

    // The device's I-Am.
    let i_am = IAmRequest {
        object_identifier: device,
        max_apdu_length: 480,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 261,
    };
    let mut out = &mut storage[..];
    let written = ApduHeader::UnconfirmedRequest(UnconfirmedServiceChoice::I_AM)
        .encode_to_sink(&mut out)
        .unwrap()
        + i_am.encode_to_sink(&mut out).unwrap();
    let (_, rest) = ApduHeader::decode(&storage[..written])?;
    assert_eq!(IAmRequest::decode(rest)?, i_am);

    // A ReadProperty for its Object_Name, confirmed.
    let request = ReadPropertyRequest {
        object_identifier: device,
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    };
    let header = ApduHeader::ConfirmedRequest(ConfirmedRequestHeader {
        segmented_response_accepted: false,
        max_segments: MaxSegments::UNSPECIFIED,
        max_apdu: MaxApdu::OCTETS_480,
        invoke_id: 9,
        service_choice: READ_PROPERTY,
    });
    let mut out = &mut storage[..];
    let written =
        header.encode_to_sink(&mut out).unwrap() + request.encode_to_sink(&mut out).unwrap();
    let (read, rest) = ApduHeader::decode(&storage[..written])?;
    let ApduHeader::ConfirmedRequest(read) = read else {
        panic!("a Confirmed-Request");
    };
    assert_eq!(ReadPropertyRequest::decode(rest)?, request);

    // The ACK, its value a CharacterString written into a buffer of its own.
    let mut value = [0; 16];
    let mut tail = &mut value[..];
    let value_len = encode_app_character_string_to_sink(&mut tail, "AHU-1").unwrap();
    let ack = ReadPropertyAckRef {
        object_identifier: device,
        property_identifier: request.property_identifier,
        property_array_index: None,
        property_value: &value[..value_len],
    };
    let mut out = &mut storage[..];
    let written = ApduHeader::ComplexAck {
        invoke_id: read.invoke_id,
        service_choice: read.service_choice,
    }
    .encode_to_sink(&mut out)
    .unwrap()
        + ack.encode_to_sink(&mut out).unwrap();
    let (_, rest) = ApduHeader::decode(&storage[..written])?;
    let read = ReadPropertyAckRef::decode(rest)?;
    assert_eq!(read, ack);
    let (name, _) = decode_application_value_ref(read.property_value, 0)?;
    let ApplicationValueRef::CharacterString(name) = name else {
        panic!("a CharacterString");
    };
    assert_eq!(name.as_str(), Some("AHU-1"));
    Ok(())
}

#[test]
fn the_exchange_allocates_nothing() {
    let (result, allocations) = counted(exchange);
    result.unwrap();
    assert_eq!(allocations, 0);
}

/// The refusals allocate nothing either: their labels are static.
#[test]
fn refusals_allocate_nothing() {
    let (kinds, allocations) = counted(|| {
        let mut kinds = [None; 6];
        let refusals = [
            WhoIsRequest::decode(&[0x09, 0x01]).err(),
            IAmRequest::decode(&[0xC4, 0x02, 0x00, 0x04]).err(),
            ReadPropertyRequest::decode(&[0x0C, 0, 0, 0, 1, 0x19, 0x55, 0x21, 0x00]).err(),
            ReadPropertyAckRef::decode(&[0x0C, 0, 0, 0, 1, 0x19, 0x55, 0x3E, 0x00]).err(),
            ApduHeader::decode(&[0x08, 0x05, 0x01, 0x00, 0x01, 0x0C]).err(),
            decode_application_value_ref(&[0x73, 0x00, 0xC3, 0x28], 0).err(),
        ];
        for (kind, refused) in kinds.iter_mut().zip(refusals) {
            *kind = refused
                .and_then(|error| error.decode_fault())
                .map(|(_, kind)| kind);
        }
        kinds
    });
    assert_eq!(allocations, 0);
    assert_eq!(
        kinds,
        [
            Some(DecodingKind::Missing),
            // Cut short: a short buffer, which is no decoding fault.
            None,
            Some(DecodingKind::Trailing),
            Some(DecodingKind::Missing),
            Some(DecodingKind::Unsupported),
            Some(DecodingKind::InvalidEncoding),
        ]
    );
    let (_, allocations) = counted(|| {
        assert!(ObjectIdentifier::new(ObjectType::DEVICE, 4_194_304).is_err());
        assert!(DeviceInstanceRange::new(2, 1).is_err());
        let mut full = [0; 2];
        assert!(encode_app_real_to_sink(&mut &mut full[..], 72.5).is_err());
        assert!((&mut full[..]).write_parts(&[&[1, 2], &[3]]).is_err());
    });
    assert_eq!(allocations, 0);
}
