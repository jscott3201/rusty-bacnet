//! BACnetxyColor and BACnetColorCommand (Addendum 135-2020ca, Clause 21):
//! an xy colour is two application REALs; the command is an unframed
//! SEQUENCE of the operation `[0]`, a target colour framed in `[1]` and
//! Unsigneds `[2]` to `[5]`.
use super::*;
use bacnet_types::constructed::{BACnetColorCommand, BACnetXyColor};
use bacnet_types::enums::ColorOperation as Op;

fn command(operation: Op) -> BACnetColorCommand {
    BACnetColorCommand::new(operation)
}

fn encode(value: &BACnetColorCommand) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_color_command(&mut buf, value);
    buf.to_vec()
}

/// One command of every operation, and one with every field, with
/// independently worked octets. REALs: 0.5 is 0x3F000000, 0.25 0x3E800000,
/// 1.0 0x3F800000 and 0.0 0x00000000.
fn golden_vectors() -> Vec<(BACnetColorCommand, Vec<u8>)> {
    vec![
        (command(Op::NONE), vec![0x09, 0x00]),
        // FADE_TO_COLOR to (0.5, 0.25) over 2,000 ms (0x07D0).
        (
            BACnetColorCommand {
                target_color: Some(BACnetXyColor::new(0.5, 0.25)),
                fade_time: Some(2_000),
                ..command(Op::FADE_TO_COLOR)
            },
            vec![
                0x09, 0x01, 0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F,
                0x3A, 0x07, 0xD0,
            ],
        ),
        // FADE_TO_CCT to 2,700 K (0x0A8C) over 100 ms.
        (
            BACnetColorCommand {
                target_color_temperature: Some(2_700),
                fade_time: Some(100),
                ..command(Op::FADE_TO_CCT)
            },
            vec![0x09, 0x02, 0x2A, 0x0A, 0x8C, 0x39, 0x64],
        ),
        // RAMP_TO_CCT to 6,500 K (0x1964) at 30,000 K/s (0x7530).
        (
            BACnetColorCommand {
                target_color_temperature: Some(6_500),
                ramp_rate: Some(30_000),
                ..command(Op::RAMP_TO_CCT)
            },
            vec![0x09, 0x03, 0x2A, 0x19, 0x64, 0x4A, 0x75, 0x30],
        ),
        // STEP_UP_CCT by 1 K, then STEP_DOWN_CCT and STOP bare.
        (
            BACnetColorCommand {
                step_increment: Some(1),
                ..command(Op::STEP_UP_CCT)
            },
            vec![0x09, 0x04, 0x59, 0x01],
        ),
        (command(Op::STEP_DOWN_CCT), vec![0x09, 0x05]),
        (command(Op::STOP), vec![0x09, 0x06]),
        // Every field: (1.0, 0.0), 30,000 K, 86,400,000 ms (0x05265C00),
        // 1 K/s and 30,000 K.
        (
            BACnetColorCommand {
                operation: Op::FADE_TO_COLOR,
                target_color: Some(BACnetXyColor::new(1.0, 0.0)),
                target_color_temperature: Some(30_000),
                fade_time: Some(86_400_000),
                ramp_rate: Some(1),
                step_increment: Some(30_000),
            },
            vec![
                0x09, 0x01, 0x1E, 0x44, 0x3F, 0x80, 0x00, 0x00, 0x44, 0x00, 0x00, 0x00, 0x00, 0x1F,
                0x2A, 0x75, 0x30, 0x3C, 0x05, 0x26, 0x5C, 0x00, 0x49, 0x01, 0x5A, 0x75, 0x30,
            ],
        ),
    ]
}

#[test]
fn color_command_golden_vectors_round_trip_for_each_operation() {
    for (value, expected) in golden_vectors() {
        assert_eq!(encode(&value), expected, "{value:?}");
        let (decoded, end) = decode_color_command(&expected, 0).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(end, expected.len());
        assert_eq!(decode_color_command_value(&expected).unwrap(), value);
    }
}

#[test]
fn color_command_round_trips_every_combination_of_optional_fields() {
    // Each optional field with its independently worked octets, in tag
    // order: (0.5, 0.25), 2,700 K (0x0A8C), 2,000 ms (0x07D0), 30,000 K/s
    // (0x7530) and 1 K.
    let target: [u8; 12] = [
        0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F,
    ];
    let fields: [&[u8]; 5] = [
        &target,
        &[0x2A, 0x0A, 0x8C],
        &[0x3A, 0x07, 0xD0],
        &[0x4A, 0x75, 0x30],
        &[0x59, 0x01],
    ];
    for mask in 0u32..32 {
        let has = |bit: u32| mask & (1 << bit) != 0;
        let value = BACnetColorCommand {
            target_color: has(0).then_some(BACnetXyColor::new(0.5, 0.25)),
            target_color_temperature: has(1).then_some(2_700),
            fade_time: has(2).then_some(2_000),
            ramp_rate: has(3).then_some(30_000),
            step_increment: has(4).then_some(1),
            ..command(Op::FADE_TO_COLOR)
        };
        let mut expected = vec![0x09, 0x01];
        for (bit, octets) in (0..).zip(fields) {
            if has(bit) {
                expected.extend_from_slice(octets);
            }
        }
        assert_eq!(encode(&value), expected, "mask {mask:05b}");
        assert_eq!(
            decode_color_command(&expected, 0).unwrap(),
            (value, expected.len()),
            "mask {mask:05b}"
        );
        assert_eq!(decode_color_command_value(&expected).unwrap(), value);
    }
}

#[test]
fn xy_color_is_two_application_reals() {
    // D65 white: 0.3127 is 0x3EA01A37 and 0.3290 0x3EA872B0.
    let d65 = [0x44, 0x3E, 0xA0, 0x1A, 0x37, 0x44, 0x3E, 0xA8, 0x72, 0xB0];
    let color = BACnetXyColor::new(0.3127, 0.3290);
    let mut buf = BytesMut::new();
    encode_xy_color(&mut buf, &color);
    assert_eq!(buf.as_ref(), d65);
    // At an offset, stopping after y.
    let data = [&[0x21, 0x05][..], &d65, &[0x21, 0x05]].concat();
    assert_eq!(decode_xy_color(&data, 2).unwrap(), (color, 12));
    // A missing y, a y of another type, and context-tagged REALs.
    for value in [
        &d65[..5],
        &[&d65[..5], &[0x21, 0x01][..]].concat()[..],
        &[0x0C, 0x3E, 0xA0, 0x1A, 0x37, 0x1C, 0x3E, 0xA8, 0x72, 0xB0][..],
    ] {
        match decode_xy_color(value, 0) {
            Err(Error::Decoding(_)) => {}
            other => panic!("{value:02X?}: {other:?}"),
        }
    }
    // A y cut inside its contents.
    assert!(matches!(
        decode_xy_color(&d65[..8], 0),
        Err(Error::BufferTooShort { have: 8, .. })
    ));
}

#[test]
fn color_command_decodes_at_an_offset_and_stops_at_the_next_other_tag() {
    let (value, octets) = golden_vectors().swap_remove(1);
    // After an application Unsigned, and before another one or the closing
    // tag of a frame.
    for after in [&[0x21, 0x05][..], &[0x2F]] {
        let data = [&[0x21, 0x05][..], &octets, after].concat();
        assert_eq!(
            decode_color_command(&data, 2).unwrap(),
            (value, 2 + octets.len())
        );
    }
    // A field out of order, an undefined [6] and a target colour given as a
    // primitive [1] each end the command before them.
    let cases: [(&[u8], usize); 3] = [
        (&[0x09, 0x02, 0x39, 0x64, 0x2A, 0x0A, 0x8C], 4),
        (&[0x09, 0x04, 0x69, 0x01], 2),
        (&[0x09, 0x01, 0x1C, 0x3F, 0x00, 0x00, 0x00], 2),
    ];
    for (data, end) in cases {
        assert_eq!(decode_color_command(data, 0).unwrap().1, end);
        match decode_color_command_value(data) {
            Err(Error::Decoding(_)) => {}
            other => panic!("{data:02X?}: {other:?}"),
        }
    }
}

#[test]
fn color_command_cut_inside_a_field_is_a_short_buffer() {
    let full = golden_vectors().pop().unwrap().1;
    // Where each field starts; the command may end at any of them.
    let boundaries = [2, 14, 17, 22, 24, full.len()];
    // Tag boundaries inside the target colour's frame: the frame never
    // closes, which is malformed rather than short.
    let unclosed = [3, 8, 13];
    for cut in 1..full.len() {
        let result = decode_color_command(&full[..cut], 0);
        if boundaries.contains(&cut) {
            assert_eq!(result.unwrap().1, cut);
        } else if unclosed.contains(&cut) {
            assert!(
                matches!(result, Err(Error::Decoding(_))),
                "cut at {cut}: {result:?}"
            );
        } else {
            match result {
                Err(Error::BufferTooShort { have, .. }) => assert_eq!(have, cut),
                other => panic!("cut at {cut}: {other:?}"),
            }
        }
    }
}

#[test]
fn color_command_wrong_tags_and_lengths_are_malformed() {
    let malformed: [&[u8]; 11] = [
        // Nothing, a colour temperature with no operation before it, an
        // application-tagged ENUMERATED and an opening tag [0].
        &[],
        &[0x2A, 0x0A, 0x8C],
        &[0x91, 0x01],
        &[0x0E, 0x09, 0x01, 0x0F],
        // An operation with no contents octets.
        &[0x08],
        // A target colour whose x is an Unsigned, whose x is three octets
        // long, whose y is missing, or that holds a third REAL.
        &[
            0x09, 0x01, 0x1E, 0x21, 0x01, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F,
        ],
        &[
            0x09, 0x01, 0x1E, 0x43, 0x3F, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F,
        ],
        &[0x09, 0x01, 0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x1F],
        &[
            0x09, 0x01, 0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x44,
            0x3E, 0x80, 0x00, 0x00, 0x1F,
        ],
        // A target colour closed by tag [2].
        &[
            0x09, 0x01, 0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x2F,
        ],
        // A fade time with no contents octets.
        &[0x09, 0x02, 0x2A, 0x0A, 0x8C, 0x38],
    ];
    for value in malformed {
        match decode_color_command(value, 0) {
            Err(Error::Decoding(_)) => {}
            other => panic!("{value:02X?}: {other:?}"),
        }
    }
}

#[test]
fn color_command_fields_too_wide_for_their_type_are_out_of_range() {
    // 2^32 in each Unsigned or ENUMERATED field.
    let too_wide: [(&[u8], &str); 5] = [
        (
            &[0x0D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "operation [0] exceeds 32 bits",
        ),
        (
            &[0x09, 0x02, 0x2D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "target-color-temperature [2] exceeds 32 bits",
        ),
        (
            &[0x09, 0x02, 0x3D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "fade-time [3] exceeds 32 bits",
        ),
        (
            &[0x09, 0x03, 0x4D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "ramp-rate [4] exceeds 32 bits",
        ),
        (
            &[0x09, 0x04, 0x5D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "step-increment [5] exceeds 32 bits",
        ),
    ];
    for (value, field) in too_wide {
        for result in [
            decode_color_command(value, 0).map(|(command, _)| command),
            decode_color_command_value(value),
        ] {
            match result {
                Err(Error::Range(refused)) => {
                    let message = refused.label;
                    assert_eq!(message, format!("color command {field}"));
                }
                other => panic!("{value:02X?}: {other:?}"),
            }
        }
    }
    // A malformed field after an oversized one decides the error, and so
    // does an octet after the command when it must fill its input.
    let wide = [0x09, 0x02, 0x2D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00];
    let broken = [&wide[..], &[0x38]].concat();
    assert!(matches!(
        decode_color_command(&broken, 0),
        Err(Error::Decoding(_))
    ));
    let trailing = [&wide[..], &[0x21, 0x01]].concat();
    assert!(matches!(
        decode_color_command(&trailing, 0),
        Err(Error::Range(_))
    ));
    assert!(matches!(
        decode_color_command_value(&trailing),
        Err(Error::Decoding(_))
    ));
}

#[test]
fn color_command_unsigned_past_four_octets_with_a_leading_zero_is_malformed() {
    // STOP and a 2,700 K target padded to five octets.
    for value in [
        &[0x0D, 0x05, 0x00, 0x00, 0x00, 0x00, 0x06][..],
        &[0x09, 0x02, 0x2D, 0x05, 0x00, 0x00, 0x00, 0x0A, 0x8C],
    ] {
        match decode_color_command(value, 0) {
            Err(Error::Decoding(_)) => {}
            other => panic!("{value:02X?}: {other:?}"),
        }
    }
    // Four octets may still open with zeros, and re-encode shortest.
    let (decoded, end) = decode_color_command(&[0x0C, 0x00, 0x00, 0x00, 0x06], 0).unwrap();
    assert_eq!((decoded, end), (command(Op::STOP), 5));
    assert_eq!(encode(&decoded), [0x09, 0x06]);
}

#[test]
fn color_command_keeps_any_value_that_fits() {
    // An undefined operation 7, a NaN x, a 0 K target and a 1 ms fade all
    // decode; ranges are the receiver's to check.
    let data = [
        0x09, 0x07, 0x1E, 0x44, 0x7F, 0xC0, 0x00, 0x00, 0x44, 0x3F, 0x80, 0x00, 0x00, 0x1F, 0x29,
        0x00, 0x39, 0x01,
    ];
    let decoded = decode_color_command_value(&data).unwrap();
    assert_eq!(decoded.operation, Op::from_raw(7));
    let color = decoded.target_color.unwrap();
    assert!(color.x.is_nan());
    assert_eq!(color.y, 1.0);
    assert_eq!(
        (decoded.target_color_temperature, decoded.fade_time),
        (Some(0), Some(1))
    );
    assert_eq!(encode(&decoded), data);
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    let mut framed = 0;
    for (_, wire) in golden_vectors() {
        framed += super::assert_members_cut_short("BACnetColorCommand", &wire, |data| {
            decode_color_command(data, 0)
        });
    }
    assert!(framed > 0);
}
