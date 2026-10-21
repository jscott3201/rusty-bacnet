use super::*;
use bacnet_types::constructed::BACnetAddress;

/// Vendor 260, model "M", serial "S", all application-tagged.
const IDENTITY: [u8; 9] = [0x22, 0x01, 0x04, 0x72, 0x00, 0x4D, 0x72, 0x00, 0x53];
/// Application-tagged Device object 1234 (type 8 in the top ten bits).
const DEVICE_1234: [u8; 5] = [0xC4, 0x02, 0x00, 0x04, 0xD2];
/// Application-tagged six-octet MAC address 0A 00 00 01 BA C0; lengths above four use the
/// extended length octet.
const MAC: [u8; 8] = [0x65, 0x06, 0x0A, 0x00, 0x00, 0x01, 0xBA, 0xC0];

fn who_am_i() -> WhoAmIRequest {
    WhoAmIRequest {
        vendor_id: 260,
        model_name: "M".into(),
        serial_number: "S".into(),
    }
}

fn you_are(
    device_identifier: Option<ObjectIdentifier>,
    device_mac_address: Option<Vec<u8>>,
) -> YouAreRequest {
    YouAreRequest {
        vendor_id: 260,
        model_name: "M".into(),
        serial_number: "S".into(),
        device_identifier,
        device_mac_address,
    }
}

fn device_1234() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap()
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn encode_you_are(req: &YouAreRequest) -> Vec<u8> {
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    buf.to_vec()
}

fn assert_who_am_i_decoding_error(data: &[u8]) {
    assert!(
        matches!(
            WhoAmIRequest::decode(data),
            Err(Error::Decoding(_) | Error::BufferTooShort { .. })
        ),
        "expected a decoding error for {data:02X?}"
    );
}

fn assert_you_are_decoding_error(data: &[u8]) {
    assert!(
        matches!(
            YouAreRequest::decode(data),
            Err(Error::Decoding(_) | Error::BufferTooShort { .. })
        ),
        "expected a decoding error for {data:02X?}"
    );
}

// --- Who-Am-I -------------------------------------------------------------

#[test]
fn who_am_i_vector() {
    let mut buf = BytesMut::new();
    who_am_i().encode(&mut buf).unwrap();
    assert_eq!(&buf[..], &IDENTITY);
    assert_eq!(WhoAmIRequest::decode(&IDENTITY).unwrap(), who_am_i());
}

#[test]
fn who_am_i_round_trip_with_extended_string_lengths() {
    let req = WhoAmIRequest {
        vendor_id: u16::MAX,
        model_name: "Model-With-A-Long-Name".into(),
        serial_number: "SN-0123456789".into(),
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    assert_eq!(&buf[..3], &[0x22, 0xFF, 0xFF]);
    assert_eq!(WhoAmIRequest::decode(&buf).unwrap(), req);
}

#[test]
fn who_am_i_vendor_zero_encodes_in_one_octet() {
    let mut req = who_am_i();
    req.vendor_id = 0;
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    assert_eq!(&buf[..2], &[0x21, 0x00]);
    assert_eq!(WhoAmIRequest::decode(&buf).unwrap(), req);
}

#[test]
fn who_am_i_rejects_missing_fields() {
    assert_who_am_i_decoding_error(&[]);
    // Vendor only, and vendor plus model.
    assert_who_am_i_decoding_error(&IDENTITY[..3]);
    assert_who_am_i_decoding_error(&IDENTITY[..6]);
}

#[test]
fn who_am_i_rejects_every_truncation() {
    for len in 0..IDENTITY.len() {
        assert_who_am_i_decoding_error(&IDENTITY[..len]);
    }
}

#[test]
fn who_am_i_rejects_trailing_data() {
    for tail in [&[0x00][..], &[0x72, 0x00, 0x53], &[0xFF, 0x01, 0x02]] {
        assert_who_am_i_decoding_error(&concat(&[&IDENTITY, tail]));
    }
}

#[test]
fn who_am_i_rejects_context_tagged_fields() {
    // The context-tagged layout the codec used to share with You-Are.
    assert_who_am_i_decoding_error(&[0x09, 0x05, 0x19, 0x00, 0x4D, 0x29, 0x00, 0x53]);
    let mut wrong_vendor = IDENTITY;
    wrong_vendor[0] = 0x0A;
    assert_who_am_i_decoding_error(&wrong_vendor);
}

#[test]
fn who_am_i_rejects_wrong_application_types() {
    // Vendor as Enumerated, model as OctetString, serial as Unsigned.
    for (index, replacement) in [(0, 0x92), (3, 0x62), (6, 0x22)] {
        let mut data = IDENTITY;
        data[index] = replacement;
        assert_who_am_i_decoding_error(&data);
    }
}

#[test]
fn who_am_i_rejects_vendor_above_u16() {
    let data = concat(&[&[0x23, 0x01, 0x00, 0x00], &IDENTITY[3..]]);
    assert_who_am_i_decoding_error(&data);
}

#[test]
fn who_am_i_rejects_unknown_character_set() {
    let mut data = IDENTITY;
    data[4] = 0x09;
    assert_who_am_i_decoding_error(&data);
}

// --- You-Are --------------------------------------------------------------

#[test]
fn you_are_vector_device_identifier_only() {
    let req = you_are(Some(device_1234()), None);
    let expected = concat(&[&IDENTITY, &DEVICE_1234]);
    assert_eq!(encode_you_are(&req), expected);
    assert_eq!(YouAreRequest::decode(&expected).unwrap(), req);
}

#[test]
fn you_are_vector_mac_address_only() {
    let req = you_are(None, Some(vec![0x0A, 0x00, 0x00, 0x01, 0xBA, 0xC0]));
    let expected = concat(&[&IDENTITY, &MAC]);
    assert_eq!(encode_you_are(&req), expected);
    assert_eq!(YouAreRequest::decode(&expected).unwrap(), req);
}

#[test]
fn you_are_vector_both_optional_fields() {
    let req = you_are(
        Some(device_1234()),
        Some(vec![0x0A, 0x00, 0x00, 0x01, 0xBA, 0xC0]),
    );
    let expected = concat(&[&IDENTITY, &DEVICE_1234, &MAC]);
    assert_eq!(encode_you_are(&req), expected);
    assert_eq!(YouAreRequest::decode(&expected).unwrap(), req);
}

#[test]
fn you_are_one_octet_mac_and_unconfigured_device_instance() {
    let req = you_are(
        Some(ObjectIdentifier::new(ObjectType::DEVICE, ObjectIdentifier::MAX_INSTANCE).unwrap()),
        Some(vec![0x17]),
    );
    let encoded = encode_you_are(&req);
    assert_eq!(&encoded[9..], &[0xC4, 0x02, 0x3F, 0xFF, 0xFF, 0x61, 0x17]);
    assert_eq!(YouAreRequest::decode(&encoded).unwrap(), req);
}

#[test]
fn you_are_vendor_id_must_fit_u16() {
    let maximum = concat(&[&[0x22, 0xFF, 0xFF], &IDENTITY[3..], &DEVICE_1234]);
    assert_eq!(YouAreRequest::decode(&maximum).unwrap().vendor_id, u16::MAX);

    // A leading zero octet is tolerated; a value above 65535 is not.
    let leading_zero = concat(&[&[0x23, 0x00, 0xFF, 0xFF], &IDENTITY[3..], &DEVICE_1234]);
    assert_eq!(
        YouAreRequest::decode(&leading_zero).unwrap().vendor_id,
        u16::MAX
    );
    for over in [
        &[0x23, 0x01, 0x00, 0x00][..],
        &[0x24, 0x00, 0x01, 0x00, 0x01],
        &[0x25, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    ] {
        assert_you_are_decoding_error(&concat(&[over, &IDENTITY[3..], &DEVICE_1234]));
    }
}

#[test]
fn you_are_rejects_neither_optional_field() {
    assert_you_are_decoding_error(&IDENTITY);
    let mut buf = BytesMut::new();
    assert!(matches!(
        you_are(None, None).encode(&mut buf),
        Err(Error::Encode(_))
    ));
    assert!(buf.is_empty());
}

#[test]
fn you_are_rejects_context_tagged_layout() {
    // Context tags 0..4 as the codec used to write them.
    let mut legacy = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut legacy, 0, 260);
    primitives::encode_ctx_character_string(&mut legacy, 1, "M").unwrap();
    primitives::encode_ctx_character_string(&mut legacy, 2, "S").unwrap();
    primitives::encode_ctx_object_id(&mut legacy, 3, &device_1234());
    assert_you_are_decoding_error(&legacy);

    // Context-tagged optional fields after a conformant prefix.
    for tail in [&[0x3C, 0x02, 0x00, 0x04, 0xD2][..], &[0x49, 0x17]] {
        assert_you_are_decoding_error(&concat(&[&IDENTITY, tail]));
    }
}

#[test]
fn you_are_rejects_missing_or_mistagged_mandatory_fields() {
    assert_you_are_decoding_error(&[]);
    // Vendor as Enumerated, model as OctetString, serial as Unsigned.
    for (index, replacement) in [(0, 0x92), (3, 0x62), (6, 0x22)] {
        let mut data = concat(&[&IDENTITY, &DEVICE_1234]);
        data[index] = replacement;
        assert_you_are_decoding_error(&data);
    }
}

#[test]
fn you_are_rejects_every_truncation() {
    let full = concat(&[&IDENTITY, &DEVICE_1234, &MAC]);
    for len in 0..full.len() {
        let accepted = YouAreRequest::decode(&full[..len]).is_ok();
        // Prefixes ending on a field boundary after the identifier are themselves complete.
        let boundary = len == IDENTITY.len() + DEVICE_1234.len();
        assert_eq!(accepted, boundary, "prefix of {len} octets");
    }
}

#[test]
fn you_are_rejects_trailing_or_misordered_data() {
    let identifier_and_mac = concat(&[&IDENTITY, &DEVICE_1234, &MAC]);
    // Duplicate and misordered optional fields.
    assert_you_are_decoding_error(&concat(&[&identifier_and_mac, &MAC]));
    assert_you_are_decoding_error(&concat(&[&identifier_and_mac, &DEVICE_1234]));
    assert_you_are_decoding_error(&concat(&[&IDENTITY, &MAC, &DEVICE_1234]));
    assert_you_are_decoding_error(&concat(&[&IDENTITY, &DEVICE_1234, &[0x00]]));
    assert_you_are_decoding_error(&concat(&[&IDENTITY, &DEVICE_1234, &DEVICE_1234]));
}

#[test]
fn you_are_rejects_non_device_identifier() {
    let analog_input = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut buf = BytesMut::new();
    assert!(matches!(
        you_are(Some(analog_input), None).encode(&mut buf),
        Err(Error::Encode(_))
    ));
    assert!(buf.is_empty());
    // The same identifier on the wire: Analog Input 1 is 00 00 00 01.
    assert_you_are_decoding_error(&concat(&[&IDENTITY, &[0xC4, 0x00, 0x00, 0x00, 0x01]]));
}

#[test]
fn you_are_rejects_wrong_identifier_length() {
    assert_you_are_decoding_error(&concat(&[&IDENTITY, &[0xC3, 0x02, 0x00, 0x04]]));
    assert_you_are_decoding_error(&concat(&[
        &IDENTITY,
        &[0xC5, 0x05, 0x02, 0x00, 0x04, 0xD2, 0x00],
    ]));
}

#[test]
fn who_am_i_model_name_over_253_octets_uses_two_octet_length() {
    // 300 characters plus the character-set octet is 301 content octets: header 75 FE 01 2D.
    let mut expected = vec![0x22, 0x01, 0x04, 0x75, 0xFE, 0x01, 0x2D, 0x00];
    expected.extend(std::iter::repeat_n(b'a', 300));
    expected.extend_from_slice(&[0x72, 0x00, 0x53]);
    let request = WhoAmIRequest {
        model_name: "a".repeat(300),
        ..who_am_i()
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();
    assert_eq!(buf.to_vec(), expected);
    assert_eq!(WhoAmIRequest::decode(&expected).unwrap(), request);
}

/// An application-tagged OCTET STRING of `length` octets: tag 6 with the extended length
/// octet, then the octets.
fn app_octets(length: u8) -> Vec<u8> {
    let mut wire = vec![0x65, length];
    wire.extend((0..length).map(|i| 0xA0u8.wrapping_add(i)));
    wire
}

#[test]
fn you_are_mac_address_holds_to_the_bacnet_address_bound() {
    let longest = BACnetAddress::MAX_MAC_LEN as u8;
    // The longest MAC the stack takes, `BACnetAddress::MAX_MAC_LEN` (#1200).
    let req = you_are(Some(device_1234()), Some(app_octets(longest)[2..].to_vec()));
    let expected = concat(&[&IDENTITY, &DEVICE_1234, &app_octets(longest)]);
    assert_eq!(encode_you_are(&req), expected);
    assert_eq!(YouAreRequest::decode(&expected).unwrap(), req);

    // One octet more names no node on any of them, in either direction.
    for prefix in [&IDENTITY[..], &concat(&[&IDENTITY, &DEVICE_1234])] {
        assert_you_are_decoding_error(&concat(&[prefix, &app_octets(longest + 1)]));
    }
    let too_long = you_are(
        Some(device_1234()),
        Some(app_octets(longest + 1)[2..].to_vec()),
    );
    let mut buf = BytesMut::from(&[0xEE][..]);
    assert!(matches!(too_long.encode(&mut buf), Err(Error::Encode(_))));
    assert_eq!(&buf[..], &[0xEE]);
}
