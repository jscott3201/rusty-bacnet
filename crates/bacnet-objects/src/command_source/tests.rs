use super::*;
use crate::{
    analog::{AnalogOutputObject, AnalogValueObject},
    binary::{BinaryOutputObject, BinaryValueObject},
    multistate::{MultiStateOutputObject, MultiStateValueObject},
    traits::BACnetObject,
};
use bacnet_encoding::{
    constructed::{decode_value_source, encode_value_source},
    primitives::decode_timestamp_choice,
};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;
use bacnet_types::{
    MacAddr,
    enums::{ErrorCode, PropertyIdentifier as P},
    primitives::{BACnetTimeStamp, PropertyValue},
};
use bytes::BytesMut;
fn device(n: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, n).unwrap()
}
fn local(n: u32, object: Option<ObjectIdentifier>) -> CommandOrigin {
    CommandOrigin::Local {
        owner_device: device(n),
        initiating_object: object,
    }
}
fn remote(address: u8, binding: CommandDeviceBinding) -> CommandOrigin {
    CommandOrigin::Remote {
        actual_address: BACnetAddress {
            network_number: 7,
            mac_address: MacAddr::from_slice(&[address]),
        },
        binding,
    }
}
fn source_value(source: &BACnetValueSource) -> PropertyValue {
    let mut b = BytesMut::new();
    encode_value_source(&mut b, source).unwrap();
    PropertyValue::ApplicationData(b.to_vec())
}
fn source(o: &dyn BACnetObject, index: Option<u32>) -> BACnetValueSource {
    let p = if index.is_some() {
        P::VALUE_SOURCE_ARRAY
    } else {
        P::VALUE_SOURCE
    };
    let PropertyValue::ApplicationData(b) = o.read_property(p, index).unwrap() else {
        panic!("typed source")
    };
    let (s, end) = decode_value_source(&b, 0).unwrap();
    assert_eq!(end, b.len());
    s
}
fn time(o: &dyn BACnetObject) -> u16 {
    let PropertyValue::ApplicationData(b) = o.read_property(P::LAST_COMMAND_TIME, None).unwrap()
    else {
        panic!("typed time")
    };
    let (t, end) = decode_timestamp_choice(&b, 0).unwrap();
    assert_eq!(end, b.len());
    let BACnetTimeStamp::SequenceNumber(n) = t else {
        panic!("sequence")
    };
    n
}
fn objects() -> Vec<(Box<dyn BACnetObject>, PropertyValue)> {
    vec![
        (
            Box::new(AnalogOutputObject::new(1, "AO", EngineeringUnits::SQUARE_METERS).unwrap()),
            PropertyValue::Real(1.0),
        ),
        (
            Box::new(AnalogValueObject::new(1, "AV", EngineeringUnits::SQUARE_METERS).unwrap()),
            PropertyValue::Real(1.0),
        ),
        (
            Box::new(BinaryOutputObject::new(1, "BO").unwrap()),
            PropertyValue::Enumerated(1),
        ),
        (
            Box::new(BinaryValueObject::new(1, "BV").unwrap()),
            PropertyValue::Enumerated(1),
        ),
        (
            Box::new(MultiStateOutputObject::new(1, "MSO", 2).unwrap()),
            PropertyValue::Unsigned(2),
        ),
        (
            Box::new(MultiStateValueObject::new(1, "MSV", 2).unwrap()),
            PropertyValue::Unsigned(2),
        ),
    ]
}
fn snapshot(o: &dyn BACnetObject) -> Vec<PropertyValue> {
    [
        P::PRESENT_VALUE,
        P::PRIORITY_ARRAY,
        P::CURRENT_COMMAND_PRIORITY,
        P::VALUE_SOURCE,
        P::VALUE_SOURCE_ARRAY,
        P::LAST_COMMAND_TIME,
    ]
    .into_iter()
    .map(|p| o.read_property(p, None).unwrap())
    .collect()
}

#[test]
fn command_source_six_family_transition_projection_and_time() {
    for (mut o, value) in objects() {
        let owner = local(1, None);
        let initiator = local(
            1,
            Some(ObjectIdentifier::new(ObjectType::SCHEDULE, 3).unwrap()),
        );
        assert_eq!(time(o.as_ref()), 0);
        assert_eq!(source(o.as_ref(), None), BACnetValueSource::None);
        for (property, value) in [
            (P::PRESENT_VALUE, value.clone()),
            (P::VALUE_SOURCE, source_value(&BACnetValueSource::None)),
            (P::PRIORITY_ARRAY, value.clone()),
        ] {
            let before = snapshot(o.as_ref());
            assert!(o.write_property(property, None, value, Some(8)).is_err());
            assert_eq!(snapshot(o.as_ref()), before);
        }
        o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(8), &owner)
            .unwrap();
        assert_eq!(time(o.as_ref()), 1);
        assert_eq!(source(o.as_ref(), None), owner.published_source());
        o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(8), &owner)
            .unwrap();
        assert_eq!(time(o.as_ref()), 1);
        o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(16), &initiator)
            .unwrap();
        assert_eq!(time(o.as_ref()), 1);
        assert_eq!(source(o.as_ref(), Some(16)), initiator.published_source());
        o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(8), &initiator)
            .unwrap();
        assert_eq!(time(o.as_ref()), 2);
        o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(4), &owner)
            .unwrap();
        assert_eq!(time(o.as_ref()), 3);
        o.write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(4),
            &initiator,
        )
        .unwrap();
        assert_eq!(time(o.as_ref()), 4);
        assert_eq!(source(o.as_ref(), Some(4)), initiator.published_source());
        assert_eq!(source(o.as_ref(), None), initiator.published_source());
        // An authorized Device may assert any valid source, including a forwarded,
        // omitted-device object, broadcast-shaped address, or none; none proves ownership.
        let claims = [
            BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: Some(device(99)),
                object_identifier: device(77),
            }),
            BACnetValueSource::Object(BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: device(88),
            }),
            BACnetValueSource::Address(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::new(),
            }),
            BACnetValueSource::None,
        ];
        for claim in claims {
            o.write_property_from(P::VALUE_SOURCE, None, source_value(&claim), Some(8), &owner)
                .unwrap();
            assert_eq!(source(o.as_ref(), None), claim);
            assert_eq!(time(o.as_ref()), 4);
            let before = snapshot(o.as_ref());
            assert!(
                o.write_property_from(
                    P::VALUE_SOURCE,
                    None,
                    source_value(&owner.published_source()),
                    Some(8),
                    &local(99, None)
                )
                .is_err()
            );
            assert_eq!(snapshot(o.as_ref()), before);
        }
        o.write_property_from(
            P::VALUE_SOURCE,
            None,
            source_value(&owner.published_source()),
            Some(16),
            &owner,
        )
        .unwrap();
        assert_eq!(source(o.as_ref(), None), BACnetValueSource::None);
        assert_eq!(time(o.as_ref()), 4);
        o.write_property_from(P::PRESENT_VALUE, None, PropertyValue::Null, Some(8), &owner)
            .unwrap();
        assert_eq!(time(o.as_ref()), 5);
        assert_eq!(source(o.as_ref(), None), owner.published_source());
        o.write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(16),
            &owner,
        )
        .unwrap();
        assert_eq!(time(o.as_ref()), 6);
        assert_eq!(source(o.as_ref(), None), BACnetValueSource::None);
        let before = time(o.as_ref());
        o.write_property(P::RELINQUISH_DEFAULT, None, value, None)
            .unwrap();
        assert_eq!(time(o.as_ref()), before);
    }
}

#[test]
fn command_source_invalid_writes_are_atomic_and_metadata_is_paired() {
    for (mut o, value) in objects() {
        let owner = local(1, None);
        o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(8), &owner)
            .unwrap();
        for (p, i, v, priority, origin) in [
            (
                P::PRESENT_VALUE,
                None,
                value.clone(),
                Some(0),
                owner.clone(),
            ),
            (
                P::PRESENT_VALUE,
                None,
                value.clone(),
                Some(17),
                owner.clone(),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                value.clone(),
                Some(8),
                owner.clone(),
            ),
            (
                P::PRESENT_VALUE,
                None,
                PropertyValue::CharacterString("wrong".into()),
                Some(8),
                owner.clone(),
            ),
            (
                P::PRESENT_VALUE,
                None,
                value.clone(),
                Some(8),
                local(ObjectIdentifier::WILDCARD_INSTANCE, None),
            ),
            (
                P::VALUE_SOURCE,
                None,
                PropertyValue::ApplicationData(vec![0x08, 0x08]),
                Some(8),
                owner.clone(),
            ),
            (
                P::VALUE_SOURCE,
                None,
                PropertyValue::ApplicationData(vec![0x1e]),
                Some(8),
                owner.clone(),
            ),
            (
                P::VALUE_SOURCE,
                None,
                source_value(&BACnetValueSource::None),
                Some(7),
                owner.clone(),
            ),
            (
                P::VALUE_SOURCE,
                Some(1),
                source_value(&BACnetValueSource::None),
                Some(8),
                owner.clone(),
            ),
            (
                P::PRIORITY_ARRAY,
                Some(8),
                value.clone(),
                None,
                owner.clone(),
            ),
        ] {
            let before = snapshot(o.as_ref());
            assert!(o.write_property_from(p, i, v, priority, &origin).is_err());
            assert_eq!(snapshot(o.as_ref()), before);
        }
        for address in [
            BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::new(),
            },
            BACnetAddress {
                network_number: u16::MAX,
                mac_address: MacAddr::from_slice(&[1]),
            },
            BACnetAddress {
                network_number: 1,
                mac_address: MacAddr::from_slice(&[1; 256]),
            },
        ] {
            let invalid = CommandOrigin::Remote {
                actual_address: address,
                binding: CommandDeviceBinding::Unknown,
            };
            let before = snapshot(o.as_ref());
            assert!(
                o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(8), &invalid)
                    .is_err()
            );
            assert_eq!(snapshot(o.as_ref()), before);
        }
        assert_eq!(
            o.read_property(P::VALUE_SOURCE_ARRAY, Some(0)).unwrap(),
            PropertyValue::Unsigned(16)
        );
        assert!(o.read_property(P::VALUE_SOURCE_ARRAY, Some(17)).is_err());
        assert!(o.read_property(P::COMMAND_TIME_ARRAY, None).is_err());
        for p in [P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME] {
            let rows = o.property_metadata();
            let row = rows.iter().find(|r| r.property_identifier == p).unwrap();
            assert!(row.is_required());
            assert_eq!(row.write_capability.is_writable(), p == P::VALUE_SOURCE);
            assert!(o.property_list().contains(&p));
        }
    }
}

#[test]
fn command_source_owner_matrix_retains_original_token() {
    use CommandDeviceBinding::*;
    use bacnet_types::enums::EngineeringUnits;
    let states = [Unknown, Unique(device(1)), Unique(device(2)), Ambiguous];
    for old in states {
        for new in states {
            for same in [false, true] {
                let a = remote(1, old);
                let b = remote(if same { 1 } else { 2 }, new);
                let expected = match (old, new) {
                    (Ambiguous, _) | (_, Ambiguous) => false,
                    (Unique(a), Unique(b)) => a == b,
                    _ => same,
                };
                assert_eq!(
                    a.permits_correction_by(&b),
                    expected,
                    "{old:?} {new:?} {same}"
                );
            }
        }
    }
    assert!(local(1, None).permits_correction_by(&local(1, Some(device(7)))));
    assert!(!local(1, None).permits_correction_by(&local(2, None)));
    assert!(!local(1, None).permits_correction_by(&remote(1, Unique(device(1)))));
    let mut o = AnalogOutputObject::new(1, "AO", EngineeringUnits::SQUARE_METERS).unwrap();
    let original = remote(1, Unknown);
    o.write_property_from(
        P::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        Some(8),
        &original,
    )
    .unwrap();
    o.write_property_from(
        P::VALUE_SOURCE,
        None,
        source_value(&local(9, None).published_source()),
        Some(8),
        &remote(1, Unique(device(9))),
    )
    .unwrap();
    assert!(
        o.write_property_from(
            P::VALUE_SOURCE,
            None,
            source_value(&BACnetValueSource::None),
            Some(8),
            &remote(2, Unique(device(9)))
        )
        .is_err()
    );
    // A new command replaces the token and grants the new Device correction rights.
    o.write_property_from(
        P::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        Some(8),
        &remote(2, Unique(device(9))),
    )
    .unwrap();
    o.write_property_from(
        P::VALUE_SOURCE,
        None,
        source_value(&BACnetValueSource::None),
        Some(8),
        &remote(3, Unique(device(9))),
    )
    .unwrap();
}

/// Only the command's owner may correct a slot's source, and it is told so
/// before its value is looked at: the server reads a refusal of a NULL as
/// the wrong datatype as a success that changes nothing (#1396), which must
/// not reach a requester with no right to the slot.
#[test]
fn command_source_correction_refuses_a_stranger_before_its_value() {
    let code = |result: Result<(), Error>| match result {
        Err(Error::Remote(RemoteError::Error { code, .. })) => code,
        other => panic!("expected a refusal, got {other:?}"),
    };
    for (mut o, value) in objects() {
        let owner = local(1, None);
        o.write_property_from(P::PRESENT_VALUE, None, value, Some(8), &owner)
            .unwrap();
        let before = snapshot(&*o);
        // A lone NULL, as the server hands it over, and a value of no source
        // datatype at all.
        for written in [
            PropertyValue::ApplicationData(vec![0x00]),
            PropertyValue::Real(1.0),
        ] {
            // A stranger at the owned slot, and the owner at a slot no one
            // commands.
            for (origin, priority) in [(local(2, None), 8), (owner.clone(), 9)] {
                assert_eq!(
                    code(o.write_property_from(
                        P::VALUE_SOURCE,
                        None,
                        written.clone(),
                        Some(priority),
                        &origin,
                    )),
                    ErrorCode::WRITE_ACCESS_DENIED,
                    "{written:?} at {priority}"
                );
            }
            assert_eq!(
                code(o.write_property_from(P::VALUE_SOURCE, None, written, Some(8), &owner)),
                ErrorCode::INVALID_DATA_TYPE
            );
        }
        assert_eq!(snapshot(&*o), before);
    }
}

#[test]
fn remote_origin_mac_holds_to_the_bacnet_address_bound() {
    // #1156: a remote writer is published as an address Value_Source, which
    // encodes only when its MAC fits BACnetAddress::MAX_MAC_LEN octets. The
    // origin check keeps a longer one out, so nothing stored fails to read.
    let origin = |len: usize| CommandOrigin::Remote {
        actual_address: BACnetAddress {
            network_number: 7,
            mac_address: MacAddr::from_slice(&vec![0xA5; len]),
        },
        binding: CommandDeviceBinding::Unknown,
    };
    for (mut o, value) in objects() {
        let longest = origin(BACnetAddress::MAX_MAC_LEN);
        o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(8), &longest)
            .unwrap();
        let BACnetValueSource::Address(published) = source(o.as_ref(), None) else {
            panic!("address source")
        };
        assert_eq!(published.mac_address.len(), BACnetAddress::MAX_MAC_LEN);
        let too_long = origin(BACnetAddress::MAX_MAC_LEN + 1);
        assert!(too_long.validate().is_err());
        let before = snapshot(o.as_ref());
        assert!(
            o.write_property_from(P::PRESENT_VALUE, None, value.clone(), Some(7), &too_long)
                .is_err()
        );
        assert_eq!(snapshot(o.as_ref()), before);
    }
}
