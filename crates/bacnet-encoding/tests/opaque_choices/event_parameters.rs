//! Event_Parameters under the tags Clause 21 deprecates (7) and reserves for
//! a later addendum (12), #2483: kept as `BACnetEventParameter::Opaque`, as
//! the notification decoder keeps the same tags, while 6 and 19, whose
//! event types carry no parameters, stay refused.

use super::*;
use bacnet_encoding::constructed::{decode_event_parameter, encode_event_parameter};
use bacnet_types::constructed::BACnetEventParameter;

fn parameter(tag: u8, data: &[u8]) -> BACnetEventParameter {
    BACnetEventParameter::Opaque {
        tag,
        data: data.to_vec(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::with_cases(256)
    })]

    /// A deprecated or reserved tag around any well-formed body: the codec
    /// writes the opening tag, the body and the closing tag, which decode
    /// to the same value and encode again to the same octets.
    #[test]
    fn a_deprecated_or_reserved_algorithm_round_trips(tag in prop_oneof![Just(7u8), Just(12u8)], body in body()) {
        let value = parameter(tag, &body);
        let octets = encoded(|buf| encode_event_parameter(buf, &value));
        prop_assert_eq!(&octets, &wire(&constructed(tag, &body)));
        let (decoded, end) = decode_event_parameter(&octets, 0).unwrap();
        prop_assert_eq!(end, octets.len());
        prop_assert_eq!(&decoded, &value);
        prop_assert_eq!(encoded(|buf| encode_event_parameter(buf, &decoded)), octets);
    }
}

#[test]
fn event_parameters_keep_tags_7_and_12_and_refuse_6_and_19() {
    for tag in [7, 12] {
        for body in [&[][..], &[0x09, 0x01], &[0x0E, 0x21, 0x05, 0x0F]] {
            let octets = wire(&constructed(tag, body));
            assert_eq!(
                decode_event_parameter(&octets, 0).unwrap(),
                (parameter(tag, body), octets.len()),
                "{octets:02X?}"
            );
        }
        // Every alternative of this CHOICE but none [20] is constructed,
        // and a malformed body still fails.
        let malformed: [&[u8]; 6] = [
            &[tag << 4 | 0x09, 1],
            &[tag << 4 | 0x0E, 0x1F, tag << 4 | 0x0F],
            &[tag << 4 | 0x0E, 0x44, 0, tag << 4 | 0x0F],
            &[tag << 4 | 0x0E, 0x43, 0, 0, 0, tag << 4 | 0x0F],
            &[tag << 4 | 0x0E, 0x09, 1],
            &[tag << 4 | 0x0E, 0xDF],
        ];
        for octets in malformed {
            assert!(decode_event_parameter(octets, 0).is_err(), "{octets:02X?}");
        }
        let deep = wire(&constructed(tag, &nested(1, MAX_CONTEXT_NESTING_DEPTH)));
        assert!(decode_event_parameter(&deep, 0).is_err());
    }
    // 6 and 19 stay refused in either form: their event types carry no
    // parameters, so a value under one is malformed, not newer.
    for tag in [6, 19] {
        for octets in [
            wire(&constructed(tag, &[0x09, 0x01])),
            wire(&primitive(tag, &[])),
        ] {
            assert!(decode_event_parameter(&octets, 0).is_err(), "{octets:02X?}");
        }
    }
}

/// The encoder keeps an opaque value only under a tag the decoder keeps:
/// 7, 12 and 23 up; never one with a variant, 6 or 19, which would not
/// read back as the same value.
#[test]
fn the_event_parameter_encoder_writes_only_tags_the_decoder_keeps() {
    for tag in [7, 12, 23, 254] {
        let value = parameter(tag, &[0x09, 0x01]);
        let octets = encoded(|buf| encode_event_parameter(buf, &value));
        assert_eq!(decode_event_parameter(&octets, 0).unwrap().0, value);
    }
    for tag in [0, 5, 6, 8, 11, 13, 19, 20, 22] {
        let mut buf = BytesMut::from(&[0xAA][..]);
        assert!(
            encode_event_parameter(&mut buf, &parameter(tag, &[])).is_err(),
            "[{tag}]"
        );
        assert_eq!(&buf[..], &[0xAA]);
    }
    let mut buf = BytesMut::new();
    assert!(encode_event_parameter(&mut buf, &parameter(7, &[0x1F])).is_err());
}
