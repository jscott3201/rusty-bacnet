//! The reporting settings a write sets, kept on the object types with
//! intrinsic reporting beyond the analog, binary and multi-state ones
//! (#2331): Access Door and Access Zone, which hold them in their
//! `ChangeOfStateReporting`, and Event and Alert Enrollment, which hold them
//! in their own fields and have setters to guard.

use alloc::sync::Arc;

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::{ErrorClass, ErrorCode, NotifyType, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::reporting_tests::{covered, detection, persistence, refused_with};
use super::test_storage::MemoryConfig;
use super::{ReportingWriteConfig, WriteConfigSnapshot};
use crate::access_control::{AccessDoorObject, AccessZoneObject};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use crate::traits::BACnetObject;
use bacnet_types::enums::EventType;

const TYPES: [ObjectType; 4] = [
    ObjectType::ACCESS_DOOR,
    ObjectType::ACCESS_ZONE,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::ALERT_ENROLLMENT,
];

fn source() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn ee(storage: &Arc<MemoryConfig>) -> EventEnrollmentObject {
    EventEnrollmentObject::new(1, "EE", EventType::CHANGE_OF_STATE)
        .and_then(|object| object.with_reporting_persistence(persistence(storage)))
        .unwrap()
}

fn ae(storage: &Arc<MemoryConfig>) -> AlertEnrollmentObject {
    AlertEnrollmentObject::new(1, "AE", source())
        .and_then(|object| object.with_reporting_persistence(persistence(storage)))
        .unwrap()
}

/// Object 1 of `kind`, keeping its reporting settings in `storage`.
fn open(kind: ObjectType, storage: &Arc<MemoryConfig>) -> Box<dyn BACnetObject> {
    open_on(kind, persistence(storage))
}

fn open_on(
    kind: ObjectType,
    kept: Arc<dyn super::WriteConfigPersistence>,
) -> Box<dyn BACnetObject> {
    match kind {
        ObjectType::ACCESS_DOOR => Box::new(
            AccessDoorObject::new(1, "Door")
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::ACCESS_ZONE => Box::new(
            AccessZoneObject::new(1, "Zone")
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::EVENT_ENROLLMENT => Box::new(
            EventEnrollmentObject::new(1, "EE", EventType::CHANGE_OF_STATE)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        _ => Box::new(
            AlertEnrollmentObject::new(1, "AE", source())
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
    }
}

fn read(object: &dyn BACnetObject, property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn write(object: &mut dyn BACnetObject, property: P, value: PropertyValue) -> Result<(), Error> {
    object.write_property(property, None, value, None)
}

/// Every type here serves all seven rows; each written row saves once,
/// before it answers, and an object rebuilt on the storage serves it.
#[test]
fn each_written_row_survives_a_rebuild_on_each_type() {
    for kind in TYPES {
        let starts = detection(open(kind, &MemoryConfig::new()).as_ref());
        assert!(starts, "{kind:?} starts with detection on");
        for (property, index, value) in covered(starts) {
            let storage = MemoryConfig::new();
            let mut object = open(kind, &storage);
            let before = read(object.as_ref(), property);
            object.write_property(property, index, value, None).unwrap();
            assert_eq!(storage.saves(), 1, "{kind:?} {property:?} {index:?}");
            let written = read(object.as_ref(), property);
            assert_ne!(written, before, "{kind:?} {property:?} changed nothing");
            drop(object);
            let rebuilt = open(kind, &storage);
            assert_eq!(
                read(rebuilt.as_ref(), property),
                written,
                "{kind:?} {property:?} {index:?} after a rebuild"
            );
        }
    }
}

#[test]
fn a_write_whose_save_fails_is_refused_with_the_old_value_kept() {
    for kind in TYPES {
        let storage = MemoryConfig::new();
        let mut object = open(kind, &storage);
        storage.fail(true);
        for (property, index, value) in covered(true) {
            let before = read(object.as_ref(), property);
            let result = object.write_property(property, index, value, None);
            assert!(
                refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
                "{kind:?} {property:?}: {result:?}"
            );
            assert_eq!(read(object.as_ref(), property), before, "{kind:?}");
        }
        assert_eq!(storage.saved(), None, "{kind:?}");
    }
}

/// Without storage nothing is staged and nothing kept; with it, the object
/// lends its staging.
#[cfg(feature = "durable-writes")]
#[test]
fn only_an_object_with_storage_lends_its_staging() {
    for kind in TYPES {
        let mut kept = open(kind, &MemoryConfig::new());
        assert!(kept.durable_writes().is_some(), "{kind:?}");
    }
    let mut door = AccessDoorObject::new(1, "Door").unwrap();
    assert!(door.durable_writes().is_none());
    assert_eq!(door.written_reporting(), ReportingWriteConfig::default());
}

fn class(value: u64) -> PropertyValue {
    PropertyValue::Unsigned(value)
}

fn enable(bits: EventTransitionBits) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![bits.to_bacnet()],
    }
}

/// One guarded setter: the row it sets, a value a write saves there, and
/// the setter called with a value other than the saved one (`over`) and
/// with one other than the starting value (`apply`, serving `applied`).
pub(super) struct Guarded<T> {
    pub(super) row: P,
    pub(super) written: PropertyValue,
    pub(super) over: fn(&mut T),
    pub(super) apply: fn(&mut T),
    pub(super) applied: PropertyValue,
}

/// For each guarded row on its own: a write saves it, and on a rebuild its
/// setter gives way while every other row's setter still applies; after
/// the clear its setter applies again. A guard keyed on the wrong row fails
/// one of the two.
pub(super) fn each_setter_gives_way_to_its_own_row<T: BACnetObject>(
    open: fn(&Arc<MemoryConfig>) -> T,
    clear: fn(&mut T) -> Result<(), Error>,
    rows: &[Guarded<T>],
) {
    for saved in rows {
        let storage = MemoryConfig::new();
        let mut object = open(&storage);
        write(&mut object, saved.row, saved.written.clone()).unwrap();
        drop(object);

        let mut object = open(&storage);
        for row in rows {
            if row.row == saved.row {
                (row.over)(&mut object);
                assert_eq!(read(&object, row.row), saved.written, "{:?} kept", row.row);
            } else {
                (row.apply)(&mut object);
                assert_eq!(
                    read(&object, row.row),
                    row.applied,
                    "{:?} applies with {:?} saved",
                    row.row,
                    saved.row
                );
            }
        }
        clear(&mut object).unwrap();
        (saved.apply)(&mut object);
        assert_eq!(
            read(&object, saved.row),
            saved.applied,
            "{:?} cleared",
            saved.row
        );
    }
}

#[test]
fn each_event_enrollment_setter_gives_way_to_its_own_row_until_cleared() {
    each_setter_gives_way_to_its_own_row(
        ee,
        EventEnrollmentObject::clear_written_reporting,
        &[
            Guarded {
                row: P::NOTIFICATION_CLASS,
                written: class(9),
                over: |object| object.set_notification_class(3),
                apply: |object| object.set_notification_class(3),
                applied: class(3),
            },
            Guarded {
                row: P::EVENT_ENABLE,
                written: enable(EventTransitionBits::TO_FAULT),
                over: |object| object.set_event_enable(EventTransitionBits::TO_NORMAL),
                apply: |object| object.set_event_enable(EventTransitionBits::TO_NORMAL),
                applied: enable(EventTransitionBits::TO_NORMAL),
            },
            // Kept beside the reporting settings (#2364).
            Guarded {
                row: P::DESCRIPTION,
                written: PropertyValue::CharacterString("lobby".into()),
                over: |object| object.set_description("configured"),
                apply: |object| object.set_description("configured"),
                applied: PropertyValue::CharacterString("configured".into()),
            },
        ],
    );
}

#[test]
fn each_alert_enrollment_setter_gives_way_to_its_own_row_until_cleared() {
    each_setter_gives_way_to_its_own_row(
        ae,
        AlertEnrollmentObject::clear_written_reporting,
        &[
            Guarded {
                row: P::NOTIFICATION_CLASS,
                written: class(9),
                over: |object| object.set_notification_class(3),
                apply: |object| object.set_notification_class(3),
                applied: class(3),
            },
            Guarded {
                row: P::EVENT_ENABLE,
                written: enable(EventTransitionBits::TO_FAULT),
                over: |object| object.set_event_enable(EventTransitionBits::TO_NORMAL),
                apply: |object| object.set_event_enable(EventTransitionBits::TO_NORMAL),
                applied: enable(EventTransitionBits::TO_NORMAL),
            },
            // Detection starts on: the write saves it off, the setter over
            // it turns it on, and the setter that applies turns it off.
            Guarded {
                row: P::EVENT_DETECTION_ENABLE,
                written: PropertyValue::Boolean(false),
                over: |object| object.set_event_detection_enable(true),
                apply: |object| object.set_event_detection_enable(false),
                applied: PropertyValue::Boolean(false),
            },
        ],
    );
}

/// Only the setter gives way: a write of Event_Detection_Enable over a
/// saved one still lands, and the clear saves nothing written.
#[test]
fn an_alert_enrollment_write_over_a_saved_row_lands() {
    let storage = MemoryConfig::new();
    let mut object = ae(&storage);
    let detection = P::EVENT_DETECTION_ENABLE;
    write(&mut object, detection, PropertyValue::Boolean(false)).unwrap();
    drop(object);
    let mut object = ae(&storage);
    write(&mut object, detection, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(read(&object, detection), PropertyValue::Boolean(true));
    object.clear_written_reporting().unwrap();
    drop(object);
    assert_eq!(
        storage.saved(),
        Some(WriteConfigSnapshot::Reporting(
            ReportingWriteConfig::default()
        ))
    );
}

/// The Access objects have no setters for the reporting rows (their list
/// setters are in `access_parameters_tests`); their clear drops what was
/// saved, so a rebuild serves the defaults again.
#[test]
fn an_access_object_clear_drops_what_was_saved() {
    let storage = MemoryConfig::new();
    write(
        open(ObjectType::ACCESS_DOOR, &storage).as_mut(),
        P::NOTIFICATION_CLASS,
        class(9),
    )
    .unwrap();
    let mut door = AccessDoorObject::new(1, "Door")
        .and_then(|door| door.with_reporting_persistence(persistence(&storage)))
        .unwrap();
    assert_eq!(door.written_reporting().notification_class, Some(9));
    door.clear_written_reporting().unwrap();
    drop(door);
    let door = open(ObjectType::ACCESS_DOOR, &storage);
    assert_eq!(read(door.as_ref(), P::NOTIFICATION_CLASS), class(0));

    let storage = MemoryConfig::new();
    write(
        open(ObjectType::ACCESS_ZONE, &storage).as_mut(),
        P::NOTIFICATION_CLASS,
        class(9),
    )
    .unwrap();
    let mut zone = AccessZoneObject::new(1, "Zone")
        .and_then(|zone| zone.with_reporting_persistence(persistence(&storage)))
        .unwrap();
    assert_eq!(zone.written_reporting().notification_class, Some(9));
    zone.clear_written_reporting().unwrap();
    drop(zone);
    let zone = open(ObjectType::ACCESS_ZONE, &storage);
    assert_eq!(read(zone.as_ref(), P::NOTIFICATION_CLASS), class(0));
}

fn notify_type(notify_type: NotifyType) -> PendingWrite {
    PendingWrite {
        property: P::NOTIFY_TYPE,
        array_index: None,
        value: PropertyValue::Enumerated(notify_type.to_raw()),
    }
}

/// An Alert Enrollment takes only ALARM and EVENT as Notify_Type: a staged
/// ACK_NOTIFICATION stages nothing, as the object refuses it, and storage
/// holding one fails its build. An Event Enrollment takes all three.
#[test]
fn an_alert_enrollment_keeps_only_the_notify_types_it_takes() {
    let storage = MemoryConfig::new();
    let mut alert = ae(&storage);
    let ack = notify_type(NotifyType::ACK_NOTIFICATION);
    #[cfg(feature = "durable-writes")]
    assert!(matches!(
        alert.stage_writes(core::slice::from_ref(&ack)),
        StageStep::Skip
    ));
    let result = write(&mut alert, P::NOTIFY_TYPE, ack.value.clone());
    assert!(refused_with(
        &result,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE
    ));
    assert_eq!(storage.saves(), 0);

    #[cfg(feature = "durable-writes")]
    {
        let mut enrollment = ee(&storage);
        assert!(matches!(
            enrollment.stage_writes(core::slice::from_ref(&ack)),
            StageStep::Staged(_)
        ));
    }

    let holding = MemoryConfig::holding(WriteConfigSnapshot::Reporting(ReportingWriteConfig {
        notify_type: Some(NotifyType::ACK_NOTIFICATION),
        ..ReportingWriteConfig::default()
    }));
    let built = AlertEnrollmentObject::new(1, "AE", source())
        .and_then(|object| object.with_reporting_persistence(persistence(&holding)));
    assert!(matches!(built, Err(Error::Encoding(_))));
    let built = EventEnrollmentObject::new(1, "EE", EventType::CHANGE_OF_STATE)
        .and_then(|object| object.with_reporting_persistence(persistence(&holding)));
    assert_eq!(
        built.unwrap().written_reporting().notify_type,
        Some(NotifyType::ACK_NOTIFICATION)
    );
}

/// A staged write on an object with no Tags saves first, and the write
/// takes it without saving again.
#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_write_saves_first_and_the_write_takes_it() {
    let storage = MemoryConfig::new();
    let mut object = ae(&storage);
    let event = notify_type(NotifyType::EVENT);
    let StageStep::Staged(wait) = object.stage_writes(core::slice::from_ref(&event)) else {
        panic!("expected a staged write");
    };
    wait.block();
    assert_eq!(storage.saves(), 1);
    assert!(object.has_staged_write());
    write(&mut object, P::NOTIFY_TYPE, event.value).unwrap();
    object.release_staged_write(&wait);
    object.wait_for_reporting_saves();
    assert_eq!(storage.saves(), 1);
    assert_eq!(
        object.written_reporting().notify_type,
        Some(NotifyType::EVENT)
    );
}

/// A door's COV snapshot is a copy for reading: a write to it saves
/// nothing.
#[test]
fn a_door_snapshot_saves_nothing() {
    let storage = MemoryConfig::new();
    let mut door = AccessDoorObject::new(1, "Door")
        .and_then(|door| door.with_reporting_persistence(persistence(&storage)))
        .unwrap();
    let mut snapshot = crate::traits::Timed::cov_snapshot(&door).unwrap();
    write(snapshot.as_mut(), P::NOTIFICATION_CLASS, class(9)).unwrap();
    assert_eq!(storage.saves(), 0);
    write(&mut door, P::NOTIFICATION_CLASS, class(9)).unwrap();
    assert_eq!(storage.saves(), 1);
}

/// What the file backend writes for each type is the public encoding of
/// what the object keeps, and an object rebuilt on the file serves it.
#[cfg(feature = "std")]
#[test]
fn the_file_backend_writes_the_public_octets_for_each_type() {
    use crate::snapshot::write_config;
    use crate::write_config::FileWriteConfigPersistence;

    let directory = std::env::temp_dir().join(format!("rb-2331-types-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    // The inhibit takes a write only while detection is on and no
    // reference is set, so the reference and then detection go last.
    let mut writes = covered(true);
    writes.sort_by_key(|(property, _, _)| {
        (
            *property == P::EVENT_DETECTION_ENABLE,
            *property == P::EVENT_ALGORITHM_INHIBIT_REF,
        )
    });
    for kind in TYPES {
        // The type's number, not its Debug name: "::" isn't allowed in a
        // Windows file name.
        let path = directory.join(format!("type-{}", kind.to_raw()));
        let file = || -> Arc<dyn super::WriteConfigPersistence> {
            Arc::new(FileWriteConfigPersistence::new(&path).unwrap())
        };
        let mut object = open_on(kind, file());
        for (property, index, value) in writes.clone() {
            object.write_property(property, index, value, None).unwrap();
        }
        let served: Vec<_> = writes
            .iter()
            .map(|(property, _, _)| read(object.as_ref(), *property))
            .collect();
        let oid = object.object_identifier();
        drop(object);
        let octets = std::fs::read(&path).unwrap();
        let Ok(WriteConfigSnapshot::Reporting(saved)) = write_config::decode(oid, &octets) else {
            panic!("{kind:?}: not a reporting file");
        };
        assert_eq!(saved.event_detection_enable, Some(false), "{kind:?}");
        assert!(saved.event_algorithm_inhibit_ref.is_some(), "{kind:?}");
        let expected = write_config::encode(oid, &WriteConfigSnapshot::Reporting(saved)).unwrap();
        assert_eq!(octets, expected, "{kind:?}");
        let rebuilt = open_on(kind, file());
        for ((property, _, _), served) in writes.iter().zip(served) {
            assert_eq!(
                read(rebuilt.as_ref(), *property),
                served,
                "{kind:?} {property:?}"
            );
        }
    }
    std::fs::remove_dir_all(&directory).unwrap();
}
