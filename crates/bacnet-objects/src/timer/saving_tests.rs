//! A written State_Change_Values and List_Of_Object_Property_References kept
//! across a rebuild (#2243).

use alloc::sync::Arc;

use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetTimerStateChangeValue};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::{StateChangeValues, TimerObject};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::traits::BACnetObject;
use crate::write_config::test_storage::MemoryConfig;
use crate::write_config::{
    ChannelWriteConfig, TimerStateChange, TimerWriteConfig, WriteConfigPersistence,
    WriteConfigSnapshot,
};

const VALUES: PropertyIdentifier = PropertyIdentifier::STATE_CHANGE_VALUES;
const REFERENCES: PropertyIdentifier = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;
const PRIORITY: PropertyIdentifier = PropertyIdentifier::PRIORITY_FOR_WRITING;

fn values(first: f32) -> StateChangeValues {
    core::array::from_fn(|index| match index {
        0 => BACnetTimerStateChangeValue::Primitive(PropertyValue::Real(first)),
        _ => BACnetTimerStateChangeValue::NoValue,
    })
}

fn reference(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
        property_identifier: 85,
        property_array_index: None,
        device_identifier: None,
    }
}

fn references_value(references: &[BACnetDeviceObjectPropertyReference]) -> PropertyValue {
    let mut octets = BytesMut::new();
    for reference in references {
        encode_device_object_property_reference(&mut octets, reference);
    }
    PropertyValue::ApplicationData(octets.to_vec())
}

/// Timer 1 kept in `storage`, configured with `configured` at priority 9.
fn timer(storage: &Arc<MemoryConfig>, configured: f32) -> TimerObject {
    let persistence = Arc::clone(storage) as Arc<dyn WriteConfigPersistence>;
    let mut timer = TimerObject::with_persistence(1, "TMR", persistence).unwrap();
    timer
        .set_state_change(values(configured), vec![reference(1)], 9)
        .unwrap();
    timer
}

fn read(timer: &TimerObject, property: PropertyIdentifier, index: Option<u32>) -> PropertyValue {
    timer.read_property(property, index).unwrap()
}

fn is_operational_problem(error: &Error) -> bool {
    matches!(error, Error::Remote(RemoteError::Error { class, code })
        if *class == ErrorClass::DEVICE && *code == ErrorCode::OPERATIONAL_PROBLEM)
}

/// A write to each of the two, made where it is, survives a rebuild, with
/// the priority the set was served with; the configured set then gives way.
#[test]
fn a_written_set_survives_a_rebuild_and_wins_over_the_configured_one() {
    let storage = MemoryConfig::new();
    {
        let mut timer = timer(&storage, 1.0);
        assert!(!timer.state_change_saved());
        timer
            .write_property(VALUES, Some(1), PropertyValue::Real(42.0), None)
            .unwrap();
        timer
            .write_property(
                REFERENCES,
                None,
                references_value(&[reference(5), reference(6)]),
                None,
            )
            .unwrap();
        assert!(timer.state_change_saved());
        assert_eq!(storage.saves(), 2);
    }
    let rebuilt = timer(&storage, 3.0);
    assert!(rebuilt.state_change_saved());
    assert_eq!(read(&rebuilt, VALUES, Some(1)), PropertyValue::Real(42.0));
    assert_eq!(
        read(&rebuilt, REFERENCES, None),
        crate::device_reference::reference_list(&[reference(5), reference(6)])
    );
    assert_eq!(read(&rebuilt, PRIORITY, None), PropertyValue::Unsigned(9));
}

/// Nothing written: the configured set applies at each start and storage
/// keeps nothing.
#[test]
fn an_unwritten_set_is_the_configured_one_and_is_not_saved() {
    let storage = MemoryConfig::new();
    drop(timer(&storage, 1.0));
    assert_eq!(storage.saves(), 0);
    let rebuilt = timer(&storage, 3.0);
    assert!(!rebuilt.state_change_saved());
    assert_eq!(read(&rebuilt, VALUES, Some(1)), PropertyValue::Real(3.0));
}

#[test]
fn a_set_that_cannot_be_saved_is_refused_and_the_old_set_stays() {
    let storage = MemoryConfig::new();
    let mut timer = timer(&storage, 1.0);
    storage.fail(true);
    let error = timer
        .write_property(VALUES, Some(1), PropertyValue::Real(42.0), None)
        .unwrap_err();
    assert!(is_operational_problem(&error), "{error:?}");
    assert_eq!(read(&timer, VALUES, Some(1)), PropertyValue::Real(1.0));
    assert!(!timer.state_change_saved());
    // A write the Timer refuses anyway refuses as before, saving nothing.
    storage.fail(false);
    assert!(
        timer
            .write_property(VALUES, Some(9), PropertyValue::Real(1.0), None)
            .is_err()
    );
    assert_eq!(storage.saves(), 0);
}

/// The server's path: the save runs at the stage, and the write then takes
/// the saved set without saving again.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_write_takes_the_saved_set() {
    let storage = MemoryConfig::new();
    let mut timer = timer(&storage, 1.0);
    let write = PendingWrite {
        property: VALUES,
        array_index: Some(2),
        value: PropertyValue::Real(7.0),
    };
    let StageStep::Staged(wait) = timer.stage_writes(core::slice::from_ref(&write)) else {
        panic!("the write didn't stage");
    };
    wait.block();
    assert_eq!(storage.saves(), 1);
    // Saved, not served yet.
    assert_eq!(read(&timer, VALUES, Some(2)), read(&timer, VALUES, Some(3)));
    timer
        .write_property(write.property, write.array_index, write.value, None)
        .unwrap();
    timer.release_staged_write(&wait);
    assert_eq!(read(&timer, VALUES, Some(2)), PropertyValue::Real(7.0));
    timer.wait_for_saves();
    assert_eq!(storage.saves(), 1);
}

/// A staged value that isn't equal to itself is still the request's own:
/// a REAL that is no number, alone at an index and inside the whole array,
/// takes the save its stage made and saves nothing more (#2331). A
/// comparison by `==` took such a write for another request's and saved it
/// again where it was.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_value_that_is_not_a_number_takes_its_save() {
    let nan = PropertyValue::Real(f32::NAN);
    let mut array = vec![PropertyValue::Null; 7];
    array[4] = PropertyValue::Double(f64::NAN);
    array[6] = nan.clone();
    for (array_index, value) in [(Some(2), nan), (None, PropertyValue::List(array))] {
        let storage = MemoryConfig::new();
        let mut timer = timer(&storage, 1.0);
        let write = PendingWrite {
            property: VALUES,
            array_index,
            value,
        };
        let StageStep::Staged(wait) = timer.stage_writes(core::slice::from_ref(&write)) else {
            panic!("the write didn't stage");
        };
        wait.block();
        assert_eq!(storage.saves(), 1, "{array_index:?}");
        timer
            .write_property(write.property, write.array_index, write.value, None)
            .unwrap();
        timer.release_staged_write(&wait);
        timer.wait_for_saves();
        assert_eq!(storage.saves(), 1, "{array_index:?}: saved again");
        let PropertyValue::Real(served) = read(&timer, VALUES, array_index.or(Some(7))) else {
            panic!("a REAL was written");
        };
        assert!(served.is_nan(), "{array_index:?}");
    }
}

/// A staged set its request never makes is put back: storage then holds
/// what the Timer serves, here nothing written.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_set_never_made_puts_storage_back() {
    let storage = MemoryConfig::new();
    let mut timer = timer(&storage, 1.0);
    let write = PendingWrite {
        property: REFERENCES,
        array_index: None,
        value: references_value(&[reference(8)]),
    };
    let StageStep::Staged(wait) = timer.stage_writes(&[write]) else {
        panic!("the write didn't stage");
    };
    wait.block();
    timer.release_staged_write(&wait);
    timer.wait_for_saves();
    assert_eq!(
        storage.saved(),
        Some(WriteConfigSnapshot::Timer(TimerWriteConfig::default()))
    );
    assert!(!timer.state_change_saved());
}

#[cfg(feature = "durable-writes")]
#[test]
fn a_timer_without_the_set_stages_nothing() {
    let storage = MemoryConfig::new();
    let persistence = Arc::clone(&storage) as Arc<dyn WriteConfigPersistence>;
    let mut timer = TimerObject::with_persistence(1, "TMR", persistence).unwrap();
    let write = PendingWrite {
        property: VALUES,
        array_index: Some(1),
        value: PropertyValue::Real(1.0),
    };
    assert!(matches!(timer.stage_writes(&[write]), StageStep::Skip));
    // A Timer without storage offers no staging at all.
    assert!(
        TimerObject::new(2, "T2")
            .unwrap()
            .durable_writes()
            .is_none()
    );
}

/// Storage holding what the Timer can't serve refuses the build.
#[test]
fn a_saved_set_the_timer_would_refuse_fails_the_build() {
    let set = |priority_for_writing| {
        WriteConfigSnapshot::Timer(TimerWriteConfig {
            state_change: Some(TimerStateChange {
                values: values(1.0),
                references: vec![reference(1)],
                priority_for_writing,
            }),
            default_timeout: None,
            description: None,
        })
    };
    let build = |storage: Arc<MemoryConfig>| {
        TimerObject::with_persistence(1, "TMR", storage as Arc<dyn WriteConfigPersistence>)
    };
    assert!(build(MemoryConfig::holding(set(0))).is_err());
    assert!(
        build(MemoryConfig::holding(WriteConfigSnapshot::Channel(
            ChannelWriteConfig::default()
        )))
        .is_err()
    );
    let restored = build(MemoryConfig::holding(set(16))).unwrap();
    assert!(restored.state_change_saved());
    assert_eq!(read(&restored, PRIORITY, None), PropertyValue::Unsigned(16));
}

/// Clearing the written set lets the configured one apply again, now and
/// after a rebuild; a clear storage refuses keeps the written set.
#[test]
fn clearing_the_written_set_lets_the_configured_one_apply() {
    let storage = MemoryConfig::new();
    timer(&storage, 1.0)
        .write_property(VALUES, Some(1), PropertyValue::Real(42.0), None)
        .unwrap();
    let mut rebuilt = timer(&storage, 3.0);
    assert!(rebuilt.state_change_saved());
    storage.fail(true);
    assert!(rebuilt.clear_written_config().is_err());
    assert!(rebuilt.state_change_saved());
    storage.fail(false);
    rebuilt.clear_written_config().unwrap();
    assert!(!rebuilt.state_change_saved());
    assert_eq!(
        storage.saved(),
        Some(WriteConfigSnapshot::Timer(TimerWriteConfig::default()))
    );
    // Served as it was until the setter applies.
    assert_eq!(read(&rebuilt, VALUES, Some(1)), PropertyValue::Real(42.0));
    rebuilt
        .set_state_change(values(3.0), vec![reference(1)], 9)
        .unwrap();
    assert_eq!(read(&rebuilt, VALUES, Some(1)), PropertyValue::Real(3.0));
    drop(rebuilt);
    let again = timer(&storage, 5.0);
    assert_eq!(read(&again, VALUES, Some(1)), PropertyValue::Real(5.0));
}
