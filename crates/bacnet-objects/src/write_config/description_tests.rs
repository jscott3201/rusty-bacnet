//! A written Description kept in the write-configuration storage (#2364):
//! a Timer and a Channel keep it with their configuration, and the objects
//! with a reporting store with their reporting settings. Each comes back on
//! a rebuild, a write whose save fails is refused with the old value kept, a
//! NULL changes nothing and saves nothing, one past the cap is refused
//! before anything is saved, a request writing it beside another kept
//! property saves once, and the clear drops it.

use alloc::sync::Arc;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

use super::test_storage::MemoryConfig;
use super::{ChannelWriteConfig, TimerWriteConfig, WriteConfigPersistence, WriteConfigSnapshot};
use crate::channel::ChannelObject;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::timer::TimerObject;
use crate::traits::BACnetObject;

fn persistence(storage: &Arc<MemoryConfig>) -> Arc<dyn WriteConfigPersistence> {
    Arc::clone(storage) as Arc<dyn WriteConfigPersistence>
}

fn text(text: &str) -> PropertyValue {
    PropertyValue::CharacterString(text.into())
}

/// One past the cap a kept Description has.
fn too_long() -> PropertyValue {
    PropertyValue::CharacterString("d".repeat(16 * 1024 + 1))
}

fn description(object: &dyn BACnetObject) -> PropertyValue {
    object.read_property(P::DESCRIPTION, None).unwrap()
}

fn write(object: &mut dyn BACnetObject, property: P, value: PropertyValue) -> Result<(), Error> {
    object.write_property(property, None, value, None)
}

#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
fn refused_with(result: &Result<(), Error>, class: ErrorClass, code: ErrorCode) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Error { class: c, code: e }))
        if *c == class && *e == code)
}

#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
fn pending(property: P, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index: None,
        value,
    }
}

fn timer(storage: &Arc<MemoryConfig>) -> TimerObject {
    TimerObject::with_persistence(1, "TMR", persistence(storage)).unwrap()
}

#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
fn channel(storage: &Arc<MemoryConfig>) -> ChannelObject {
    ChannelObject::with_persistence(1, "CH", 4, persistence(storage)).unwrap()
}

#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
fn saved_description(storage: &MemoryConfig) -> Option<String> {
    storage
        .saved()
        .and_then(|saved| saved.description().map(Into::into))
}

/// Run `check` on a Timer, with Default_Timeout as the part it keeps beside
/// Description, and on a Channel, with Channel_Number.
#[cfg(feature = "durable-writes")]
fn on_each_config_object(
    check: impl Fn(&dyn Fn(&Arc<MemoryConfig>) -> Box<dyn KeptObject>, P, PropertyValue),
) {
    check(
        &|storage| Box::new(timer(storage)),
        P::DEFAULT_TIMEOUT,
        PropertyValue::Unsigned(5_000),
    );
    check(
        &|storage| Box::new(channel(storage)),
        P::CHANNEL_NUMBER,
        PropertyValue::Unsigned(77),
    );
}

/// What an object with storage needs beyond `BACnetObject`: its staging, when
/// the build has it.
#[cfg(feature = "durable-writes")]
trait Staging: DurableWrites {}
#[cfg(feature = "durable-writes")]
impl<T: DurableWrites> Staging for T {}
#[cfg(not(feature = "durable-writes"))]
trait Staging {}
#[cfg(not(feature = "durable-writes"))]
impl<T> Staging for T {}

/// An object with storage, as the server reaches it.
trait KeptObject: BACnetObject + Staging {
    fn object(&self) -> &dyn BACnetObject;
    fn object_mut(&mut self) -> &mut dyn BACnetObject;
}

impl<T: BACnetObject + Staging> KeptObject for T {
    fn object(&self) -> &dyn BACnetObject {
        self
    }

    fn object_mut(&mut self) -> &mut dyn BACnetObject {
        self
    }
}

#[test]
fn a_written_description_survives_a_rebuild() {
    on_each_config_object(|open, _, _| {
        let storage = MemoryConfig::new();
        let mut object = open(&storage);
        write(object.object_mut(), P::DESCRIPTION, text("lobby")).unwrap();
        assert_eq!(storage.saves(), 1);
        assert_eq!(description(object.object()), text("lobby"));
        drop(object);
        assert_eq!(description(open(&storage).object()), text("lobby"));
    });
}

#[test]
fn a_description_whose_save_fails_is_refused_with_the_old_value_kept() {
    on_each_config_object(|open, _, _| {
        let storage = MemoryConfig::new();
        let mut object = open(&storage);
        write(object.object_mut(), P::DESCRIPTION, text("lobby")).unwrap();
        storage.fail(true);
        let result = write(object.object_mut(), P::DESCRIPTION, text("hall"));
        assert!(refused_with(
            &result,
            ErrorClass::DEVICE,
            ErrorCode::OPERATIONAL_PROBLEM
        ));
        assert_eq!(description(object.object()), text("lobby"));
        assert_eq!(saved_description(&storage).as_deref(), Some("lobby"));
    });
}

/// A NULL changes nothing and saves nothing; one past the cap, or of
/// another type, is refused before anything is saved.
#[test]
fn a_null_saves_nothing_and_a_refused_description_saves_nothing() {
    on_each_config_object(|open, _, _| {
        let storage = MemoryConfig::new();
        let mut object = open(&storage);
        write(object.object_mut(), P::DESCRIPTION, text("lobby")).unwrap();
        write(object.object_mut(), P::DESCRIPTION, PropertyValue::Null).unwrap();
        assert_eq!(description(object.object()), text("lobby"));
        #[cfg(feature = "durable-writes")]
        {
            let null = pending(P::DESCRIPTION, PropertyValue::Null);
            assert!(matches!(object.stage_writes(&[null]), StageStep::Skip));
            let long = object.stage_writes(&[pending(P::DESCRIPTION, too_long())]);
            assert!(matches!(long, StageStep::Skip));
        }
        let result = write(object.object_mut(), P::DESCRIPTION, too_long());
        assert!(refused_with(
            &result,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
        ));
        let result = write(
            object.object_mut(),
            P::DESCRIPTION,
            PropertyValue::Unsigned(1),
        );
        assert!(refused_with(
            &result,
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE
        ));
        assert_eq!(storage.saves(), 1);
        assert_eq!(description(object.object()), text("lobby"));
    });
}

/// A request writing Description and another kept part stages one save,
/// and each write takes it without saving again.
#[cfg(feature = "durable-writes")]
#[test]
fn a_request_writing_description_and_another_part_saves_once() {
    on_each_config_object(|open, property, value| {
        let storage = MemoryConfig::new();
        let mut object = open(&storage);
        let writes = [
            pending(P::DESCRIPTION, text("lobby")),
            pending(property, value.clone()),
        ];
        let StageStep::Staged(wait) = object.stage_writes(&writes) else {
            panic!("{property:?}: expected a staged write");
        };
        wait.block();
        assert_eq!(storage.saves(), 1);
        for write in writes {
            object
                .object_mut()
                .write_property(write.property, None, write.value, None)
                .unwrap();
        }
        object.release_staged_write(&wait);
        drop(object);
        assert_eq!(storage.saves(), 1, "{property:?}");
        let rebuilt = open(&storage);
        assert_eq!(description(rebuilt.object()), text("lobby"));
        assert_eq!(
            rebuilt.object().read_property(property, None).unwrap(),
            value
        );
    });
}

/// `clear_written_config` drops a saved Description: the text served stays,
/// and a rebuild serves the application's again.
#[test]
fn clear_written_config_drops_the_description() {
    let storage = MemoryConfig::new();
    let mut timer = TimerObject::with_persistence(1, "TMR", persistence(&storage)).unwrap();
    write(&mut timer, P::DESCRIPTION, text("lobby")).unwrap();
    timer.clear_written_config().unwrap();
    assert_eq!(description(&timer), text("lobby"));
    assert_eq!(
        storage.saved(),
        Some(WriteConfigSnapshot::Timer(TimerWriteConfig::default()))
    );
    drop(timer);
    assert_eq!(description(&self::timer(&storage)), text(""));

    let storage = MemoryConfig::new();
    let mut channel = ChannelObject::with_persistence(1, "CH", 4, persistence(&storage)).unwrap();
    write(&mut channel, P::DESCRIPTION, text("lobby")).unwrap();
    channel.clear_written_config().unwrap();
    assert_eq!(description(&channel), text("lobby"));
    assert_eq!(
        storage.saved(),
        Some(WriteConfigSnapshot::Channel(ChannelWriteConfig::default()))
    );
}

/// Without storage a Description write is made where it is, as before.
#[test]
fn without_storage_nothing_is_kept() {
    let mut timer = TimerObject::new(1, "TMR").unwrap();
    write(&mut timer, P::DESCRIPTION, text("lobby")).unwrap();
    assert_eq!(description(&timer), text("lobby"));
    timer.clear_written_config().unwrap();
    let mut channel = ChannelObject::new(1, "CH", 4).unwrap();
    write(&mut channel, P::DESCRIPTION, too_long()).unwrap();
}

/// Storage holding a Description past the cap fails the build.
#[test]
fn a_saved_description_past_its_cap_fails_the_build() {
    let long = Some("d".repeat(16 * 1024 + 1));
    let timer_config = TimerWriteConfig {
        description: long.clone(),
        ..Default::default()
    };
    let storage = MemoryConfig::holding(WriteConfigSnapshot::Timer(timer_config));
    assert!(TimerObject::with_persistence(1, "TMR", persistence(&storage)).is_err());
    let channel_config = ChannelWriteConfig {
        description: long,
        ..Default::default()
    };
    let storage = MemoryConfig::holding(WriteConfigSnapshot::Channel(channel_config));
    assert!(ChannelObject::with_persistence(1, "CH", 4, persistence(&storage)).is_err());
}

/// The objects with a reporting store keep Description as one more row.
#[cfg(feature = "event-reporting")]
mod reporting {
    use super::*;
    use crate::analog::AnalogInputObject;
    use crate::write_config::ReportingWriteConfig;
    use bacnet_types::enums::EngineeringUnits;

    fn ai(storage: &Arc<MemoryConfig>) -> AnalogInputObject {
        AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS)
            .and_then(|object| object.with_reporting_persistence(persistence(storage)))
            .unwrap()
    }

    /// A saved Description wins over `set_description` while the other
    /// rows' setters still apply; after the clear the setter applies again.
    #[test]
    fn a_saved_description_wins_over_its_setter_until_cleared() {
        let storage = MemoryConfig::new();
        write(&mut ai(&storage), P::DESCRIPTION, text("lobby")).unwrap();
        let mut object = ai(&storage);
        object.set_description("configured");
        object.set_notification_class(3);
        assert_eq!(description(&object), text("lobby"));
        assert_eq!(
            object.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            object.written_reporting().description.as_deref(),
            Some("lobby")
        );
        object.clear_written_reporting().unwrap();
        assert_eq!(description(&object), text("lobby"));
        object.set_description("configured");
        assert_eq!(description(&object), text("configured"));
        drop(object);
        assert_eq!(
            storage.saved(),
            Some(WriteConfigSnapshot::Reporting(
                ReportingWriteConfig::default()
            ))
        );
    }

    /// A saved reporting row leaves the Description setter alone, and
    /// without storage the setter always applies.
    #[test]
    fn the_setter_applies_unless_a_description_is_saved() {
        let storage = MemoryConfig::new();
        let mut object = ai(&storage);
        write(
            &mut object,
            P::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(9),
        )
        .unwrap();
        object.set_description("configured");
        assert_eq!(description(&object), text("configured"));
        let mut plain = AnalogInputObject::new(2, "AI", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        write(&mut plain, P::DESCRIPTION, text("lobby")).unwrap();
        plain.set_description("configured");
        assert_eq!(description(&plain), text("configured"));
    }

    #[test]
    fn a_null_or_refused_description_saves_nothing() {
        let storage = MemoryConfig::new();
        let mut object = ai(&storage);
        write(&mut object, P::DESCRIPTION, PropertyValue::Null).unwrap();
        #[cfg(feature = "durable-writes")]
        {
            let null = pending(P::DESCRIPTION, PropertyValue::Null);
            assert!(matches!(object.stage_writes(&[null]), StageStep::Skip));
            let long = pending(P::DESCRIPTION, too_long());
            assert!(matches!(object.stage_writes(&[long]), StageStep::Skip));
        }
        let result = write(&mut object, P::DESCRIPTION, too_long());
        assert!(refused_with(
            &result,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
        ));
        assert_eq!(storage.saves(), 0);
        assert_eq!(object.written_reporting(), ReportingWriteConfig::default());
    }

    /// A long Description the object took before it had storage isn't
    /// kept, so it refuses no other row's write.
    #[test]
    fn a_long_description_taken_before_storage_refuses_no_other_row() {
        let storage = MemoryConfig::new();
        let mut object =
            AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        write(&mut object, P::DESCRIPTION, too_long()).unwrap();
        let mut object = object
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        write(
            &mut object,
            P::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(9),
        )
        .unwrap();
        assert_eq!(object.written_reporting().description, None);
        assert_eq!(storage.saves(), 1);
    }

    /// A request writing Description and a reporting setting stages one
    /// save, which both writes take.
    #[cfg(feature = "durable-writes")]
    #[test]
    fn a_request_writing_description_and_a_setting_saves_once() {
        let storage = MemoryConfig::new();
        let mut object = ai(&storage);
        let writes = [
            pending(P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
            pending(P::DESCRIPTION, text("lobby")),
        ];
        let StageStep::Staged(wait) = object.stage_writes(&writes) else {
            panic!("expected a staged write");
        };
        wait.block();
        assert_eq!(storage.saves(), 1);
        assert_eq!(description(&object), text(""));
        for write in writes {
            object
                .write_property(write.property, None, write.value, None)
                .unwrap();
        }
        object.release_staged_write(&wait);
        object.wait_for_reporting_saves();
        assert_eq!(storage.saves(), 1);
        let written = object.written_reporting();
        assert_eq!(written.description.as_deref(), Some("lobby"));
        assert_eq!(written.notification_class, Some(9));
        drop(object);
        assert_eq!(description(&ai(&storage)), text("lobby"));
    }

    #[test]
    fn a_saved_description_past_its_cap_fails_the_build() {
        let config = ReportingWriteConfig {
            description: Some("d".repeat(16 * 1024 + 1)),
            ..Default::default()
        };
        let storage = MemoryConfig::holding(WriteConfigSnapshot::Reporting(config));
        let built = AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS)
            .and_then(|object| object.with_reporting_persistence(persistence(&storage)));
        assert!(matches!(built, Err(Error::Encoding(_))));
    }
}
