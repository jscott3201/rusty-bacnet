//! The event algorithm parameters a write sets on an Event Enrollment, kept
//! with its reporting settings (#2331): Event_Parameters, Fault_Parameters
//! and Time_Delay_Normal. Each comes back on a rebuild; a write whose save
//! fails is refused; one the enrollment refuses saves nothing; the three
//! setters give way to a saved row until the clear; and a request's writes
//! save once. The cap, the file's octets and what a restore does are in
//! `enrollment_restore_tests`.

use alloc::sync::Arc;

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
    FaultParameters,
};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventType, FaultType, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::parameters_tests::{read, saved, write};
use super::reporting_tests::{persistence, refused_with};
use super::test_storage::MemoryConfig;
use super::{AlgorithmWriteConfig, ReportingWriteConfig, WriteConfigPersistence};
#[cfg(feature = "durable-writes")]
use crate::durable::StageStep;
use crate::event_enrollment::EventEnrollmentObject;
#[cfg(feature = "durable-writes")]
use crate::snapshot::enrollment_parameters_tests::extended_fault;
use crate::traits::BACnetObject;

/// The most octets of either parameter storage keeps.
pub(super) const CAP: usize = 64 * 1024;

pub(super) fn monitored(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    )
}

/// The Event_Parameters the application configures at each start.
fn configured() -> BACnetEventParameter {
    BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 5.0,
        high_limit: 50.0,
        deadband: 0.0,
    }
}

/// Event_Parameters of another algorithm than the configured one, with a
/// list.
pub(super) fn state_parameters() -> BACnetEventParameter {
    BACnetEventParameter::ChangeOfState {
        time_delay: 4,
        list_of_values: vec![
            BACnetPropertyStates::State(3),
            BACnetPropertyStates::State(7),
        ],
    }
}

pub(super) fn range_fault() -> FaultParameters {
    FaultParameters::FaultOutOfRange {
        min_normal: -5.0,
        max_normal: 90.0,
    }
}

/// `parameters` as Event_Parameters are served and written.
pub(super) fn event_value(parameters: &BACnetEventParameter) -> PropertyValue {
    let octets = crate::event_enrollment::parameters::event_parameter_octets(parameters).unwrap();
    PropertyValue::ApplicationData(octets.to_vec())
}

/// `parameters` as Fault_Parameters are served and written.
pub(super) fn fault_value(parameters: &FaultParameters) -> PropertyValue {
    let octets = crate::event_enrollment::parameters::fault_parameter_octets(parameters).unwrap();
    PropertyValue::ApplicationData(octets.to_vec())
}

/// Event Enrollment 1 as an application builds it at each start: it
/// monitors Analog Input 1 for OUT_OF_RANGE with [`configured`], then
/// attaches `storage`, if any.
pub(super) fn try_open(
    storage: Option<&Arc<MemoryConfig>>,
) -> Result<EventEnrollmentObject, Error> {
    built_on(storage.map(persistence))
}

pub(super) fn built_on(
    storage: Option<Arc<dyn WriteConfigPersistence>>,
) -> Result<EventEnrollmentObject, Error> {
    let mut enrollment = EventEnrollmentObject::new(1, "EE", EventType::OUT_OF_RANGE)?;
    enrollment.set_object_property_reference(Some(monitored(1)))?;
    enrollment.set_event_parameters(configured());
    match storage {
        Some(storage) => enrollment.with_reporting_persistence(storage),
        None => Ok(enrollment),
    }
}

pub(super) fn open(storage: Option<&Arc<MemoryConfig>>) -> EventEnrollmentObject {
    try_open(storage).unwrap()
}

/// Each row's write, and what storage keeps of it alone.
pub(super) fn covered() -> [(P, PropertyValue, AlgorithmWriteConfig); 3] {
    let none = AlgorithmWriteConfig::default;
    [
        (
            P::EVENT_PARAMETERS,
            event_value(&state_parameters()),
            AlgorithmWriteConfig {
                event_parameters: Some(state_parameters()),
                ..none()
            },
        ),
        (
            P::FAULT_PARAMETERS,
            fault_value(&range_fault()),
            AlgorithmWriteConfig {
                fault_parameters: Some(range_fault()),
                ..none()
            },
        ),
        (
            P::TIME_DELAY_NORMAL,
            PropertyValue::Unsigned(9),
            AlgorithmWriteConfig {
                time_delay_normal: Some(9),
                ..none()
            },
        ),
    ]
}

pub(super) fn enumerated(value: u32) -> PropertyValue {
    PropertyValue::Enumerated(value)
}

/// Each parameter written saves once, before it answers, storage keeps that
/// row and no other, and an enrollment rebuilt on the storage serves it
/// over what the application configures. Event_Type and Fault_Type follow
/// the restored parameters, as they follow written ones.
#[test]
fn each_written_parameter_survives_a_rebuild_and_only_it_is_kept() {
    for (property, value, kept) in covered() {
        let storage = MemoryConfig::new();
        let mut enrollment = open(Some(&storage));
        assert_ne!(read(&enrollment, property), value, "{property:?}");
        write(&mut enrollment, property, value.clone());
        assert_eq!(storage.saves(), 1, "{property:?}");
        assert_eq!(read(&enrollment, property), value, "{property:?}");
        let expected = ReportingWriteConfig {
            algorithm: kept.clone(),
            ..Default::default()
        };
        assert_eq!(saved(&storage), expected, "{property:?}");
        assert_eq!(enrollment.written_reporting(), expected, "{property:?}");
        drop(enrollment);

        let rebuilt = open(Some(&storage));
        assert_eq!(read(&rebuilt, property), value, "{property:?}");
        assert_eq!(rebuilt.written_reporting(), expected, "{property:?}");
        let (event_type, fault_type) = match property {
            P::EVENT_PARAMETERS => (EventType::CHANGE_OF_STATE, FaultType::NONE),
            P::FAULT_PARAMETERS => (EventType::OUT_OF_RANGE, FaultType::FAULT_OUT_OF_RANGE),
            _ => (EventType::OUT_OF_RANGE, FaultType::NONE),
        };
        assert_eq!(
            read(&rebuilt, P::EVENT_TYPE),
            enumerated(event_type.to_raw())
        );
        assert_eq!(
            read(&rebuilt, P::FAULT_TYPE),
            enumerated(fault_type.to_raw())
        );
        // A row no write set is the application's, as before.
        if property != P::EVENT_PARAMETERS {
            let served = read(&rebuilt, P::EVENT_PARAMETERS);
            assert_eq!(served, event_value(&configured()));
        }
    }
}

/// Time_Delay is no row of an enrollment: it is a member of
/// Event_Parameters and kept with them, and Time_Delay_Normal is kept only
/// once it is written itself, so neither is kept twice. Until then it reads
/// as the parameters' delay, the restored ones' too.
#[test]
fn time_delay_normal_is_kept_apart_from_the_delay_inside_the_parameters() {
    let storage = MemoryConfig::new();
    let mut enrollment = open(Some(&storage));
    write(
        &mut enrollment,
        P::EVENT_PARAMETERS,
        event_value(&state_parameters()),
    );
    assert_eq!(saved(&storage).algorithm.time_delay, None);
    assert_eq!(saved(&storage).algorithm.time_delay_normal, None);
    assert_eq!(
        read(&enrollment, P::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(4)
    );
    drop(enrollment);
    let mut rebuilt = open(Some(&storage));
    assert_eq!(
        read(&rebuilt, P::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(4)
    );
    assert_eq!(
        rebuilt.written_reporting().algorithm.time_delay_normal,
        None
    );

    write(
        &mut rebuilt,
        P::TIME_DELAY_NORMAL,
        PropertyValue::Unsigned(9),
    );
    let kept = saved(&storage).algorithm;
    assert_eq!(kept.time_delay, None);
    assert_eq!(kept.time_delay_normal, Some(9));
    assert_eq!(kept.event_parameters, Some(state_parameters()));
    drop(rebuilt);
    let mut rebuilt = open(Some(&storage));
    assert_eq!(
        read(&rebuilt, P::TIME_DELAY_NORMAL),
        PropertyValue::Unsigned(9)
    );
    assert_eq!(
        read(&rebuilt, P::EVENT_PARAMETERS),
        event_value(&state_parameters())
    );

    // The enrollment has no Time_Delay property: a write of it is refused
    // as without storage and saves nothing.
    let saves = storage.saves();
    let plain = open(None)
        .write_property(P::TIME_DELAY, None, PropertyValue::Unsigned(1), None)
        .unwrap_err();
    let kept = rebuilt
        .write_property(P::TIME_DELAY, None, PropertyValue::Unsigned(1), None)
        .unwrap_err();
    assert_eq!(kept.to_string(), plain.to_string());
    assert_eq!(storage.saves(), saves);
}

/// A parameter write whose save fails is refused with DEVICE /
/// OPERATIONAL_PROBLEM, and the old value stays: on the wire, in the types
/// that follow it, and in storage.
#[test]
fn a_parameter_write_whose_save_fails_keeps_the_old_value() {
    let storage = MemoryConfig::new();
    let mut enrollment = open(Some(&storage));
    write(
        &mut enrollment,
        P::TIME_DELAY_NORMAL,
        PropertyValue::Unsigned(2),
    );
    let kept = saved(&storage);
    storage.fail(true);
    for (property, value, _) in covered() {
        let before = read(&enrollment, property);
        let result = enrollment.write_property(property, None, value, None);
        assert!(
            refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
            "{property:?}: {result:?}"
        );
        assert_eq!(read(&enrollment, property), before, "{property:?}");
    }
    let event_type = enumerated(EventType::OUT_OF_RANGE.to_raw());
    assert_eq!(read(&enrollment, P::EVENT_TYPE), event_type);
    let fault_type = enumerated(FaultType::NONE.to_raw());
    assert_eq!(read(&enrollment, P::FAULT_TYPE), fault_type);
    assert_eq!(saved(&storage), kept);
    assert_eq!(enrollment.written_reporting(), kept);
    assert_eq!(storage.saves(), 1);
    // The setters still apply to the rows no write got through for.
    enrollment.set_event_parameters(state_parameters());
    assert_eq!(
        read(&enrollment, P::EVENT_PARAMETERS),
        event_value(&state_parameters())
    );
}

/// A write the enrollment refuses is refused as without storage and saves
/// nothing.
#[test]
fn a_refused_parameter_write_saves_nothing() {
    let data = PropertyValue::ApplicationData;
    let refusals = [
        // No value, a value cut short, octets after one, a tag the CHOICE
        // reserves, and an application value.
        (P::EVENT_PARAMETERS, None, data(vec![])),
        (P::EVENT_PARAMETERS, None, data(vec![0x5E, 0x09, 0x1E])),
        (P::EVENT_PARAMETERS, None, data(vec![0xF8, 20, 0x00])),
        (P::EVENT_PARAMETERS, None, data(vec![0x6E, 0x6F])),
        (P::EVENT_PARAMETERS, None, PropertyValue::Real(1.0)),
        (P::EVENT_PARAMETERS, None, PropertyValue::Null),
        // A string in a character set the stack can't convert.
        (
            P::EVENT_PARAMETERS,
            None,
            data(vec![
                0xFE, 0x11, 0x09, 0x00, 0x1E, 0x73, 2, 0x30, 0x42, 0x1F, 0xFF, 0x11,
            ]),
        ),
        (P::EVENT_PARAMETERS, Some(1), event_value(&configured())),
        (P::FAULT_PARAMETERS, None, data(vec![])),
        (P::FAULT_PARAMETERS, None, data(vec![0x8E, 0x8F])),
        (P::FAULT_PARAMETERS, None, data(vec![0x08, 0x00])),
        (P::FAULT_PARAMETERS, None, PropertyValue::Null),
        (P::FAULT_PARAMETERS, Some(0), fault_value(&range_fault())),
        (P::TIME_DELAY_NORMAL, None, PropertyValue::Real(1.0)),
        (P::TIME_DELAY_NORMAL, None, PropertyValue::Unsigned(1 << 32)),
        (P::TIME_DELAY_NORMAL, None, PropertyValue::Null),
        (P::TIME_DELAY_NORMAL, Some(1), PropertyValue::Unsigned(4)),
    ];
    let storage = MemoryConfig::new();
    let mut kept = open(Some(&storage));
    let mut plain = open(None);
    for (property, index, value) in refusals {
        let before = read(&kept, property);
        let refusal = plain
            .write_property(property, index, value.clone(), None)
            .unwrap_err();
        let result = kept.write_property(property, index, value.clone(), None);
        assert_eq!(
            result.unwrap_err().to_string(),
            refusal.to_string(),
            "{property:?} {value:?}"
        );
        assert_eq!(read(&kept, property), before, "{property:?} {value:?}");
    }
    assert_eq!(storage.saves(), 0);
    assert_eq!(kept.written_reporting(), ReportingWriteConfig::default());
}

/// A row with its written value, a setter call that would replace it, and
/// what the row serves once that setter applies.
struct Guarded {
    row: P,
    written: PropertyValue,
    set: fn(&mut EventEnrollmentObject),
    applied: PropertyValue,
}

fn buffer_ready() -> BACnetEventParameter {
    BACnetEventParameter::BufferReady {
        notification_threshold: 5,
        previous_notification_count: 0,
    }
}

fn flags_fault() -> FaultParameters {
    FaultParameters::FaultStatusFlags {
        reference: BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            P::STATUS_FLAGS.to_raw(),
        ),
    }
}

fn guarded() -> [Guarded; 3] {
    [
        Guarded {
            row: P::EVENT_PARAMETERS,
            written: event_value(&state_parameters()),
            set: |enrollment| enrollment.set_event_parameters(buffer_ready()),
            applied: event_value(&buffer_ready()),
        },
        Guarded {
            row: P::FAULT_PARAMETERS,
            written: fault_value(&range_fault()),
            set: |enrollment| enrollment.set_fault_parameters(Some(flags_fault())),
            applied: fault_value(&flags_fault()),
        },
        Guarded {
            row: P::TIME_DELAY_NORMAL,
            written: PropertyValue::Unsigned(9),
            set: |enrollment| enrollment.set_time_delay_normal(Some(77)),
            applied: PropertyValue::Unsigned(77),
        },
    ]
}

/// Each of the three setters gives way to its own saved row, on the
/// enrollment that took the write and on one rebuilt on its storage, while
/// the other two still apply; after `clear_written_reporting` it applies
/// again, and the next start serves what the application configures.
#[test]
fn each_setter_gives_way_to_its_own_saved_row_until_cleared() {
    for (index, row) in guarded().iter().enumerate() {
        let storage = MemoryConfig::new();
        let mut enrollment = open(Some(&storage));
        write(&mut enrollment, row.row, row.written.clone());
        let check = |enrollment: &mut EventEnrollmentObject, what: &str| {
            for other in &guarded() {
                (other.set)(enrollment);
            }
            for (at, other) in guarded().iter().enumerate() {
                let expected = if at == index {
                    &row.written
                } else {
                    &other.applied
                };
                let served = read(enrollment, other.row);
                assert_eq!(&served, expected, "{:?} {what}: {:?}", row.row, other.row);
            }
        };
        check(&mut enrollment, "on the written enrollment");
        // The forms that unset a row give way too.
        enrollment.set_fault_parameters(None);
        enrollment.set_time_delay_normal(None);
        assert_eq!(read(&enrollment, row.row), row.written, "{:?}", row.row);
        drop(enrollment);

        let mut rebuilt = open(Some(&storage));
        assert_eq!(read(&rebuilt, row.row), row.written, "{:?}", row.row);
        check(&mut rebuilt, "on a rebuilt enrollment");
        assert_eq!(storage.saves(), 1, "a setter saves nothing");

        storage.fail(true);
        assert!(rebuilt.clear_written_reporting().is_err());
        (row.set)(&mut rebuilt);
        assert_eq!(read(&rebuilt, row.row), row.written, "a failed clear");
        storage.fail(false);
        rebuilt.clear_written_reporting().unwrap();
        assert_eq!(read(&rebuilt, row.row), row.written, "served until changed");
        (row.set)(&mut rebuilt);
        assert_eq!(read(&rebuilt, row.row), row.applied, "{:?}", row.row);
        assert_eq!(saved(&storage), ReportingWriteConfig::default());
        drop(rebuilt);
        let plain = open(None);
        let restarted = open(Some(&storage));
        assert_eq!(read(&restarted, row.row), read(&plain, row.row));
    }
}

/// The clear drops the three rows with the reporting settings and
/// Description, in one save.
#[test]
fn the_clear_drops_the_parameters_with_the_rest() {
    let storage = MemoryConfig::new();
    let mut enrollment = open(Some(&storage));
    for (property, value, _) in covered() {
        write(&mut enrollment, property, value);
    }
    write(
        &mut enrollment,
        P::NOTIFICATION_CLASS,
        PropertyValue::Unsigned(9),
    );
    let label = PropertyValue::CharacterString("lobby".into());
    write(&mut enrollment, P::DESCRIPTION, label);
    let kept = saved(&storage);
    assert_eq!(kept.algorithm.event_parameters, Some(state_parameters()));
    assert_eq!(kept.algorithm.fault_parameters, Some(range_fault()));
    assert_eq!(kept.algorithm.time_delay_normal, Some(9));
    assert_eq!(kept.notification_class, Some(9));
    let saves = storage.saves();
    enrollment.clear_written_reporting().unwrap();
    assert_eq!(storage.saves(), saves + 1);
    assert_eq!(saved(&storage), ReportingWriteConfig::default());
    assert_eq!(
        enrollment.written_reporting(),
        ReportingWriteConfig::default()
    );
}

/// A request writing the three parameters and a reporting row stages one
/// save of them all, ahead of the enrollment serving any; its writes then
/// take their steps without saving again. When that save fails each write
/// is refused and the old values stay.
#[cfg(feature = "durable-writes")]
#[test]
fn a_request_of_parameters_and_a_reporting_row_saves_once() {
    use super::parameters_tests::pending;
    let mut request: Vec<_> = covered()
        .into_iter()
        .map(|(property, value, _)| pending(property, value))
        .collect();
    request.insert(
        1,
        pending(P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
    );
    for fails in [false, true] {
        let storage = MemoryConfig::new();
        let mut enrollment = open(Some(&storage));
        let before = request
            .iter()
            .map(|write| read(&enrollment, write.property))
            .collect::<Vec<_>>();
        storage.fail(fails);
        let durable = enrollment.durable_writes().unwrap();
        let StageStep::Staged(wait) = durable.stage_writes(&request) else {
            panic!("the request stages");
        };
        wait.block();
        assert_eq!(storage.saves(), usize::from(!fails));
        for (write, before) in request.iter().zip(&before) {
            assert_eq!(&read(&enrollment, write.property), before, "not yet served");
        }
        for (write, before) in request.iter().zip(&before) {
            let result = enrollment.write_property(write.property, None, write.value.clone(), None);
            if fails {
                assert!(
                    refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
                    "{:?}: {result:?}",
                    write.property
                );
                assert_eq!(&read(&enrollment, write.property), before);
            } else {
                result.unwrap();
                assert_eq!(read(&enrollment, write.property), write.value);
            }
        }
        enrollment
            .durable_writes()
            .unwrap()
            .release_staged_write(&wait);
        drop(enrollment);
        assert_eq!(storage.saves(), usize::from(!fails));
        if fails {
            assert_eq!(storage.saved(), None);
            continue;
        }
        let kept = saved(&storage);
        assert_eq!(kept.notification_class, Some(9));
        let algorithm = AlgorithmWriteConfig {
            event_parameters: Some(state_parameters()),
            fault_parameters: Some(range_fault()),
            time_delay_normal: Some(9),
            ..Default::default()
        };
        assert_eq!(kept.algorithm, algorithm);
    }
}

/// A staged step refuses what the enrollment refuses, a value past the cap
/// included, so the request's later writes aren't saved ahead of a write
/// that will never be made.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_parameter_past_the_cap_ends_the_steps() {
    use super::parameters_tests::pending;
    let storage = MemoryConfig::new();
    let mut enrollment = open(Some(&storage));
    let request = [
        pending(P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(7)),
        pending(P::FAULT_PARAMETERS, fault_value(&extended_fault(CAP + 1))),
        pending(P::EVENT_PARAMETERS, event_value(&state_parameters())),
    ];
    let durable = enrollment.durable_writes().unwrap();
    let StageStep::Staged(wait) = durable.stage_writes(&request) else {
        panic!("the request stages");
    };
    wait.block();
    let algorithm = AlgorithmWriteConfig {
        time_delay_normal: Some(7),
        ..Default::default()
    };
    assert_eq!(saved(&storage).algorithm, algorithm);
    assert_eq!(storage.saves(), 1);
    let [delay, fault, _] = &request;
    enrollment
        .write_property(delay.property, None, delay.value.clone(), None)
        .unwrap();
    let result = enrollment.write_property(fault.property, None, fault.value.clone(), None);
    assert!(
        refused_with(
            &result,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
        ),
        "{result:?}"
    );
    assert_eq!(storage.saves(), 1);
    assert_eq!(saved(&storage).algorithm, algorithm);
    enrollment
        .durable_writes()
        .unwrap()
        .release_staged_write(&wait);
}
