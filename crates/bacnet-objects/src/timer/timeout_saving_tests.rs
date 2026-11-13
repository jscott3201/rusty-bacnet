//! A written Default_Timeout kept across a rebuild (#2312), and the saved
//! value that no longer fits the application's range.

use alloc::sync::Arc;

use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetTimerStateChangeValue};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{DEFAULT_TIMEOUT, StateChangeValues, TimerObject};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::traits::BACnetObject;
use crate::write_config::test_storage::MemoryConfig;
use crate::write_config::{TimerWriteConfig, WriteConfigPersistence, WriteConfigSnapshot};

const TIMEOUT: PropertyIdentifier = PropertyIdentifier::DEFAULT_TIMEOUT;
const VALUES: PropertyIdentifier = PropertyIdentifier::STATE_CHANGE_VALUES;

fn values(first: f32) -> StateChangeValues {
    core::array::from_fn(|index| match index {
        0 => BACnetTimerStateChangeValue::Primitive(PropertyValue::Real(first)),
        _ => BACnetTimerStateChangeValue::NoValue,
    })
}

fn reference() -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        property_identifier: 85,
        property_array_index: None,
        device_identifier: None,
    }
}

/// Timer 1 kept in `storage`, built as an application would: its set, then
/// `timeout` as its Default_Timeout.
pub(super) fn timer(storage: &Arc<MemoryConfig>, timeout: u64) -> TimerObject {
    let persistence = Arc::clone(storage) as Arc<dyn WriteConfigPersistence>;
    let mut timer = TimerObject::with_persistence(1, "TMR", persistence).unwrap();
    timer
        .set_state_change(values(1.0), vec![reference()], 9)
        .unwrap();
    timer.set_default_timeout(timeout).unwrap();
    timer
}

pub(super) fn timeout(timer: &TimerObject) -> PropertyValue {
    timer.read_property(TIMEOUT, None).unwrap()
}

pub(super) fn saved(storage: &MemoryConfig) -> TimerWriteConfig {
    match storage.saved() {
        Some(WriteConfigSnapshot::Timer(config)) => config,
        other => panic!("storage holds {other:?}"),
    }
}

#[test]
fn a_written_default_timeout_survives_a_rebuild_and_wins_over_the_setter() {
    let storage = MemoryConfig::new();
    {
        let mut timer = timer(&storage, 30_000);
        assert!(!timer.default_timeout_saved());
        timer
            .write_property(TIMEOUT, None, PropertyValue::Unsigned(5_000), None)
            .unwrap();
        assert!(timer.default_timeout_saved());
        assert!(!timer.state_change_saved());
    }
    assert_eq!(saved(&storage).default_timeout, Some(5_000));
    assert_eq!(saved(&storage).state_change, None);
    let mut rebuilt = timer(&storage, 30_000);
    assert!(rebuilt.default_timeout_saved());
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(5_000));
    // The setter checks its argument but doesn't take it.
    assert!(rebuilt.set_default_timeout(0).is_err());
    rebuilt.set_default_timeout(40_000).unwrap();
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(5_000));
    // It starts the countdown, too.
    rebuilt
        .write_property(
            PropertyIdentifier::TIMER_RUNNING,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert_eq!(
        rebuilt
            .read_property(PropertyIdentifier::INITIAL_TIMEOUT, None)
            .unwrap(),
        PropertyValue::Unsigned(5_000)
    );
}

/// The clear covers Default_Timeout with the set: the setters apply again,
/// now and after a rebuild.
#[test]
fn clearing_drops_a_written_default_timeout_with_the_set() {
    let storage = MemoryConfig::new();
    let mut first = timer(&storage, 30_000);
    first
        .write_property(TIMEOUT, None, PropertyValue::Unsigned(5_000), None)
        .unwrap();
    first
        .write_property(VALUES, Some(1), PropertyValue::Real(42.0), None)
        .unwrap();
    drop(first);
    let mut rebuilt = timer(&storage, 30_000);
    rebuilt.clear_written_config().unwrap();
    assert!(!rebuilt.default_timeout_saved());
    assert!(!rebuilt.state_change_saved());
    assert_eq!(saved(&storage), TimerWriteConfig::default());
    // Served as it was until the setter applies.
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(5_000));
    rebuilt.set_default_timeout(20_000).unwrap();
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(20_000));
    drop(rebuilt);
    assert_eq!(
        timeout(&timer(&storage, 30_000)),
        PropertyValue::Unsigned(30_000)
    );
}

#[test]
fn a_default_timeout_that_cannot_be_saved_is_refused_and_the_old_one_stays() {
    let storage = MemoryConfig::new();
    let mut timer = timer(&storage, 30_000);
    storage.fail(true);
    let error = timer
        .write_property(TIMEOUT, None, PropertyValue::Unsigned(5_000), None)
        .unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::DEVICE && code == ErrorCode::OPERATIONAL_PROBLEM),
        "{error:?}"
    );
    assert_eq!(timeout(&timer), PropertyValue::Unsigned(30_000));
    assert!(!timer.default_timeout_saved());
    // Out of range is refused as before, saving nothing.
    storage.fail(false);
    assert!(
        timer
            .write_property(TIMEOUT, None, PropertyValue::Unsigned(0), None)
            .is_err()
    );
    assert_eq!(storage.saves(), 0);
}

/// The server's path, on a Timer without the set: the write stages, saves
/// once, and then takes the saved value.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_default_timeout_write_takes_the_saved_value() {
    let storage = MemoryConfig::new();
    let persistence = Arc::clone(&storage) as Arc<dyn WriteConfigPersistence>;
    let mut timer = TimerObject::with_persistence(1, "TMR", persistence).unwrap();
    let write = PendingWrite {
        property: TIMEOUT,
        array_index: None,
        value: PropertyValue::Unsigned(7_000),
    };
    let StageStep::Staged(wait) = timer.stage_writes(core::slice::from_ref(&write)) else {
        panic!("the write didn't stage");
    };
    wait.block();
    assert_eq!(storage.saves(), 1);
    assert_eq!(timeout(&timer), PropertyValue::Unsigned(DEFAULT_TIMEOUT));
    timer
        .write_property(write.property, None, write.value, None)
        .unwrap();
    timer.release_staged_write(&wait);
    assert_eq!(timeout(&timer), PropertyValue::Unsigned(7_000));
    timer.wait_for_saves();
    assert_eq!(storage.saves(), 1);
    assert_eq!(saved(&storage).default_timeout, Some(7_000));
}

/// The lead's decision on #2312: a saved Default_Timeout the application's
/// range no longer holds is dropped for the application's value; the rest
/// of what was saved still applies, and storage is saved without it.
#[test]
fn a_saved_default_timeout_outside_a_narrowed_range_gives_way_to_the_applications() {
    let storage = MemoryConfig::new();
    let mut first = timer(&storage, 30_000);
    first
        .write_property(TIMEOUT, None, PropertyValue::Unsigned(5_000), None)
        .unwrap();
    first
        .write_property(VALUES, Some(1), PropertyValue::Real(42.0), None)
        .unwrap();
    drop(first);

    let mut rebuilt = timer(&storage, 30_000);
    // The application's own value must fit the range.
    assert!(rebuilt.set_pres_value_range(40_000, 100_000).is_err());
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(5_000));
    rebuilt.set_pres_value_range(10_000, 100_000).unwrap();
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(30_000));
    assert!(!rebuilt.default_timeout_saved());
    assert!(rebuilt.state_change_saved());
    assert_eq!(
        rebuilt.read_property(VALUES, Some(1)).unwrap(),
        PropertyValue::Real(42.0)
    );
    rebuilt.wait_for_saves();
    let kept = saved(&storage);
    assert_eq!(kept.default_timeout, None);
    assert!(kept.state_change.is_some());

    // A rebuild with the wide range keeps the application's value: the
    // dropped one doesn't come back.
    drop(rebuilt);
    assert_eq!(
        timeout(&timer(&storage, 20_000)),
        PropertyValue::Unsigned(20_000)
    );
}

/// A saved value that fits the narrowed range stays.
#[test]
fn a_saved_default_timeout_inside_a_narrowed_range_stays() {
    let storage = MemoryConfig::new();
    timer(&storage, 30_000)
        .write_property(TIMEOUT, None, PropertyValue::Unsigned(50_000), None)
        .unwrap();
    let mut rebuilt = timer(&storage, 30_000);
    rebuilt.set_pres_value_range(10_000, 100_000).unwrap();
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(50_000));
    assert!(rebuilt.default_timeout_saved());
}

/// Storage holding a value no Timer takes (zero) builds, serving the
/// default, and is saved without it.
#[test]
fn a_saved_default_timeout_no_timer_takes_is_dropped_at_the_build() {
    let storage = MemoryConfig::holding(WriteConfigSnapshot::Timer(TimerWriteConfig {
        default_timeout: Some(0),
        ..TimerWriteConfig::default()
    }));
    let persistence = Arc::clone(&storage) as Arc<dyn WriteConfigPersistence>;
    let timer = TimerObject::with_persistence(1, "TMR", persistence).unwrap();
    assert_eq!(timeout(&timer), PropertyValue::Unsigned(DEFAULT_TIMEOUT));
    assert!(!timer.default_timeout_saved());
    timer.wait_for_saves();
    assert_eq!(saved(&storage), TimerWriteConfig::default());
}

/// Past the range a new Timer takes (#2312 review): what the application's
/// range allows.
pub(super) const WIDE: u64 = 20_000_000_000;
pub(super) const LONG: u64 = 10_000_000_000;

/// Storage holding LONG, written under a range up to WIDE.
pub(super) fn holding_long() -> Arc<MemoryConfig> {
    let storage = MemoryConfig::new();
    let mut first = timer(&storage, 30_000);
    first.set_pres_value_range(1, WIDE).unwrap();
    first
        .write_property(TIMEOUT, None, PropertyValue::Unsigned(LONG), None)
        .unwrap();
    drop(first);
    assert_eq!(saved(&storage).default_timeout, Some(LONG));
    storage
}

/// A saved value past the range a new Timer takes is held at the build,
/// and served once the application's range holds it.
#[test]
fn a_saved_default_timeout_past_the_new_timer_range_comes_back_with_the_wide_range() {
    let storage = holding_long();
    let mut rebuilt = timer(&storage, 30_000);
    // Held: not served, still saved.
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(30_000));
    assert!(!rebuilt.default_timeout_saved());
    rebuilt.wait_for_saves();
    assert_eq!(saved(&storage).default_timeout, Some(LONG));
    rebuilt.set_pres_value_range(1, WIDE).unwrap();
    assert_eq!(timeout(&rebuilt), PropertyValue::Unsigned(LONG));
    assert!(rebuilt.default_timeout_saved());
    rebuilt.wait_for_saves();
    assert_eq!(saved(&storage).default_timeout, Some(LONG));
}

/// A save made while the value is held keeps it; a Timer whose range is
/// never set serves the application's value and keeps the held one saved.
#[test]
fn a_held_default_timeout_survives_saves_and_stays_unserved_without_a_range() {
    let storage = holding_long();
    let mut rebuilt = timer(&storage, 30_000);
    rebuilt
        .write_property(VALUES, Some(1), PropertyValue::Real(42.0), None)
        .unwrap();
    rebuilt.wait_for_saves();
    let kept = saved(&storage);
    assert_eq!(kept.default_timeout, Some(LONG));
    assert!(kept.state_change.is_some());
    drop(rebuilt);
    let unresolved = timer(&storage, 30_000);
    assert_eq!(timeout(&unresolved), PropertyValue::Unsigned(30_000));
    drop(unresolved);
    assert_eq!(saved(&storage).default_timeout, Some(LONG));
}

/// A held value the range set doesn't hold is dropped and saved without;
/// a peer's write replaces one, and the clear drops one.
#[test]
fn a_held_default_timeout_gives_way_to_a_range_a_write_or_a_clear() {
    let storage = holding_long();
    let mut narrowed = timer(&storage, 30_000);
    narrowed.set_pres_value_range(1, LONG - 1).unwrap();
    assert_eq!(timeout(&narrowed), PropertyValue::Unsigned(30_000));
    narrowed.wait_for_saves();
    assert_eq!(saved(&storage).default_timeout, None);

    let storage = holding_long();
    let mut written = timer(&storage, 30_000);
    written
        .write_property(TIMEOUT, None, PropertyValue::Unsigned(7_000), None)
        .unwrap();
    written.set_pres_value_range(1, WIDE).unwrap();
    assert_eq!(timeout(&written), PropertyValue::Unsigned(7_000));
    assert_eq!(saved(&storage).default_timeout, Some(7_000));

    let storage = holding_long();
    let mut cleared = timer(&storage, 30_000);
    cleared.clear_written_config().unwrap();
    assert_eq!(saved(&storage), TimerWriteConfig::default());
    cleared.set_pres_value_range(1, WIDE).unwrap();
    assert_eq!(timeout(&cleared), PropertyValue::Unsigned(30_000));
}

/// A saved value exactly at a narrowed bound stays; one past it goes.
#[test]
fn a_saved_default_timeout_at_a_narrowed_bound_stays_and_one_past_it_goes() {
    for (saved_value, kept) in [
        (100_000, true),
        (100_001, false),
        (10_000, true),
        (9_999, false),
    ] {
        let storage = MemoryConfig::new();
        timer(&storage, 30_000)
            .write_property(TIMEOUT, None, PropertyValue::Unsigned(saved_value), None)
            .unwrap();
        let mut rebuilt = timer(&storage, 30_000);
        rebuilt.set_pres_value_range(10_000, 100_000).unwrap();
        let expected = if kept { saved_value } else { 30_000 };
        assert_eq!(
            timeout(&rebuilt),
            PropertyValue::Unsigned(expected),
            "{saved_value}"
        );
        assert_eq!(rebuilt.default_timeout_saved(), kept, "{saved_value}");
    }
}
