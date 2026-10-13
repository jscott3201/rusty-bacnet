//! BACnetChannelValue extents (Clause 21): one application-tagged primitive,
//! a lighting command framed in context tag 0, or Addendum 135-2020ca's xy
//! colour in tag 1 and colour command in tag 2 (#1474).
use super::*;

/// Operation 1 with a target level of 50.0, framed in [0].
const LIGHTING: [u8; 9] = [0x0E, 0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x0F];

/// x 0.5 (0x3F000000) and y 0.25 (0x3E800000), framed in [1].
const XY: [u8; 12] = [
    0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F,
];

/// FADE_TO_CCT (2) to 2,700 K (0x0A8C) over 100 ms, framed in [2].
const COLOR_COMMAND: [u8; 9] = [0x2E, 0x09, 0x02, 0x2A, 0x0A, 0x8C, 0x39, 0x64, 0x2F];

fn is_lighting_command_channel_value(value: &[u8]) -> bool {
    constructed_channel_value(value) == Some(ConstructedChannelValue::LightingCommand)
}

#[test]
fn primitives_and_a_lighting_command_are_channel_values() {
    let values: [&[u8]; 7] = [
        &[0x00],                         // NULL
        &[0x44, 0x42, 0x90, 0x00, 0x00], // REAL 72.0
        &[0x91, 0x01],                   // ENUMERATED 1
        &[0x73, 0x00, 0x6F, 0x6E],       // CharacterString "on"
        &LIGHTING,
        &XY,
        &COLOR_COMMAND,
    ];
    for value in values {
        // A value ends where the next one would start.
        let followed = [value, &[0x21, 0x05]].concat();
        assert_eq!(
            channel_value_end(&followed, 0).unwrap(),
            value.len(),
            "{value:02X?}"
        );
        assert_eq!(
            is_lighting_command_channel_value(value),
            value == LIGHTING,
            "{value:02X?}"
        );
    }
    // Fields may be skipped as long as their numbers increase.
    assert!(is_lighting_command_channel_value(&[
        0x0E, 0x09, 0x03, 0x59, 0x10, 0x0F
    ]));
    // Trailing octets make it more than one value.
    assert!(!is_lighting_command_channel_value(
        &[&LIGHTING[..], &[0x00]].concat()
    ));
}

#[test]
fn each_constructed_alternative_is_told_by_its_tag() {
    use ConstructedChannelValue as C;
    // STOP, the shortest colour command.
    let stop: &[u8] = &[0x2E, 0x09, 0x06, 0x2F];
    // FADE_TO_COLOR to (0.5, 0.25): a frame [1] nested inside [2].
    let fade_to_color: &[u8] = &[
        0x2E, 0x09, 0x01, 0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F,
        0x2F,
    ];
    // Any REAL is an xy colour here (NaN and -1.0); the Color object checks
    // the range.
    let any_reals: &[u8] = &[
        0x1E, 0x44, 0x7F, 0xC0, 0x00, 0x00, 0x44, 0xBF, 0x80, 0x00, 0x00, 0x1F,
    ];
    for (value, kind) in [
        (&LIGHTING[..], C::LightingCommand),
        (&XY[..], C::XyColor),
        (&COLOR_COMMAND[..], C::ColorCommand),
        (stop, C::ColorCommand),
        (fade_to_color, C::ColorCommand),
        (any_reals, C::XyColor),
    ] {
        assert_eq!(channel_value_end(value, 0).unwrap(), value.len());
        assert_eq!(constructed_channel_value(value), Some(kind), "{value:02X?}");
    }
    // A primitive is no constructed alternative, and leftovers spoil one.
    assert_eq!(
        constructed_channel_value(&[0x44, 0x42, 0x90, 0x00, 0x00]),
        None
    );
    assert_eq!(
        constructed_channel_value(&[&XY[..], &[0x00]].concat()),
        None
    );
    assert_eq!(constructed_channel_value(&[]), None);
}

#[test]
fn malformed_colour_alternatives_are_refused() {
    let bad: [&[u8]; 9] = [
        // An xy colour of one REAL, of three, of an Unsigned first, and
        // closed by the wrong tag.
        &[0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x1F],
        &[
            0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3F, 0x00,
            0x00, 0x00, 0x1F,
        ],
        &[0x1E, 0x21, 0x01, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x1F],
        &[
            0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x2F,
        ],
        // A colour command with no operation, a REAL inside, an operation
        // too wide for 32 bits, fields out of order, and field 6.
        &[0x2E, 0x2A, 0x0A, 0x8C, 0x2F],
        &[0x2E, 0x44, 0x42, 0x90, 0x00, 0x00, 0x2F],
        &[0x2E, 0x0D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2F],
        &[0x2E, 0x09, 0x02, 0x39, 0x64, 0x2A, 0x0A, 0x8C, 0x2F],
        &[0x2E, 0x09, 0x04, 0x69, 0x01, 0x2F],
    ];
    for value in bad {
        assert!(
            matches!(
                channel_value_end(value, 0),
                Err(bacnet_types::error::Error::Decoding { .. })
            ),
            "{value:02X?}"
        );
        assert_eq!(constructed_channel_value(value), None, "{value:02X?}");
    }
    // Context tag 3 is no alternative, nor is a primitive context tag.
    assert!(channel_value_end(&[0x3E, 0x09, 0x06, 0x3F], 0).is_err());
    assert!(channel_value_end(&[0x29, 0x01], 0).is_err());
    // Cut short inside the xy colour.
    assert!(matches!(
        channel_value_end(&XY[..4], 0),
        Err(bacnet_types::error::Error::BufferTooShort { .. })
    ));
}

#[test]
fn malformed_lighting_commands_are_refused() {
    let bad: [&[u8]; 12] = [
        // No operation field, and an empty command.
        &[0x0E, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x0F],
        &[0x0E, 0x0F],
        // A priority too wide for an Unsigned8, an operation too wide for 32
        // bits, operation 1 in five octets, and a constructed field.
        &[0x0E, 0x09, 0x01, 0x5A, 0x01, 0x2C, 0x0F],
        &[0x0E, 0x0D, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00, 0x0F],
        &[0x0E, 0x0D, 0x05, 0x00, 0x00, 0x00, 0x00, 0x01, 0x0F],
        &[0x0E, 0x09, 0x01, 0x1E, 0x1F, 0x0F],
        // Fields out of order.
        &[
            0x0E, 0x09, 0x01, 0x2C, 0x40, 0x00, 0x00, 0x00, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x0F,
        ],
        // A REAL field with three octets, and field 6.
        &[0x0E, 0x09, 0x01, 0x1B, 0x42, 0x48, 0x00, 0x0F],
        &[0x0E, 0x09, 0x01, 0x69, 0x00, 0x0F],
        // Priority 0 and 17.
        &[0x0E, 0x09, 0x01, 0x59, 0x00, 0x0F],
        &[0x0E, 0x09, 0x01, 0x59, 0x11, 0x0F],
        // No closing tag.
        &[0x0E, 0x09, 0x01],
    ];
    for value in bad {
        // None of them is cut short, so each is a decoding error.
        assert!(
            matches!(
                channel_value_end(value, 0),
                Err(bacnet_types::error::Error::Decoding { .. })
            ),
            "{value:02X?}"
        );
        assert!(!is_lighting_command_channel_value(value), "{value:02X?}");
    }
    // Any other context tag isn't a channel value at all.
    assert!(channel_value_end(&[0x19, 0x01], 0).is_err());
    assert!(channel_value_end(&[], 0).is_err());
}

#[test]
fn contents_cut_short_are_a_short_buffer() {
    let short = |value: &[u8]| match channel_value_end(value, 0) {
        Err(bacnet_types::error::Error::BufferTooShort { need, have }) => (need, have),
        other => panic!("expected a short buffer for {value:02X?}, got {other:?}"),
    };
    // The operation field says one octet and the data stops.
    assert_eq!(short(&[0x0E, 0x09]), (3, 2));
    // A target level REAL with two of its four octets.
    assert_eq!(short(&[0x0E, 0x09, 0x01, 0x1C, 0x42, 0x48]), (8, 6));
    // An application REAL cut short reads the same way.
    assert_eq!(short(&[0x44, 0x42, 0x90]), (5, 3));
}

#[test]
fn a_level_of_the_wrong_length_is_malformed_even_when_cut_short() {
    // The target level [1] says three octets and holds two: the length is
    // wrong before the missing octet matters (#1303).
    match channel_value_end(&[0x0E, 0x09, 0x01, 0x1B, 0x42, 0x48], 0) {
        Err(bacnet_types::error::Error::Decoding {
            offset, message, ..
        }) => {
            assert_eq!(offset, 3);
            assert_eq!(
                message,
                "lighting command: [1] REAL has 3 contents octets, expected 4"
            );
        }
        other => panic!("expected a decoding error, got {other:?}"),
    }
}
