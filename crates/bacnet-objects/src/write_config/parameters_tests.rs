//! The event algorithm parameters a write sets on an analog object, kept
//! with its reporting settings (#2331): each comes back on a rebuild and its
//! detector uses it, a write whose save fails is refused, storage holding
//! what the object refuses on the wire fails the build, the clear drops
//! them with the rest, and a request's writes save once. The binary and
//! multi-state objects' are in `state_parameters_tests` and
//! `state_parameter_rules_tests`.

use alloc::sync::Arc;

use bacnet_types::bitstring::LimitEnable;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, EventState, EventType, ObjectType,
    PropertyIdentifier as P,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::reporting_tests::{persistence, refused_with};
use super::test_storage::MemoryConfig;
use super::{AlgorithmWriteConfig, ReportingWriteConfig, WriteConfigSnapshot};
use crate::access_control::{AccessDoorObject, AccessZoneObject};
use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::BinaryValueObject;
#[cfg(feature = "durable-writes")]
use crate::durable::{PendingWrite, StageStep};
use crate::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use crate::traits::BACnetObject;

const ANALOG: [ObjectType; 3] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
];

/// Object 1 of `kind` as an application builds it at each start: it
/// configures a High_Limit of 50 and a Low_Limit of 5, then attaches
/// `storage`, if any.
fn open(kind: ObjectType, storage: Option<&Arc<MemoryConfig>>) -> Box<dyn BACnetObject> {
    try_open(kind, storage).unwrap()
}

fn try_open(
    kind: ObjectType,
    storage: Option<&Arc<MemoryConfig>>,
) -> Result<Box<dyn BACnetObject>, Error> {
    let units = EngineeringUnits::DEGREES_CELSIUS;
    macro_rules! build {
        ($object:ty, $name:literal) => {{
            let mut object = <$object>::new(1, $name, units)?;
            object.write_property(P::HIGH_LIMIT, None, PropertyValue::Real(50.0), None)?;
            object.write_property(P::LOW_LIMIT, None, PropertyValue::Real(5.0), None)?;
            let object = match storage {
                Some(storage) => object.with_reporting_persistence(persistence(storage))?,
                None => object,
            };
            Ok(Box::new(object))
        }};
    }
    match kind {
        ObjectType::ANALOG_INPUT => build!(AnalogInputObject, "AI"),
        ObjectType::ANALOG_OUTPUT => build!(AnalogOutputObject, "AO"),
        _ => build!(AnalogValueObject, "AV"),
    }
}

fn limit_enable(enable: LimitEnable) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 6,
        data: vec![enable.to_bacnet()],
    }
}

/// Each parameter's write, and what storage keeps of it alone.
fn covered() -> Vec<(P, PropertyValue, AlgorithmWriteConfig)> {
    let none = AlgorithmWriteConfig::default;
    vec![
        (
            P::TIME_DELAY,
            PropertyValue::Unsigned(7),
            AlgorithmWriteConfig {
                time_delay: Some(7),
                ..none()
            },
        ),
        (
            P::TIME_DELAY_NORMAL,
            PropertyValue::Unsigned(3),
            AlgorithmWriteConfig {
                time_delay_normal: Some(3),
                ..none()
            },
        ),
        (
            P::HIGH_LIMIT,
            PropertyValue::Real(80.0),
            AlgorithmWriteConfig {
                high_limit: Some(80.0),
                ..none()
            },
        ),
        (
            P::LOW_LIMIT,
            PropertyValue::Real(-10.0),
            AlgorithmWriteConfig {
                low_limit: Some(-10.0),
                ..none()
            },
        ),
        (
            P::DEADBAND,
            PropertyValue::Real(2.5),
            AlgorithmWriteConfig {
                deadband: Some(2.5),
                ..none()
            },
        ),
        (
            P::LIMIT_ENABLE,
            limit_enable(LimitEnable::all()),
            AlgorithmWriteConfig {
                limit_enable: Some(LimitEnable::all()),
                ..none()
            },
        ),
    ]
}

pub(super) fn saved(storage: &MemoryConfig) -> ReportingWriteConfig {
    match storage.saved() {
        Some(WriteConfigSnapshot::Reporting(saved)) => saved,
        other => panic!("expected reporting settings, got {other:?}"),
    }
}

pub(super) fn read(object: &dyn BACnetObject, property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

pub(super) fn write(object: &mut dyn BACnetObject, property: P, value: PropertyValue) {
    object.write_property(property, None, value, None).unwrap();
}

/// Give `object` the Present_Value `value`: an input's while out of
/// service, an output's or value's by command.
fn set_value(object: &mut dyn BACnetObject, value: f32) {
    let value = PropertyValue::Real(value);
    if object.object_identifier().object_type() == ObjectType::ANALOG_INPUT {
        write(object, P::OUT_OF_SERVICE, PropertyValue::Boolean(true));
        write(object, P::PRESENT_VALUE, value);
    } else {
        let origin = crate::command_source::test_origin();
        object
            .write_property_from(P::PRESENT_VALUE, None, value, Some(8), &origin)
            .unwrap();
    }
}

/// The Event_State the object's detector proposes for its Present_Value
/// now, if any.
pub(super) fn evaluated(object: &mut dyn BACnetObject) -> Option<EventState> {
    let reporting = object.event_reporting_mut().unwrap();
    reporting
        .evaluate_intrinsic_reporting()
        .map(|outcome| outcome.change.to)
}

/// One second passes on the detector's countdown.
pub(super) fn ticked(object: &mut dyn BACnetObject) -> Option<EventState> {
    let reporting = object.event_reporting_mut().unwrap();
    reporting
        .tick_intrinsic_reporting()
        .map(|outcome| outcome.change.to)
}

/// Each parameter written on each analog type saves once, before it
/// answers, storage keeps that parameter and no other, and an object
/// rebuilt on the storage serves it over what the application configures.
#[test]
fn each_written_parameter_survives_a_rebuild_and_only_it_is_kept() {
    for kind in ANALOG {
        for (property, value, kept) in covered() {
            let storage = MemoryConfig::new();
            let mut object = open(kind, Some(&storage));
            let before = read(object.as_ref(), property);
            write(object.as_mut(), property, value);
            assert_eq!(storage.saves(), 1, "{kind} {property:?}");
            let written = read(object.as_ref(), property);
            assert_ne!(written, before, "{kind} {property:?} changed nothing");
            let expected = ReportingWriteConfig {
                algorithm: kept,
                ..Default::default()
            };
            assert_eq!(saved(&storage), expected, "{kind} {property:?}");
            drop(object);
            let rebuilt = open(kind, Some(&storage));
            assert_eq!(read(rebuilt.as_ref(), property), written, "{kind}");
            // A parameter no write set is the application's, as before.
            if property != P::LOW_LIMIT {
                assert_eq!(
                    read(rebuilt.as_ref(), P::LOW_LIMIT),
                    PropertyValue::Real(5.0)
                );
            }
        }
    }
}

/// A rebuilt object's detector uses the restored parameters: the restored
/// limit, enabled by the restored Limit_Enable, raises HIGH_LIMIT once the
/// restored Time_Delay has run, counted by hand; the configured limit of 50
/// alone would have raised it at once and the unwritten defaults never.
#[test]
fn a_rebuilt_detector_uses_the_restored_parameters() {
    for kind in ANALOG {
        let storage = MemoryConfig::new();
        let mut object = open(kind, Some(&storage));
        write(object.as_mut(), P::HIGH_LIMIT, PropertyValue::Real(80.0));
        write(
            object.as_mut(),
            P::LIMIT_ENABLE,
            limit_enable(LimitEnable::HIGH_LIMIT_ENABLE),
        );
        write(object.as_mut(), P::TIME_DELAY, PropertyValue::Unsigned(2));
        write(
            object.as_mut(),
            P::TIME_DELAY_NORMAL,
            PropertyValue::Unsigned(0),
        );
        drop(object);

        let mut rebuilt = open(kind, Some(&storage));
        // Between the configured limit and the restored one: no alarm.
        set_value(rebuilt.as_mut(), 60.0);
        assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(ticked(rebuilt.as_mut()), None, "{kind}");
        // Past the restored limit: the restored delay of two seconds runs.
        set_value(rebuilt.as_mut(), 81.0);
        assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(ticked(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(
            ticked(rebuilt.as_mut()),
            Some(EventState::HIGH_LIMIT),
            "{kind}"
        );

        // Without the storage the same value raises nothing: Limit_Enable
        // starts empty.
        let mut plain = open(kind, None);
        set_value(plain.as_mut(), 81.0);
        assert_eq!(evaluated(plain.as_mut()), None, "{kind}");
        assert_eq!(ticked(plain.as_mut()), None, "{kind}");
    }
}

/// A parameter write whose save fails is refused with DEVICE /
/// OPERATIONAL_PROBLEM, and the old value stays: on the wire, in the
/// detector and in storage.
#[test]
fn a_parameter_write_whose_save_fails_keeps_the_old_value() {
    for kind in ANALOG {
        let storage = MemoryConfig::new();
        let mut object = open(kind, Some(&storage));
        write(
            object.as_mut(),
            P::LIMIT_ENABLE,
            limit_enable(LimitEnable::HIGH_LIMIT_ENABLE),
        );
        let kept = saved(&storage);
        storage.fail(true);
        for (property, value, _) in covered() {
            let before = read(object.as_ref(), property);
            if value == before {
                continue;
            }
            let result = object.write_property(property, None, value, None);
            assert!(
                refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
                "{kind} {property:?}: {result:?}"
            );
            assert_eq!(read(object.as_ref(), property), before, "{kind}");
        }
        assert_eq!(saved(&storage), kept, "{kind}");
        assert_eq!(storage.saves(), 1, "{kind}");
        // The detector still has the configured limit of 50 and no delay:
        // the refused 80 and the refused Time_Delay of 7 took no effect.
        set_value(object.as_mut(), 60.0);
        assert_eq!(
            evaluated(object.as_mut()),
            Some(EventState::HIGH_LIMIT),
            "{kind}"
        );
    }
}

/// A write the object refuses is refused as without storage and saves
/// nothing.
#[test]
fn a_refused_parameter_write_saves_nothing() {
    let refusals = [
        (P::HIGH_LIMIT, None, PropertyValue::Real(f32::NAN)),
        (P::LOW_LIMIT, None, PropertyValue::Real(f32::INFINITY)),
        (P::DEADBAND, None, PropertyValue::Real(-1.0)),
        (P::DEADBAND, None, PropertyValue::Unsigned(1)),
        (P::HIGH_LIMIT, None, PropertyValue::Null),
        (P::TIME_DELAY, None, PropertyValue::Unsigned(1 << 32)),
        (P::TIME_DELAY_NORMAL, None, PropertyValue::Real(1.0)),
        (P::TIME_DELAY, Some(1), PropertyValue::Unsigned(4)),
        (
            P::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xE0],
            },
        ),
    ];
    for kind in ANALOG {
        let storage = MemoryConfig::new();
        let mut kept = open(kind, Some(&storage));
        let mut plain = open(kind, None);
        for (property, index, value) in refusals.clone() {
            let before = read(kept.as_ref(), property);
            let refusal = plain
                .write_property(property, index, value.clone(), None)
                .unwrap_err();
            let result = kept.write_property(property, index, value.clone(), None);
            assert_eq!(
                result.unwrap_err().to_string(),
                refusal.to_string(),
                "{kind} {property:?} {value:?}"
            );
            assert_eq!(read(kept.as_ref(), property), before);
        }
        assert_eq!(storage.saves(), 0, "{kind}");
    }
}

/// The clear drops the written parameters with the reporting settings and
/// Description, in one save; the values served stay until the next start,
/// when what the application configures applies again.
#[test]
fn the_clear_drops_parameters_reporting_rows_and_description_together() {
    for kind in ANALOG {
        let storage = MemoryConfig::new();
        let mut object = open(kind, Some(&storage));
        write(object.as_mut(), P::HIGH_LIMIT, PropertyValue::Real(80.0));
        write(object.as_mut(), P::TIME_DELAY, PropertyValue::Unsigned(7));
        write(
            object.as_mut(),
            P::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(9),
        );
        let label = PropertyValue::CharacterString("lobby".into());
        write(object.as_mut(), P::DESCRIPTION, label);
        let kept = saved(&storage);
        assert_eq!(kept.algorithm.high_limit, Some(80.0));
        assert_eq!(kept.algorithm.time_delay, Some(7));
        assert_eq!(kept.notification_class, Some(9));
        assert_eq!(kept.description.as_deref(), Some("lobby"));
        drop(object);

        macro_rules! clear {
            ($object:ty, $name:literal) => {{
                let mut object = <$object>::new(1, $name, EngineeringUnits::DEGREES_CELSIUS)
                    .and_then(|object| object.with_reporting_persistence(persistence(&storage)))
                    .unwrap();
                assert_eq!(object.written_reporting(), kept);
                storage.fail(true);
                assert!(object.clear_written_reporting().is_err());
                assert_eq!(
                    object.written_reporting(),
                    kept,
                    "a failed clear keeps them"
                );
                storage.fail(false);
                object.clear_written_reporting().unwrap();
                assert_eq!(object.written_reporting(), ReportingWriteConfig::default());
                assert_eq!(read(&object, P::HIGH_LIMIT), PropertyValue::Real(80.0));
            }};
        }
        match kind {
            ObjectType::ANALOG_INPUT => clear!(AnalogInputObject, "AI"),
            ObjectType::ANALOG_OUTPUT => clear!(AnalogOutputObject, "AO"),
            _ => clear!(AnalogValueObject, "AV"),
        }
        assert_eq!(saved(&storage), ReportingWriteConfig::default(), "{kind}");
        let rebuilt = open(kind, Some(&storage));
        assert_eq!(
            read(rebuilt.as_ref(), P::HIGH_LIMIT),
            PropertyValue::Real(50.0)
        );
        assert_eq!(
            read(rebuilt.as_ref(), P::TIME_DELAY),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            read(rebuilt.as_ref(), P::NOTIFICATION_CLASS),
            PropertyValue::Unsigned(0)
        );
    }
}

/// Storage holding `algorithm` for an object of `kind`.
pub(super) fn holding(algorithm: AlgorithmWriteConfig) -> Arc<MemoryConfig> {
    MemoryConfig::holding(WriteConfigSnapshot::Reporting(ReportingWriteConfig {
        algorithm,
        ..Default::default()
    }))
}

/// A stored value is taken or refused as a write of it is: what the object
/// refuses on the wire fails the build, and what it takes is served.
#[test]
fn a_stored_parameter_is_validated_as_a_write_of_it_is() {
    let none = AlgorithmWriteConfig::default;
    let mut cases = Vec::new();
    for value in [
        80.0,
        -3.5,
        0.0,
        f32::MAX,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ] {
        cases.push((
            P::HIGH_LIMIT,
            PropertyValue::Real(value),
            AlgorithmWriteConfig {
                high_limit: Some(value),
                ..none()
            },
        ));
        cases.push((
            P::LOW_LIMIT,
            PropertyValue::Real(value),
            AlgorithmWriteConfig {
                low_limit: Some(value),
                ..none()
            },
        ));
        cases.push((
            P::DEADBAND,
            PropertyValue::Real(value),
            AlgorithmWriteConfig {
                deadband: Some(value),
                ..none()
            },
        ));
    }
    for delay in [0, 1, u32::MAX] {
        cases.push((
            P::TIME_DELAY,
            PropertyValue::Unsigned(u64::from(delay)),
            AlgorithmWriteConfig {
                time_delay: Some(delay),
                ..none()
            },
        ));
    }
    for kind in ANALOG {
        let mut refused = 0;
        for (property, value, stored) in cases.clone() {
            let written = open(kind, None).write_property(property, None, value.clone(), None);
            let built = try_open(kind, Some(&holding(stored)));
            assert_eq!(
                built.is_ok(),
                written.is_ok(),
                "{kind} {property:?} {value:?}: a write gives {written:?}"
            );
            match built {
                Ok(object) => assert_eq!(read(object.as_ref(), property), value),
                Err(error) => {
                    assert!(matches!(error, Error::Encoding(_)), "{error:?}");
                    refused += 1;
                }
            }
        }
        // Three values that aren't finite on three properties, and the
        // negative deadband.
        assert_eq!(refused, 10, "{kind}");
        // Bits no Limit_Enable write can carry.
        let bits = AlgorithmWriteConfig {
            limit_enable: Some(LimitEnable::from_bits_retain(0x04)),
            ..none()
        };
        assert!(try_open(kind, Some(&holding(bits))).is_err(), "{kind}");
    }
}

/// The four object types with a reporting store that keep no parameter in
/// this build.
const OTHERS: [ObjectType; 4] = [
    ObjectType::ACCESS_DOOR,
    ObjectType::ACCESS_ZONE,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::ALERT_ENROLLMENT,
];

/// Object 1 of `kind`, one of [`OTHERS`], keeping its reporting settings
/// in `storage`.
fn try_other(
    kind: ObjectType,
    storage: &Arc<MemoryConfig>,
) -> Result<Box<dyn BACnetObject>, Error> {
    let kept = persistence(storage);
    macro_rules! build {
        ($object:expr) => {
            Ok(Box::new($object?.with_reporting_persistence(kept)?))
        };
    }
    let source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)?;
    match kind {
        ObjectType::ACCESS_DOOR => build!(AccessDoorObject::new(1, "Door")),
        ObjectType::ACCESS_ZONE => build!(AccessZoneObject::new(1, "Zone")),
        ObjectType::EVENT_ENROLLMENT => build!(EventEnrollmentObject::new(
            1,
            "EE",
            EventType::CHANGE_OF_STATE
        )),
        _ => build!(AlertEnrollmentObject::new(1, "AE", source)),
    }
}

/// A type keeps only its own parameters. On each of the four other types
/// with a reporting store, storage holding any parameter fails the build,
/// and a write of a time delay is answered as without storage, saves
/// nothing and is gone after a rebuild: Time_Delay where the type has it,
/// Time_Delay_Normal on an Event Enrollment, and neither on an Alert
/// Enrollment, which refuses both.
#[test]
fn a_type_keeps_only_its_own_parameters() {
    for kind in OTHERS {
        let more = [
            AlgorithmWriteConfig {
                alarm_value: Some(bacnet_types::enums::BinaryPV::ACTIVE),
                ..Default::default()
            },
            AlgorithmWriteConfig {
                alarm_values: Some(vec![1]),
                ..Default::default()
            },
        ];
        let stored = covered().into_iter().map(|(_, _, stored)| stored);
        for stored in stored.chain(more) {
            let built = try_other(kind, &holding(stored));
            assert!(matches!(built, Err(Error::Encoding(_))), "{kind}");
        }
        let storage = MemoryConfig::new();
        let mut kept = try_other(kind, &storage).unwrap();
        let mut plain = try_other(kind, &MemoryConfig::new()).unwrap();
        // As built: an unwritten Time_Delay_Normal reads as Time_Delay, so
        // both are read before either is written.
        let delays = [P::TIME_DELAY, P::TIME_DELAY_NORMAL];
        let built = delays.map(|property| read_ok(kept.as_ref(), property));
        for (property, before) in delays.into_iter().zip(built) {
            let value = PropertyValue::Unsigned(7);
            let expected = plain.write_property(property, None, value.clone(), None);
            let result = kept.write_property(property, None, value.clone(), None);
            assert_eq!(result.is_ok(), expected.is_ok(), "{kind} {property:?}");
            if result.is_ok() {
                assert_eq!(read(kept.as_ref(), property), value, "{kind}");
            }
            assert_eq!(storage.saves(), 0, "{kind} {property:?}");
            let rebuilt = try_other(kind, &storage).unwrap();
            assert_eq!(
                read_ok(rebuilt.as_ref(), property),
                before,
                "{kind} {property:?} after a rebuild"
            );
        }
        let taken = delays
            .into_iter()
            .filter(|property| {
                read_ok(kept.as_ref(), *property) == Some(PropertyValue::Unsigned(7))
            })
            .count();
        let expected = match kind {
            ObjectType::ALERT_ENROLLMENT => 0,
            ObjectType::EVENT_ENROLLMENT => 1,
            _ => 2,
        };
        assert_eq!(taken, expected, "{kind}: the delays a write changed");
    }
    let object = BinaryValueObject::new(1, "BV")
        .and_then(|object| object.with_reporting_persistence(persistence(&MemoryConfig::new())))
        .unwrap();
    assert_eq!(object.written_reporting(), ReportingWriteConfig::default());
}

fn read_ok(object: &dyn BACnetObject, property: P) -> Option<PropertyValue> {
    object.read_property(property, None).ok()
}

#[cfg(feature = "durable-writes")]
pub(super) fn pending(property: P, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index: None,
        value,
    }
}

/// A request writing two parameters and a reporting row stages one save of
/// them all, ahead of the object serving any; its writes then take their
/// steps without saving again. When that save fails each write is refused
/// and the old values stay.
#[cfg(feature = "durable-writes")]
#[test]
fn a_request_of_parameters_and_a_reporting_row_saves_once() {
    let request = [
        pending(P::HIGH_LIMIT, PropertyValue::Real(80.0)),
        pending(P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
        pending(P::TIME_DELAY, PropertyValue::Unsigned(7)),
    ];
    for kind in ANALOG {
        for fails in [false, true] {
            let storage = MemoryConfig::new();
            let mut object = open(kind, Some(&storage));
            storage.fail(fails);
            let durable = object.durable_writes().unwrap();
            let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                panic!("{kind}: the request stages");
            };
            wait.block();
            assert_eq!(storage.saves(), usize::from(!fails), "{kind}");
            assert_eq!(
                read(object.as_ref(), P::HIGH_LIMIT),
                PropertyValue::Real(50.0)
            );
            for write in &request {
                let result = object.write_property(write.property, None, write.value.clone(), None);
                if fails {
                    assert!(
                        refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
                        "{kind} {:?}: {result:?}",
                        write.property
                    );
                } else {
                    result.unwrap();
                }
            }
            object.durable_writes().unwrap().release_staged_write(&wait);
            drop(object);
            assert_eq!(storage.saves(), usize::from(!fails), "{kind}");
            if fails {
                assert_eq!(storage.saved(), None, "{kind}");
                continue;
            }
            let kept = saved(&storage);
            assert_eq!(kept.notification_class, Some(9));
            let algorithm = AlgorithmWriteConfig {
                high_limit: Some(80.0),
                time_delay: Some(7),
                ..Default::default()
            };
            assert_eq!(kept.algorithm, algorithm, "{kind}");
        }
    }
}

/// A staged step refuses what the object refuses, so the request's later
/// writes aren't saved ahead of a write that will never be made.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_parameter_the_object_refuses_ends_the_steps() {
    let storage = MemoryConfig::new();
    let mut object = open(ObjectType::ANALOG_VALUE, Some(&storage));
    let request = [
        pending(P::TIME_DELAY, PropertyValue::Unsigned(7)),
        pending(P::DEADBAND, PropertyValue::Real(-1.0)),
        pending(P::HIGH_LIMIT, PropertyValue::Real(80.0)),
    ];
    let durable = object.durable_writes().unwrap();
    let StageStep::Staged(wait) = durable.stage_writes(&request) else {
        panic!("the request stages");
    };
    wait.block();
    let algorithm = AlgorithmWriteConfig {
        time_delay: Some(7),
        ..Default::default()
    };
    assert_eq!(saved(&storage).algorithm, algorithm);
    object.durable_writes().unwrap().release_staged_write(&wait);
}
