use super::*;

fn raw_context_value(buf: &mut BytesMut, tag_number: u8, value: &[u8]) {
    tags::encode_tag(buf, tag_number, tags::TagClass::Context, value.len() as u32);
    buf.extend_from_slice(value);
}

fn raw_change_of_life_safety(
    values: [&[u8]; 3],
    field_tags: [u8; 4],
    status_flags: &[u8],
    closing_tag: Option<u8>,
) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, 8);
    raw_context_value(&mut buf, field_tags[0], values[0]);
    raw_context_value(&mut buf, field_tags[1], values[1]);
    raw_context_value(&mut buf, field_tags[2], status_flags);
    raw_context_value(&mut buf, field_tags[3], values[2]);
    if let Some(tag) = closing_tag {
        tags::encode_closing_tag(&mut buf, tag);
    }
    buf
}

fn decode_params(data: &[u8]) -> Result<NotificationParameters, Error> {
    NotificationParameters::decode(data, 0)
}

#[test]
fn change_of_life_safety_accepts_u32_max_with_leading_zero() {
    let max = [0, 0xff, 0xff, 0xff, 0xff];
    let decoded = decode_params(&raw_change_of_life_safety(
        [&max, &max, &max],
        [0, 1, 2, 3],
        &[4, 0xf0],
        Some(8),
    ))
    .unwrap();

    assert_eq!(
        decoded,
        NotificationParameters::ChangeOfLifeSafety {
            new_state: u32::MAX,
            new_mode: u32::MAX,
            status_flags: 0x0f,
            operation_expected: u32::MAX,
        }
    );
}

#[test]
fn change_of_life_safety_rejects_values_above_u32() {
    let one = [1];
    let overflow_alias = [1, 0, 0, 0, 1];
    let u64_max = [0xff; 8];

    for field in 0..3 {
        for invalid in [&overflow_alias[..], &u64_max] {
            let mut values: [&[u8]; 3] = [&one, &one, &one];
            values[field] = invalid;
            assert!(decode_params(&raw_change_of_life_safety(
                values,
                [0, 1, 2, 3],
                &[4, 0x80],
                Some(8),
            ))
            .is_err());
        }
    }
}

#[test]
fn change_of_life_safety_requires_exact_field_tags() {
    for field in 0..4 {
        let mut field_tags = [0, 1, 2, 3];
        field_tags[field] = 7;
        assert!(decode_params(&raw_change_of_life_safety(
            [&[1], &[1], &[1]],
            field_tags,
            &[4, 0x80],
            Some(8),
        ))
        .is_err());
    }

    let mut application_tagged =
        raw_change_of_life_safety([&[1], &[1], &[1]], [0, 1, 2, 3], &[4, 0x80], Some(8));
    application_tagged[1] &= !0x08;
    assert!(decode_params(&application_tagged).is_err());
}

#[test]
fn change_of_life_safety_requires_four_status_bits_with_zero_padding() {
    for invalid in [
        &[][..],
        &[4][..],
        &[3, 0xf0][..],
        &[4, 0xf1][..],
        &[4, 0xf0, 0][..],
    ] {
        assert!(decode_params(&raw_change_of_life_safety(
            [&[1], &[1], &[1]],
            [0, 1, 2, 3],
            invalid,
            Some(8),
        ))
        .is_err());
    }
}

#[test]
fn change_of_life_safety_requires_its_immediate_closing_tag() {
    assert!(decode_params(&raw_change_of_life_safety(
        [&[1], &[1], &[1]],
        [0, 1, 2, 3],
        &[4, 0x80],
        None,
    ))
    .is_err());
    assert!(decode_params(&raw_change_of_life_safety(
        [&[1], &[1], &[1]],
        [0, 1, 2, 3],
        &[4, 0x80],
        Some(9),
    ))
    .is_err());

    let mut extra_field =
        raw_change_of_life_safety([&[1], &[1], &[1]], [0, 1, 2, 3], &[4, 0x80], None);
    raw_context_value(&mut extra_field, 4, &[1]);
    tags::encode_closing_tag(&mut extra_field, 8);
    assert!(decode_params(&extra_field).is_err());
}

#[test]
fn change_of_life_safety_rejects_every_truncated_prefix() {
    let encoded = raw_change_of_life_safety([&[1], &[1], &[1]], [0, 1, 2, 3], &[4, 0x80], Some(8));

    for end in 0..encoded.len() {
        assert!(decode_params(&encoded[..end]).is_err(), "prefix {end}");
    }
}
