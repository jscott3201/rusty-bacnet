use super::*;

fn group(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn real_72() -> Vec<u8> {
    vec![0x44, 0x42, 0x90, 0x00, 0x00]
}

fn encode(req: &WriteGroupRequest) -> Vec<u8> {
    let mut buf = BytesMut::new();
    req.encode(&mut buf).unwrap();
    buf.to_vec()
}

fn single(channel: u16, value: Vec<u8>) -> WriteGroupRequest {
    WriteGroupRequest {
        group_number: group(1),
        write_priority: 8,
        change_list: vec![GroupChannelValue {
            channel,
            override_priority: None,
            value,
        }],
        inhibit_delay: None,
    }
}

fn assert_decoding_error(data: &[u8]) {
    match WriteGroupRequest::decode(data) {
        Err(Error::Decoding(_)) | Err(Error::BufferTooShort { .. }) => {}
        other => panic!("expected a decoding error for {data:02X?}, got {other:?}"),
    }
}

// --- byte-exact vectors ---------------------------------------------------

#[test]
fn vector_real_value_without_wrapper() {
    // group 1, priority 8, channel 5 holding REAL 72.0.
    let expected = [
        0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x44, 0x42, 0x90, 0x00, 0x00, 0x2F,
    ];
    let req = single(5, real_72());
    assert_eq!(encode(&req), expected);
    assert_eq!(WriteGroupRequest::decode(&expected).unwrap(), req);
}

#[test]
fn vector_override_priority_inhibit_and_multiple_entries() {
    // group 258, priority 16; channel 300 with override 10 holding NULL; channel 0 holding
    // Boolean TRUE; inhibit-delay TRUE.
    let expected = [
        0x0A, 0x01, 0x02, 0x19, 0x10, 0x2E, 0x0A, 0x01, 0x2C, 0x19, 0x0A, 0x00, 0x09, 0x00, 0x11,
        0x2F, 0x39, 0x01,
    ];
    let req = WriteGroupRequest {
        group_number: group(258),
        write_priority: 16,
        change_list: vec![
            GroupChannelValue {
                channel: 300,
                override_priority: Some(10),
                value: vec![0x00],
            },
            GroupChannelValue {
                channel: 0,
                override_priority: None,
                value: vec![0x11],
            },
        ],
        inhibit_delay: Some(true),
    };
    assert_eq!(encode(&req), expected);
    assert_eq!(WriteGroupRequest::decode(&expected).unwrap(), req);
}

#[test]
fn vector_lighting_command_value() {
    // Channel 7 holding a lighting command: operation 1, target-level REAL 50.0, wrapped in
    // context tag 0.
    let value = vec![0x0E, 0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x0F];
    let expected = [
        0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x07, 0x0E, 0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00,
        0x0F, 0x2F,
    ];
    let req = single(7, value);
    assert_eq!(encode(&req), expected);
    assert_eq!(WriteGroupRequest::decode(&expected).unwrap(), req);
}

#[test]
fn vector_channel_bounds_and_group_bounds() {
    // Largest channel number and group number: Unsigned content widths of 2 and 4 octets.
    let req = WriteGroupRequest {
        group_number: group(u32::MAX),
        write_priority: 1,
        change_list: vec![GroupChannelValue {
            channel: u16::MAX,
            override_priority: Some(1),
            value: vec![0x21, 0x2F],
        }],
        inhibit_delay: Some(false),
    };
    let expected = [
        0x0C, 0xFF, 0xFF, 0xFF, 0xFF, 0x19, 0x01, 0x2E, 0x0A, 0xFF, 0xFF, 0x19, 0x01, 0x21, 0x2F,
        0x2F, 0x39, 0x00,
    ];
    assert_eq!(encode(&req), expected);
    // A value whose content octet equals the closing-tag octet must not end the list early.
    assert_eq!(WriteGroupRequest::decode(&expected).unwrap(), req);
}

#[test]
fn vector_annex_f_examples() {
    // The three WriteGroup encodings of Annex F.3.11, service data only.
    let entry = |channel, override_priority, value: &[u8]| GroupChannelValue {
        channel,
        override_priority,
        value: value.to_vec(),
    };
    let cases = [
        (
            WriteGroupRequest {
                group_number: group(23),
                write_priority: 8,
                change_list: vec![
                    entry(268, None, &[0x22, 0x04, 0x57]),
                    entry(269, None, &[0x22, 0x08, 0xAE]),
                ],
                inhibit_delay: None,
            },
            vec![
                0x09, 0x17, 0x19, 0x08, 0x2E, 0x0A, 0x01, 0x0C, 0x22, 0x04, 0x57, 0x0A, 0x01, 0x0D,
                0x22, 0x08, 0xAE, 0x2F,
            ],
        ),
        (
            WriteGroupRequest {
                group_number: group(23),
                write_priority: 8,
                change_list: vec![
                    entry(12, None, &[0x44, 0x42, 0x86, 0x00, 0x00]),
                    entry(13, None, &[0x44, 0x42, 0x90, 0x00, 0x00]),
                ],
                inhibit_delay: Some(true),
            },
            vec![
                0x09, 0x17, 0x19, 0x08, 0x2E, 0x09, 0x0C, 0x44, 0x42, 0x86, 0x00, 0x00, 0x09, 0x0D,
                0x44, 0x42, 0x90, 0x00, 0x00, 0x2F, 0x39, 0x01,
            ],
        ),
        (
            WriteGroupRequest {
                group_number: group(23),
                write_priority: 8,
                change_list: vec![
                    entry(12, None, &[0x22, 0x04, 0x57]),
                    entry(13, Some(10), &[0x74, 0x00, b'A', b'B', b'C']),
                ],
                inhibit_delay: None,
            },
            vec![
                0x09, 0x17, 0x19, 0x08, 0x2E, 0x09, 0x0C, 0x22, 0x04, 0x57, 0x09, 0x0D, 0x19, 0x0A,
                0x74, 0x00, 0x41, 0x42, 0x43, 0x2F,
            ],
        ),
    ];
    for (req, expected) in cases {
        assert_eq!(encode(&req), expected);
        assert_eq!(WriteGroupRequest::decode(&expected).unwrap(), req);
    }
}

#[test]
fn decode_accepts_leading_zero_unsigned_widths() {
    let data = [
        0x0C, 0x00, 0x00, 0x00, 0x01, 0x1A, 0x00, 0x10, 0x2E, 0x0B, 0x00, 0x00, 0x05, 0x1A, 0x00,
        0x0A, 0x44, 0x42, 0x90, 0x00, 0x00, 0x2F,
    ];
    let decoded = WriteGroupRequest::decode(&data).unwrap();
    assert_eq!(decoded.group_number, group(1));
    assert_eq!(decoded.write_priority, 16);
    assert_eq!(decoded.change_list[0].channel, 5);
    assert_eq!(decoded.change_list[0].override_priority, Some(10));
}

#[test]
fn round_trip_every_primitive_value_type() {
    let values: [&[u8]; 6] = [
        &[0x00],
        &[0x10],
        &[0x21, 0x07],
        &[0x91, 0x03],
        &[0x75, 0x05, 0x00, b'a', b'b', b'c', b'd'],
        &[0xC4, 0x02, 0x00, 0x00, 0x01],
    ];
    for value in values {
        let req = single(1, value.to_vec());
        assert_eq!(WriteGroupRequest::decode(&encode(&req)).unwrap(), req);
    }
}

// --- decode negatives -----------------------------------------------------

#[test]
fn decode_rejects_entry_without_channel() {
    assert_decoding_error(&[
        0x09, 0x01, 0x19, 0x08, 0x2E, 0x44, 0x42, 0x90, 0x00, 0x00, 0x2F,
    ]);
    // An override priority with no channel before it.
    assert_decoding_error(&[0x09, 0x01, 0x19, 0x08, 0x2E, 0x19, 0x05, 0x00, 0x2F]);
}

#[test]
fn decode_rejects_object_identifier_channel() {
    // The earlier, non-conformant form sent the channel as a four-octet identifier.
    assert_decoding_error(&[
        0x09, 0x01, 0x19, 0x08, 0x2E, 0x0C, 0x02, 0x80, 0x00, 0x01, 0x44, 0x42, 0x90, 0x00, 0x00,
        0x2F,
    ]);
}

#[test]
fn decode_rejects_value_wrapped_in_context_two() {
    assert_decoding_error(&[
        0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x2E, 0x44, 0x42, 0x90, 0x00, 0x00, 0x2F, 0x2F,
    ]);
}

#[test]
fn decode_rejects_missing_value() {
    assert_decoding_error(&[0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x2F]);
}

#[test]
fn decode_rejects_context_tagged_primitive_value() {
    assert_decoding_error(&[0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x49, 0x01, 0x2F]);
}

#[test]
fn decode_rejects_constructed_value_other_than_lighting_command() {
    assert_decoding_error(&[
        0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x1E, 0x09, 0x01, 0x1F, 0x2F,
    ]);
    // A lighting command must begin with its operation field, and cannot be empty.
    assert_decoding_error(&[
        0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x0E, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x0F, 0x2F,
    ]);
    assert_decoding_error(&[0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x0E, 0x0F, 0x2F]);
}

#[test]
fn decode_rejects_malformed_application_value() {
    // NULL with content, BOOLEAN with a length other than 0 or 1, and a truncated REAL.
    for value in [&[0x01, 0x00][..], &[0x12, 0x00, 0x00], &[0x44, 0x42, 0x90]] {
        let mut data = vec![0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05];
        data.extend_from_slice(value);
        data.push(0x2F);
        assert_decoding_error(&data);
    }
}

#[test]
fn decode_rejects_empty_change_list() {
    assert_decoding_error(&[0x09, 0x01, 0x19, 0x08, 0x2E, 0x2F]);
    assert_decoding_error(&[0x09, 0x01, 0x19, 0x08, 0x2E, 0x2F, 0x39, 0x01]);
}

#[test]
fn decode_rejects_group_zero_as_decoding_error() {
    let data = [
        0x09, 0x00, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x44, 0x42, 0x90, 0x00, 0x00, 0x2F,
    ];
    assert!(matches!(
        WriteGroupRequest::decode(&data),
        Err(Error::Decoding(_))
    ));
}

#[test]
fn decode_rejects_out_of_range_numbers() {
    let entry = [0x09, 0x05, 0x00];
    let build = |group: &[u8], priority: &[u8], entry_head: &[u8]| {
        let mut data = group.to_vec();
        data.extend_from_slice(priority);
        data.push(0x2E);
        data.extend_from_slice(entry_head);
        data.push(0x2F);
        data
    };
    // Positive control: the same scaffold with in-range values decodes.
    WriteGroupRequest::decode(&build(&[0x09, 0x01], &[0x19, 0x08], &entry)).unwrap();
    // Group number above u32: a canonical five-octet Unsigned of 2^32, so the group check is
    // the first failure.
    let err = WriteGroupRequest::decode(&build(
        &[0x0D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
        &[0x19, 0x08],
        &entry,
    ))
    .unwrap_err();
    assert!(matches!(err, Error::Decoding(_)), "{err:?}");
    assert!(err.to_string().contains("group number"), "{err}");
    // Write priority 0, 17 and 257.
    for priority in [&[0x19, 0x00][..], &[0x19, 0x11], &[0x1A, 0x01, 0x01]] {
        assert_decoding_error(&build(&[0x09, 0x01], priority, &entry));
    }
    // The refusal names the priority and its value with a static label (#1776).
    let err = WriteGroupRequest::decode(&build(&[0x09, 0x01], &[0x19, 0x11], &entry)).unwrap_err();
    assert_eq!(
        err.to_string(),
        "decoding error at offset 2: WriteGroup write-priority 17 out of range 1-16"
    );
    // Override priority 0, 17 and 256.
    for override_priority in [&[0x19, 0x00][..], &[0x19, 0x11], &[0x1A, 0x01, 0x00]] {
        let mut entry_head = vec![0x09, 0x05];
        entry_head.extend_from_slice(override_priority);
        entry_head.push(0x00);
        assert_decoding_error(&build(&[0x09, 0x01], &[0x19, 0x08], &entry_head));
    }
    // Channel 65536.
    assert_decoding_error(&build(
        &[0x09, 0x01],
        &[0x19, 0x08],
        &[0x0B, 0x01, 0x00, 0x00, 0x00],
    ));
}

#[test]
fn decode_priority_errors_name_the_field() {
    let data = [0x09, 0x01, 0x19, 0x11, 0x2E, 0x09, 0x05, 0x00, 0x2F];
    let message = WriteGroupRequest::decode(&data).unwrap_err().to_string();
    assert!(message.contains("write-priority 17"), "{message}");
}

#[test]
fn decode_rejects_bad_inhibit_delay() {
    let head = [0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x00, 0x2F];
    for tail in [
        &[0x39, 0x02][..],
        &[0x39, 0x00, 0x00],
        &[0x3A, 0x00, 0x01],
        &[0x38],
    ] {
        let mut data = head.to_vec();
        data.extend_from_slice(tail);
        assert_decoding_error(&data);
    }
}

#[test]
fn decode_rejects_trailing_data() {
    let mut data = encode(&single(5, real_72()));
    data.push(0x00);
    assert_decoding_error(&data);

    // A second inhibit-delay field.
    let mut data = encode(&single(5, real_72()));
    data.extend_from_slice(&[0x39, 0x01, 0x39, 0x01]);
    assert_decoding_error(&data);
}

#[test]
fn decode_rejects_every_truncation() {
    // Without the optional trailing field every strict prefix is incomplete.
    let full = encode(&single(5, real_72()));
    for len in 0..full.len() {
        assert!(
            WriteGroupRequest::decode(&full[..len]).is_err(),
            "prefix of {len} octets decoded"
        );
    }
    assert!(WriteGroupRequest::decode(&full).is_ok());
}

#[test]
fn decode_rejects_empty_input_and_wrong_leading_tags() {
    assert_decoding_error(&[]);
    // Application-tagged group number.
    assert_decoding_error(&[0x21, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05, 0x00, 0x2F]);
    // Missing change list.
    assert_decoding_error(&[0x09, 0x01, 0x19, 0x08]);
}

#[test]
fn decode_rejects_oversized_change_list() {
    let mut data = vec![0x09, 0x01, 0x19, 0x08, 0x2E];
    for _ in 0..=MAX_DECODED_ITEMS {
        data.extend_from_slice(&[0x09, 0x00, 0x00]);
    }
    data.push(0x2F);
    assert_decoding_error(&data);
}

// --- encode negatives -----------------------------------------------------

fn assert_encode_error(req: &WriteGroupRequest) {
    let mut buf = BytesMut::new();
    assert!(matches!(req.encode(&mut buf), Err(Error::Encode(_))));
    assert!(
        buf.is_empty(),
        "failed encode must not write partial output"
    );
}

#[test]
fn encode_rejects_write_priority_outside_one_to_sixteen() {
    for priority in [0, 17, 255] {
        let mut req = single(5, real_72());
        req.write_priority = priority;
        assert_encode_error(&req);
    }
    for priority in [1, 16] {
        let mut req = single(5, real_72());
        req.write_priority = priority;
        assert!(req.encode(&mut BytesMut::new()).is_ok());
    }
}

#[test]
fn encode_rejects_override_priority_outside_one_to_sixteen() {
    for priority in [0, 17, 255] {
        let mut req = single(5, real_72());
        req.change_list[0].override_priority = Some(priority);
        assert_encode_error(&req);
    }
}

#[test]
fn encode_rejects_empty_change_list() {
    let mut req = single(5, real_72());
    req.change_list.clear();
    assert_encode_error(&req);
}

#[test]
fn encode_rejects_values_that_are_not_one_channel_value() {
    let bad_values: [&[u8]; 8] = [
        &[],
        &[0x44, 0x42, 0x90],
        // Two elements.
        &[0x00, 0x00],
        // Context-tagged primitive and a wrapper in context tag 2.
        &[0x49, 0x01],
        &[0x2E, 0x44, 0x42, 0x90, 0x00, 0x00, 0x2F],
        // Lighting command without its operation field, and an unterminated one.
        &[0x0E, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x0F],
        &[0x0E, 0x09, 0x01],
        // Reserved application tag number.
        &[0xD0],
    ];
    for value in bad_values {
        assert_encode_error(&single(5, value.to_vec()));
    }
}

// --- channel values in other forms ---------------------------------------

/// Channel 5 carrying `value`, inside an otherwise valid request.
fn wire_with_value(value: &[u8]) -> Vec<u8> {
    let mut data = vec![0x09, 0x01, 0x19, 0x08, 0x2E, 0x09, 0x05];
    data.extend_from_slice(value);
    data.push(0x2F);
    data
}

#[test]
fn character_string_values_in_other_charsets_round_trip() {
    let values: [&[u8]; 4] = [
        // DBCS (charset 1) with a two-octet code page, JIS X 0208 (2), UCS-4 (3) and UCS-2 (4),
        // the last two holding U+0041.
        &[0x74, 0x01, 0x03, 0xA8, 0x41],
        &[0x73, 0x02, 0x30, 0x21],
        &[0x75, 0x05, 0x03, 0x00, 0x00, 0x00, 0x41],
        &[0x73, 0x04, 0x00, 0x41],
    ];
    for value in values {
        let req = single(5, value.to_vec());
        let mut buf = BytesMut::new();
        req.encode(&mut buf)
            .unwrap_or_else(|e| panic!("{value:02X?}: {e}"));
        assert_eq!(WriteGroupRequest::decode(&buf).unwrap(), req);
        assert!(WriteGroupRequest::decode(&wire_with_value(value)).is_ok());
    }
}

#[test]
fn truncated_application_value_is_rejected_by_decode_and_encode() {
    // A REAL that stops after two content octets inside the list.
    let data = wire_with_value(&[0x44, 0x42, 0x90]);
    assert_decoding_error(&data);
    // A CharacterString claiming more octets than remain, and a UCS-4 string with 3 payload octets.
    assert_decoding_error(&wire_with_value(&[0x75, 0x09, 0x03, 0x00, 0x00]));
    assert_decoding_error(&wire_with_value(&[0x74, 0x03, 0x00, 0x00, 0x41]));
    assert_encode_error(&single(5, vec![0x44, 0x42, 0x90]));
    assert_encode_error(&single(5, vec![0x74, 0x03, 0x00, 0x00, 0x41]));
}

#[test]
fn invalid_value_error_points_at_the_value() {
    let data = wire_with_value(&[0x74, 0x03, 0x00, 0x00, 0x41]);
    match WriteGroupRequest::decode(&data) {
        Err(error) if error.decode_fault().is_some() => {
            let (offset, _) = error.decode_fault().unwrap();
            assert!(offset >= 7, "offset {offset}");
        }
        other => panic!("expected a decoding error, got {other:?}"),
    }
}

// --- lighting command structure -------------------------------------------

/// Wrap lighting-command fields in context tag 0.
fn lighting(fields: &[u8]) -> Vec<u8> {
    let mut value = vec![0x0E];
    value.extend_from_slice(fields);
    value.push(0x0F);
    value
}

#[test]
fn lighting_command_with_every_field_round_trips() {
    // operation 1, target-level 50.0, ramp-rate 2.0, step-increment 1.0, fade-time 1000,
    // priority 8.
    let fields = [
        0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x2C, 0x40, 0x00, 0x00, 0x00, 0x3C, 0x3F, 0x80,
        0x00, 0x00, 0x4A, 0x03, 0xE8, 0x59, 0x08,
    ];
    let req = single(5, lighting(&fields));
    assert_eq!(WriteGroupRequest::decode(&encode(&req)).unwrap(), req);
    // Fields may be skipped as long as their numbers increase.
    let req = single(5, lighting(&[0x09, 0x03, 0x59, 0x10]));
    assert_eq!(WriteGroupRequest::decode(&encode(&req)).unwrap(), req);
}

#[test]
fn lighting_command_rejects_malformed_structure() {
    let bad: [&[u8]; 10] = [
        // Out of order, and a repeated field.
        &[
            0x09, 0x01, 0x2C, 0x40, 0x00, 0x00, 0x00, 0x1C, 0x42, 0x48, 0x00, 0x00,
        ],
        &[
            0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x1C, 0x42, 0x48, 0x00, 0x00,
        ],
        // A nested opening tag and a nested constructed element.
        &[0x09, 0x01, 0x1E, 0x1F],
        &[0x09, 0x01, 0x3E, 0x09, 0x01, 0x3F],
        // Application-tagged element between context fields.
        &[0x09, 0x01, 0x21, 0x01],
        // REAL with the wrong length.
        &[0x09, 0x01, 0x1B, 0x42, 0x48, 0x00],
        &[0x09, 0x01, 0x1D, 0x05, 0x42, 0x48, 0x00, 0x00, 0x00],
        // Field number above 5, empty Unsigned, and five-octet Unsigned.
        &[0x09, 0x01, 0x69, 0x00],
        &[0x08],
        &[0x09, 0x01, 0x5D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
    ];
    for fields in bad {
        let value = lighting(fields);
        assert_encode_error(&single(5, value.clone()));
        assert_decoding_error(&wire_with_value(&value));
    }
    // Priority outside 1-16 inside a lighting command.
    for priority in [0x00, 0x11] {
        let value = lighting(&[0x09, 0x01, 0x59, priority]);
        assert_encode_error(&single(5, value.clone()));
        assert_decoding_error(&wire_with_value(&value));
    }
}
