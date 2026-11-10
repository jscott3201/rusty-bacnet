//! The write configuration file (#2243): what it round-trips, and the
//! files it refuses, every truncation among them.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetLightingCommand, BACnetTimerStateChangeValue,
};
use bacnet_types::enums::{LightingOperation, ObjectType};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use super::*;

struct FileCase {
    directory: PathBuf,
    path: PathBuf,
}

impl FileCase {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "rusty-write-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("config.bin");
        Self { directory, path }
    }

    fn store(&self) -> FileWriteConfigPersistence {
        FileWriteConfigPersistence::new(&self.path).unwrap()
    }

    fn octets(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap()
    }

    fn put(&self, octets: &[u8]) {
        std::fs::write(&self.path, octets).unwrap();
    }
}

impl Drop for FileCase {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn timer() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TIMER, 3).unwrap()
}

fn channel() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHANNEL, 3).unwrap()
}

fn reference(instance: u32, device: Option<u32>) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
        property_identifier: 85,
        property_array_index: (instance == 2).then_some(4),
        device_identifier: device.map(|d| ObjectIdentifier::new(ObjectType::DEVICE, d).unwrap()),
    }
}

/// A Timer set with each kind of value.
pub(crate) fn timer_sample() -> WriteConfigSnapshot {
    let date = Date {
        year: 126,
        month: 11,
        day: 2,
        day_of_week: 1,
    };
    let time = Time {
        hour: 7,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    WriteConfigSnapshot::Timer(TimerWriteConfig {
        state_change: Some(TimerStateChange {
            values: [
                BACnetTimerStateChangeValue::Primitive(PropertyValue::Real(21.5)),
                BACnetTimerStateChangeValue::NoValue,
                BACnetTimerStateChangeValue::Primitive(PropertyValue::Null),
                BACnetTimerStateChangeValue::DateTime(date, time),
                BACnetTimerStateChangeValue::Constructed(vec![0x21, 0x07]),
                BACnetTimerStateChangeValue::LightingCommand(BACnetLightingCommand {
                    operation: LightingOperation::FADE_TO,
                    target_level: Some(40.0),
                    ramp_rate: None,
                    step_increment: None,
                    fade_time: Some(2000),
                    priority: Some(6),
                }),
                BACnetTimerStateChangeValue::Primitive(PropertyValue::Unsigned(9)),
            ],
            references: vec![reference(1, None), reference(2, Some(77))],
            priority_for_writing: 9,
        }),
        default_timeout: Some(45_000),
        description: None,
    })
}

/// A Channel with every part saved.
pub(crate) fn channel_sample() -> WriteConfigSnapshot {
    WriteConfigSnapshot::Channel(ChannelWriteConfig {
        members: Some(ChannelMembers {
            references: vec![reference(1, None), reference(2, Some(8))],
            execution_delay: vec![0, 250],
        }),
        channel_number: Some(12),
        control_groups: Some(vec![3, 0, 70_000]),
        allow_group_delay_inhibit: Some(true),
        description: None,
    })
}

#[test]
fn each_object_types_configuration_round_trips() {
    for (oid, snapshot) in [
        (timer(), timer_sample()),
        (channel(), channel_sample()),
        (
            timer(),
            WriteConfigSnapshot::Timer(TimerWriteConfig::default()),
        ),
        (
            timer(),
            WriteConfigSnapshot::Timer(TimerWriteConfig {
                default_timeout: Some(u64::MAX),
                ..TimerWriteConfig::default()
            }),
        ),
        (
            channel(),
            WriteConfigSnapshot::Channel(ChannelWriteConfig::default()),
        ),
        (
            channel(),
            WriteConfigSnapshot::Channel(ChannelWriteConfig {
                control_groups: Some(vec![1]),
                ..ChannelWriteConfig::default()
            }),
        ),
    ] {
        let case = FileCase::new();
        let store = case.store();
        assert_eq!(store.load(oid).unwrap(), None);
        store.save(oid, &snapshot).unwrap();
        assert_eq!(store.load(oid).unwrap(), Some(snapshot));
    }
}

/// A file cut short anywhere is refused, as is one with an octet more.
#[test]
fn every_truncation_and_a_trailing_octet_are_refused() {
    for (oid, snapshot) in [(timer(), timer_sample()), (channel(), channel_sample())] {
        let case = FileCase::new();
        let store = case.store();
        store.save(oid, &snapshot).unwrap();
        let whole = case.octets();
        for length in 0..whole.len() {
            case.put(&whole[..length]);
            assert!(
                store.load(oid).is_err(),
                "{oid}: a file cut to {length} of {} octets loaded",
                whole.len()
            );
        }
        let mut longer = whole.clone();
        longer.push(0);
        case.put(&longer);
        assert!(store.load(oid).is_err());
        case.put(&whole);
        assert_eq!(store.load(oid).unwrap(), Some(snapshot));
    }
}

/// The header is 12 octets; the body's first octet names the object type,
/// and a Timer's set opens with its presence octet and priority.
#[test]
fn damaged_parts_are_refused() {
    let case = FileCase::new();
    let store = case.store();
    store.save(timer(), &timer_sample()).unwrap();
    let whole = case.octets();
    let with = |at: usize, octet: u8| {
        let mut damaged = whole.clone();
        damaged[at] = octet;
        case.put(&damaged);
        store.load(timer())
    };
    // A Channel's kind octet, an unknown one, the kind saved before
    // Default_Timeout was kept (whose body ends before the Default_Timeout
    // part), a presence octet of 2, priorities 0 and 17, and a
    // Default_Timeout presence octet of 2 (the ninth octet from the end).
    let timeout_presence = whole.len() - 9;
    for (at, octet) in [
        (12, 2),
        (12, 9),
        (12, 1),
        (13, 2),
        (14, 0),
        (14, 17),
        (timeout_presence, 2),
    ] {
        assert!(with(at, octet).is_err(), "octet {at} = {octet} loaded");
    }
    // Priority 16 is fine.
    assert!(with(14, 16).is_ok());

    // A Control_Groups size of zero.
    let case = FileCase::new();
    let store = case.store();
    let groups = WriteConfigSnapshot::Channel(ChannelWriteConfig {
        control_groups: Some(vec![5]),
        ..ChannelWriteConfig::default()
    });
    store.save(channel(), &groups).unwrap();
    let mut damaged = case.octets();
    // Body: kind, no members, no number, then the groups' presence and size.
    assert_eq!(damaged[12..17], [2, 0, 0, 1, 1]);
    damaged[16] = 0;
    case.put(&damaged);
    assert!(store.load(channel()).is_err());
}

#[test]
fn a_snapshot_a_load_would_refuse_is_never_saved() {
    let case = FileCase::new();
    let store = case.store();
    let WriteConfigSnapshot::Timer(mut config) = timer_sample() else {
        unreachable!()
    };
    config.state_change.as_mut().unwrap().priority_for_writing = 0;
    assert!(
        store
            .save(timer(), &WriteConfigSnapshot::Timer(config))
            .is_err()
    );
    // A Channel's configuration for a Timer, and the other way round.
    assert!(store.save(timer(), &channel_sample()).is_err());
    assert!(store.save(channel(), &timer_sample()).is_err());
    let mismatched = WriteConfigSnapshot::Channel(ChannelWriteConfig {
        members: Some(ChannelMembers {
            references: vec![reference(1, None)],
            execution_delay: vec![],
        }),
        ..ChannelWriteConfig::default()
    });
    assert!(store.save(channel(), &mismatched).is_err());
    let no_groups = WriteConfigSnapshot::Channel(ChannelWriteConfig {
        control_groups: Some(vec![]),
        ..ChannelWriteConfig::default()
    });
    assert!(store.save(channel(), &no_groups).is_err());
    assert!(!case.path.exists(), "a refused save wrote a file");
}

#[test]
fn another_objects_file_is_refused() {
    let case = FileCase::new();
    let store = case.store();
    store.save(timer(), &timer_sample()).unwrap();
    let other = ObjectIdentifier::new(ObjectType::TIMER, 4).unwrap();
    assert!(store.load(other).is_err());
}

/// A Timer's file saved before Default_Timeout was kept (#2312): kind 1,
/// its body ending after the set. It still loads, with no Default_Timeout
/// written, and saves back in the current kind.
#[test]
fn a_timer_file_from_before_default_timeout_still_loads() {
    let case = FileCase::new();
    let store = case.store();
    let WriteConfigSnapshot::Timer(mut config) = timer_sample() else {
        unreachable!()
    };
    config.default_timeout = None;
    let without = WriteConfigSnapshot::Timer(config);
    store.save(timer(), &without).unwrap();
    let mut old = case.octets();
    assert_eq!((old[12], old.last()), (3, Some(&0)));
    // The kind octet, and no Default_Timeout part.
    old[12] = 1;
    old.pop();
    case.put(&old);
    assert_eq!(store.load(timer()).unwrap(), Some(without.clone()));
    // Every truncation of the old layout is refused too.
    for length in 0..old.len() {
        case.put(&old[..length]);
        assert!(store.load(timer()).is_err(), "cut to {length} loaded");
    }
    case.put(&old);
    store.save(timer(), &without).unwrap();
    assert_eq!(case.octets()[12], 3);
    assert_eq!(store.load(timer()).unwrap(), Some(without));
}

/// A file the builder call refuses can't be dropped by
/// `clear_written_reporting`, which needs the object. An application
/// replaces it without loading it, with a save of nothing written, and the
/// object then opens on its configured values (docs/rust-api.md).
#[cfg(feature = "event-reporting")]
#[test]
fn a_file_that_does_not_load_is_replaced_by_a_save_of_nothing_written() {
    use crate::analog::AnalogInputObject;
    use crate::traits::BACnetObject;
    use alloc::sync::Arc;
    use bacnet_types::enums::EngineeringUnits;

    let case = FileCase::new();
    let open = || {
        let persistence: Arc<dyn WriteConfigPersistence> = Arc::new(case.store());
        AnalogInputObject::new(3, "AI", EngineeringUnits::DEGREES_CELSIUS)
            .and_then(|object| object.with_reporting_persistence(persistence))
    };
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let files: [&[u8]; 3] = [
        b"not a write configuration file",
        // This object's header, then a kind no build knows.
        &[b"RBNWCF01".as_slice(), &oid.encode(), &[200]].concat(),
        &[],
    ];
    for octets in files {
        case.put(octets);
        assert!(open().is_err(), "{octets:?} opened");
        assert_eq!(case.octets(), octets, "a refused build wrote");
        let nothing = WriteConfigSnapshot::Reporting(ReportingWriteConfig::default());
        case.store().save(oid, &nothing).unwrap();
        let object = open().unwrap();
        assert_eq!(object.written_reporting(), ReportingWriteConfig::default());
        assert_eq!(object.object_identifier(), oid);
        assert_eq!(case.store().load(oid).unwrap(), Some(nothing));
    }
}
