//! The device-instance range Who-Is and Who-Has carry (#1483).
//!
//! Clauses 16.10.1.1.1-2 and 16.9.1.1.1-2 send both limits or neither, with
//! the low one no greater than the high one. A range holds both, so a
//! request can't carry one alone, and neither the constructors nor the
//! decoders take a low limit above the high one.

use super::*;
use crate::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_types::enums::ObjectType;

#[test]
fn a_range_holds_both_limits_in_order() {
    let range = DeviceInstanceRange::new(10, 20).unwrap();
    assert_eq!((range.low(), range.high()), (10, 20));
    for (instance, inside) in [(9, false), (10, true), (15, true), (20, true), (21, false)] {
        assert_eq!(range.contains(instance), inside, "{instance}");
    }
    let one = DeviceInstanceRange::single(7).unwrap();
    assert_eq!(one, DeviceInstanceRange::new(7, 7).unwrap());
    assert!(one.contains(7) && !one.contains(6) && !one.contains(8));
}

#[test]
fn a_low_limit_above_the_high_one_is_refused() {
    let error = DeviceInstanceRange::new(20, 10).unwrap_err();
    assert!(
        matches!(&error, Error::Range(refused)
            if refused.message().to_string().contains("low limit 20 is above its high limit 10")),
        "{error:?}"
    );
    assert!(DeviceInstanceRange::from_limits(Some(20), Some(10)).is_err());
}

#[test]
fn separate_limits_make_a_range_only_in_pairs() {
    assert_eq!(DeviceInstanceRange::from_limits(None, None).unwrap(), None);
    assert_eq!(
        DeviceInstanceRange::from_limits(Some(1), Some(5)).unwrap(),
        Some(DeviceInstanceRange::new(1, 5).unwrap())
    );
    for (low, high, named) in [
        (Some(1), None, "low limit 1 was given without a high limit"),
        (None, Some(5), "high limit 5 was given without a low limit"),
    ] {
        let error = DeviceInstanceRange::from_limits(low, high).unwrap_err();
        assert!(
            matches!(&error, Error::Range(refused)
                if refused.message().to_string().contains("needs both limits or neither")
                    && refused.message().to_string().contains(named)),
            "{error:?}"
        );
    }
}

/// Both requests put a range on the wire as its two limits, `[0]` then
/// `[1]`, and read it back.
#[test]
fn both_requests_carry_a_range_as_two_limits() {
    let range = Some(DeviceInstanceRange::new(1, 10).unwrap());
    let limits = [0x09, 0x01, 0x19, 0x0A];

    let who_is = WhoIsRequest { range };
    let mut buf = BytesMut::new();
    who_is.encode(&mut buf);
    assert_eq!(buf[..], limits);
    assert_eq!(WhoIsRequest::decode(&buf).unwrap(), who_is);

    let who_has = WhoHasRequest {
        range,
        object: WhoHasObject::Identifier(
            ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        ),
    };
    let mut buf = BytesMut::new();
    who_has.encode(&mut buf).unwrap();
    assert_eq!(buf[..4], limits);
    assert_eq!(WhoHasRequest::decode(&buf).unwrap(), who_has);
}

/// A Who-Has with one limit is malformed, as a Who-Is is (#1447, #1483), and
/// so is one whose low limit is above its high one. Each names the fault.
#[test]
fn a_who_has_with_one_limit_or_an_empty_range_is_refused() {
    // AV-1 as a context [2] object identifier.
    let av_1: &[u8] = &[0x2C, 0x00, 0x80, 0x00, 0x01];
    for (limits, fault) in [
        (
            &[0x09, 0x01][..],
            "WhoHas low limit needs the high limit [1]",
        ),
        (&[0x19, 0x0A], "WhoHas high limit needs the low limit [0]"),
        (
            &[0x09, 0x0A, 0x19, 0x01],
            "WhoHas low limit exceeds high limit",
        ),
    ] {
        let error = WhoHasRequest::decode(&[limits, av_1].concat()).unwrap_err();
        assert!(
            matches!(&error, Error::Decoding(_)) && error.to_string().contains(fault),
            "{limits:02X?}: {error:?}"
        );
    }
}

/// Sending keeps each limit to the instance range, 0 to 4194303
/// (Clauses 16.9.1.1.1-2 and 16.10.1.1.1-2), while a decoder takes a larger
/// limit as written, since some devices send one to mean every device.
#[test]
fn a_limit_past_the_highest_instance_is_refused_when_sending_only() {
    let top = ObjectIdentifier::MAX_INSTANCE;
    assert_eq!(top, 4_194_303);
    let range = DeviceInstanceRange::new(0, top).unwrap();
    assert!(range.contains(top));
    assert_eq!(DeviceInstanceRange::single(top).unwrap().high(), top);
    let device = ObjectIdentifier::new(ObjectType::DEVICE, top).unwrap();
    assert_eq!(
        DeviceInstanceRange::device(device),
        DeviceInstanceRange::single(top).unwrap()
    );
    for (refused, high) in [
        (DeviceInstanceRange::new(0, top + 1), top + 1),
        (DeviceInstanceRange::new(top + 1, u32::MAX), u32::MAX),
        (DeviceInstanceRange::single(top + 1), top + 1),
        (
            DeviceInstanceRange::from_limits(Some(1), Some(top + 1)).map(Option::unwrap),
            top + 1,
        ),
    ] {
        let expected = format!("high limit {high} is above the highest instance, 4194303");
        assert!(
            matches!(&refused, Err(Error::Range(error))
                if error.message().to_string().contains(&expected)),
            "{refused:?}"
        );
    }

    // [0] low limit 0 and [1] high limit 4294967295 decode as written and
    // take in every instance.
    let data = [0x09, 0x00, 0x1C, 0xFF, 0xFF, 0xFF, 0xFF];
    let range = WhoIsRequest::decode(&data).unwrap().range.unwrap();
    assert_eq!((range.low(), range.high()), (0, u32::MAX));
    assert!(range.contains(top));
}
