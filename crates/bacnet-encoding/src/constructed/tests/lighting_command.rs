//! BACnetLightingCommand (Clause 21): an unframed SEQUENCE of primitive
//! context tags, the operation `[0]` and then optional fields `[1]` to `[5]`.
use super::*;
use bacnet_types::constructed::BACnetLightingCommand;
use bacnet_types::enums::LightingOperation as Op;

fn command(operation: Op) -> BACnetLightingCommand {
    BACnetLightingCommand::new(operation)
}

fn encode(value: &BACnetLightingCommand) -> Vec<u8> {
    let mut buf = BytesMut::new();
    encode_lighting_command(&mut buf, value);
    buf.to_vec()
}

/// One command of every standard operation and of the proprietary range's
/// ends, with independently worked octets. REALs: 50.0 is 0x42480000, 25.0
/// 0x41C80000, 10.0 0x41200000 and 5.0 0x40A00000.
fn golden_vectors() -> Vec<(BACnetLightingCommand, Vec<u8>)> {
    vec![
        (command(Op::NONE), vec![0x09, 0x00]),
        // FADE_TO 50.0 % over 2,000 ms (0x07D0) at priority 8.
        (
            BACnetLightingCommand {
                target_level: Some(50.0),
                fade_time: Some(2_000),
                priority: Some(8),
                ..command(Op::FADE_TO)
            },
            vec![
                0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x4A, 0x07, 0xD0, 0x59, 0x08,
            ],
        ),
        // RAMP_TO 25.0 % at 10.0 %/s.
        (
            BACnetLightingCommand {
                target_level: Some(25.0),
                ramp_rate: Some(10.0),
                ..command(Op::RAMP_TO)
            },
            vec![
                0x09, 0x02, 0x1C, 0x41, 0xC8, 0x00, 0x00, 0x2C, 0x41, 0x20, 0x00, 0x00,
            ],
        ),
        // STEP_UP by 5.0 % at priority 16.
        (
            BACnetLightingCommand {
                step_increment: Some(5.0),
                priority: Some(16),
                ..command(Op::STEP_UP)
            },
            vec![0x09, 0x03, 0x3C, 0x40, 0xA0, 0x00, 0x00, 0x59, 0x10],
        ),
        (command(Op::STEP_DOWN), vec![0x09, 0x04]),
        (command(Op::STEP_ON), vec![0x09, 0x05]),
        (command(Op::STEP_OFF), vec![0x09, 0x06]),
        (
            BACnetLightingCommand {
                priority: Some(1),
                ..command(Op::WARN)
            },
            vec![0x09, 0x07, 0x59, 0x01],
        ),
        (command(Op::WARN_OFF), vec![0x09, 0x08]),
        (command(Op::WARN_RELINQUISH), vec![0x09, 0x09]),
        (command(Op::STOP), vec![0x09, 0x0A]),
        // Proprietary operations 256 and 65,535 take two octets.
        (command(Op::from_raw(256)), vec![0x0A, 0x01, 0x00]),
        (command(Op::from_raw(65_535)), vec![0x0A, 0xFF, 0xFF]),
        // Every field, and an 86,400,000 ms (0x05265C00) fade time.
        (
            BACnetLightingCommand {
                operation: Op::FADE_TO,
                target_level: Some(50.0),
                ramp_rate: Some(10.0),
                step_increment: Some(5.0),
                fade_time: Some(86_400_000),
                priority: Some(8),
            },
            vec![
                0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x2C, 0x41, 0x20, 0x00, 0x00, 0x3C, 0x40,
                0xA0, 0x00, 0x00, 0x4C, 0x05, 0x26, 0x5C, 0x00, 0x59, 0x08,
            ],
        ),
    ]
}

#[test]
fn lighting_command_golden_vectors_round_trip_for_each_operation() {
    for (value, expected) in golden_vectors() {
        assert_eq!(encode(&value), expected, "{value:?}");
        let (decoded, end) = decode_lighting_command(&expected, 0).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(end, expected.len());
    }
}

#[test]
fn lighting_command_decodes_at_an_offset_and_stops_at_the_next_other_tag() {
    let (value, octets) = golden_vectors().swap_remove(1);
    // After an application Unsigned, and before another one or the closing
    // tag of a frame.
    for after in [&[0x21, 0x05][..], &[0x0F]] {
        let data = [&[0x21, 0x05][..], &octets, after].concat();
        assert_eq!(
            decode_lighting_command(&data, 2).unwrap(),
            (value, 2 + octets.len())
        );
    }
    // A field out of order or past the last one ends the command there.
    let data = [0x09, 0x02, 0x2C, 0x41, 0x20, 0x00, 0x00, 0x1C, 0x00];
    let (decoded, end) = decode_lighting_command(&data, 0).unwrap();
    assert_eq!(
        (decoded.ramp_rate, decoded.target_level, end),
        (Some(10.0), None, 7)
    );
    assert_eq!(
        decode_lighting_command(&[0x09, 0x07, 0x69, 0x01], 0)
            .unwrap()
            .1,
        2
    );
}

#[test]
fn lighting_command_cut_inside_a_field_is_a_short_buffer() {
    let full = golden_vectors().pop().unwrap().1;
    // Where each field starts; the command may end at any of them.
    let boundaries = [2, 7, 12, 17, 22, full.len()];
    for cut in 1..full.len() {
        let result = decode_lighting_command(&full[..cut], 0);
        if boundaries.contains(&cut) {
            assert_eq!(result.unwrap().1, cut);
        } else {
            match result {
                Err(Error::BufferTooShort { have, .. }) => assert_eq!(have, cut),
                other => panic!("cut at {cut}: {other:?}"),
            }
        }
    }
}

#[test]
fn lighting_command_wrong_tags_and_lengths_are_malformed() {
    let malformed: [&[u8]; 9] = [
        // Nothing, a target level with no operation before it, an
        // application-tagged ENUMERATED and an opening tag [0].
        &[],
        &[0x1C, 0x42, 0x48, 0x00, 0x00],
        &[0x91, 0x01],
        &[0x0E, 0x09, 0x01, 0x0F],
        // An operation with no contents octets.
        &[0x08],
        // A target level of three octets and of five.
        &[0x09, 0x01, 0x1B, 0x42, 0x48, 0x00],
        &[0x09, 0x01, 0x1D, 0x05, 0x42, 0x48, 0x00, 0x00, 0x00],
        // A ramp rate of three octets that the data also cuts short: the
        // header is wrong before the missing octet matters.
        &[0x09, 0x02, 0x2B, 0x41],
        // A fade time with no contents octets.
        &[0x09, 0x01, 0x48],
    ];
    for value in malformed {
        match decode_lighting_command(value, 0) {
            Err(Error::Decoding(_)) => {}
            other => panic!("{value:02X?}: {other:?}"),
        }
    }
}

#[test]
fn lighting_command_fields_too_wide_for_their_type_are_out_of_range() {
    let too_wide: [(&[u8], &str); 3] = [
        // Operation 2^32, fade time 2^32 and priority 256.
        (
            &[0x0D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "operation [0] exceeds 32 bits",
        ),
        (
            &[0x09, 0x01, 0x4D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00],
            "fade-time [4] exceeds 32 bits",
        ),
        (
            &[0x09, 0x07, 0x5A, 0x01, 0x00],
            "priority [5] exceeds an Unsigned8",
        ),
    ];
    for (value, field) in too_wide {
        match decode_lighting_command(value, 0) {
            Err(Error::Range(refused)) => {
                let message = refused.label;
                assert_eq!(message, format!("lighting command {field}"));
            }
            other => panic!("{value:02X?}: {other:?}"),
        }
    }
    // A malformed field after an oversized one decides the error.
    let both = [
        0x0D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x1B, 0x42, 0x48, 0x00,
    ];
    assert!(matches!(
        decode_lighting_command(&both, 0),
        Err(Error::Decoding(_))
    ));
}

#[test]
fn lighting_command_unsigned_past_four_octets_with_a_leading_zero_is_malformed() {
    // Every field fits 32 bits, so five octets opening with zero aren't the
    // shortest form: operation 1, fade time 100 and priority 8 padded so.
    for value in [
        &[0x0D, 0x05, 0x00, 0x00, 0x00, 0x00, 0x01][..],
        &[0x09, 0x01, 0x4D, 0x05, 0x00, 0x00, 0x00, 0x00, 0x64],
        &[0x09, 0x07, 0x5D, 0x05, 0x00, 0x00, 0x00, 0x00, 0x08],
    ] {
        match decode_lighting_command(value, 0) {
            Err(Error::Decoding(_)) => {}
            other => panic!("{value:02X?}: {other:?}"),
        }
    }
    // Four octets may still open with zeros.
    let (decoded, end) = decode_lighting_command(&[0x0C, 0x00, 0x00, 0x00, 0x01], 0).unwrap();
    assert_eq!((decoded, end), (command(Op::FADE_TO), 5));
}

#[test]
fn lighting_command_value_must_fill_its_input_before_a_field_is_too_wide() {
    for (value, _) in golden_vectors() {
        let octets = encode(&value);
        assert_eq!(decode_lighting_command_value(&octets).unwrap(), value);
    }
    // WARN at priority 256: too wide alone, but broken once anything follows,
    // whether an octet past the command or a field out of order.
    let wide = [0x09, 0x07, 0x5A, 0x01, 0x00];
    assert!(matches!(
        decode_lighting_command_value(&wide),
        Err(Error::Range(_))
    ));
    for value in [
        [&wide[..], &[0xFF]].concat(),
        [&wide[..], &[0x1C, 0x42, 0x48, 0x00, 0x00]].concat(),
    ] {
        match decode_lighting_command_value(&value) {
            Err(Error::Decoding(_)) => {}
            other => panic!("{value:02X?}: {other:?}"),
        }
    }
    // The offset reader stops before the extra octet, so it can only report
    // the oversized field.
    assert!(matches!(
        decode_lighting_command(&[0x09, 0x07, 0x5A, 0x01, 0x00, 0xFF], 0),
        Err(Error::Range(_))
    ));
}

#[test]
fn lighting_command_keeps_any_value_that_fits_and_re_encodes_it_shortest() {
    // Leading zero octets, priority 0, a NaN target level and a fade time of
    // 1 ms all decode; ranges are the receiver's to check.
    let data = [
        0x0B, 0x00, 0x00, 0x01, 0x1C, 0x7F, 0xC0, 0x00, 0x00, 0x49, 0x01, 0x59, 0x00,
    ];
    let (decoded, end) = decode_lighting_command(&data, 0).unwrap();
    assert_eq!(end, data.len());
    assert_eq!(decoded.operation, Op::FADE_TO);
    assert!(decoded.target_level.unwrap().is_nan());
    assert_eq!((decoded.fade_time, decoded.priority), (Some(1), Some(0)));
    assert_eq!(
        encode(&decoded),
        [0x09, 0x01, 0x1C, 0x7F, 0xC0, 0x00, 0x00, 0x49, 0x01, 0x59, 0x00]
    );
}

#[test]
fn members_cut_short_are_a_short_buffer() {
    for (_, wire) in golden_vectors() {
        super::assert_members_cut_short("BACnetLightingCommand", &wire, |data| {
            decode_lighting_command(data, 0)
        });
    }
}
