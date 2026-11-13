//! The reporting settings a write sets on an intrinsic reporter, kept in its
//! write-configuration storage (#2331): each comes back on a rebuild, a write
//! whose save fails is refused, a saved row wins over its setter until the
//! clear, and a staged write saves before the object serves it.

use alloc::sync::Arc;

use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, NotifyType, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::test_storage::MemoryConfig;
use super::{ReportingWriteConfig, TimerWriteConfig, WriteConfigPersistence, WriteConfigSnapshot};
use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{DurableWrites, PendingWrite, STAGED_WRITE_LIFETIME, SaveWait, StageStep};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::traits::BACnetObject;

const TYPES: [ObjectType; 9] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
];

pub(super) fn persistence(storage: &Arc<MemoryConfig>) -> Arc<dyn WriteConfigPersistence> {
    Arc::clone(storage) as Arc<dyn WriteConfigPersistence>
}

/// Object 1 of `kind`, keeping its reporting settings in `storage`.
fn open(kind: ObjectType, storage: &Arc<MemoryConfig>) -> Box<dyn BACnetObject> {
    let units = EngineeringUnits::DEGREES_CELSIUS;
    let kept = persistence(storage);
    match kind {
        ObjectType::ANALOG_INPUT => Box::new(
            AnalogInputObject::new(1, "AI", units)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::ANALOG_OUTPUT => Box::new(
            AnalogOutputObject::new(1, "AO", units)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::ANALOG_VALUE => Box::new(
            AnalogValueObject::new(1, "AV", units)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::BINARY_INPUT => Box::new(
            BinaryInputObject::new(1, "BI")
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::BINARY_OUTPUT => Box::new(
            BinaryOutputObject::new(1, "BO")
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::BINARY_VALUE => Box::new(
            BinaryValueObject::new(1, "BV")
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::MULTI_STATE_INPUT => Box::new(
            MultiStateInputObject::new(1, "MSI", 3)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        ObjectType::MULTI_STATE_OUTPUT => Box::new(
            MultiStateOutputObject::new(1, "MSO", 3)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
        _ => Box::new(
            MultiStateValueObject::new(1, "MSV", 3)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap(),
        ),
    }
}

fn ai(storage: &Arc<MemoryConfig>) -> AnalogInputObject {
    AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS)
        .and_then(|object| object.with_reporting_persistence(persistence(storage)))
        .unwrap()
}

pub(super) fn reference_value() -> PropertyValue {
    let mut octets = BytesMut::new();
    encode_object_property_reference(
        &mut octets,
        &BACnetObjectPropertyReference::new(
            ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap(),
            P::PRESENT_VALUE.to_raw(),
        ),
    );
    PropertyValue::ApplicationData(octets.to_vec())
}

pub(super) fn texts(texts: [&str; 3]) -> PropertyValue {
    PropertyValue::List(
        texts
            .iter()
            .map(|text| PropertyValue::CharacterString((*text).into()))
            .collect(),
    )
}

/// Each kept row's write, Description included: the property, the index
/// and the value. The detection write turns it to the other value, read off
/// the object.
pub(super) fn covered(detection: bool) -> Vec<(P, Option<u32>, PropertyValue)> {
    vec![
        (P::NOTIFICATION_CLASS, None, PropertyValue::Unsigned(9)),
        (
            P::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_FAULT.to_bacnet()],
            },
        ),
        (
            P::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
        ),
        (
            P::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(!detection),
        ),
        (
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            None,
            texts(["off", "fault", "back"]),
        ),
        (
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            Some(3),
            PropertyValue::CharacterString("back".into()),
        ),
        (P::EVENT_ALGORITHM_INHIBIT_REF, None, reference_value()),
        (
            P::EVENT_ALGORITHM_INHIBIT,
            None,
            PropertyValue::Boolean(true),
        ),
        // Kept beside them (#2364).
        (
            P::DESCRIPTION,
            None,
            PropertyValue::CharacterString("lobby".into()),
        ),
    ]
}

pub(super) fn detection(object: &dyn BACnetObject) -> bool {
    object
        .read_property(P::EVENT_DETECTION_ENABLE, None)
        .unwrap()
        == PropertyValue::Boolean(true)
}

/// Each row written on each object type saves once, before it answers, and
/// an object rebuilt on the storage serves it. The inhibit is written with
/// detection on, as it must be; an Output, whose detection starts off,
/// turns it on first.
#[test]
fn each_written_row_survives_a_rebuild_on_each_type() {
    for kind in TYPES {
        let starts = detection(open(kind, &MemoryConfig::new()).as_ref());
        for (property, index, value) in covered(starts) {
            let storage = MemoryConfig::new();
            let mut object = open(kind, &storage);
            let mut saves = 0;
            if property == P::EVENT_ALGORITHM_INHIBIT && !starts {
                object
                    .write_property(
                        P::EVENT_DETECTION_ENABLE,
                        None,
                        PropertyValue::Boolean(true),
                        None,
                    )
                    .unwrap();
                saves += 1;
            }
            let before = object.read_property(property, None).unwrap();
            object.write_property(property, index, value, None).unwrap();
            saves += 1;
            assert_eq!(storage.saves(), saves, "{kind:?} {property:?} {index:?}");
            let written = object.read_property(property, None).unwrap();
            assert_ne!(
                written, before,
                "{kind:?} {property:?} {index:?} changed nothing"
            );
            drop(object);
            let rebuilt = open(kind, &storage);
            assert_eq!(
                rebuilt.read_property(property, None).unwrap(),
                written,
                "{kind:?} {property:?} {index:?} after a rebuild"
            );
        }
    }
}

pub(super) fn refused_with(result: &Result<(), Error>, class: ErrorClass, code: ErrorCode) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Error { class: c, code: e }))
        if *c == class && *e == code)
}

#[test]
fn a_write_whose_save_fails_is_refused_with_the_old_value_kept() {
    for kind in TYPES {
        let storage = MemoryConfig::new();
        let mut object = open(kind, &storage);
        let starts = detection(object.as_ref());
        storage.fail(true);
        for (property, index, value) in covered(starts) {
            if property == P::EVENT_ALGORITHM_INHIBIT && !starts {
                continue;
            }
            let before = object.read_property(property, None).unwrap();
            let result = object.write_property(property, index, value, None);
            assert!(
                refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
                "{kind:?} {property:?}: {result:?}"
            );
            assert_eq!(object.read_property(property, None).unwrap(), before);
        }
        assert_eq!(storage.saved(), None, "{kind:?}");
    }
}

/// A write the object refuses saves nothing, and an over-long message text
/// is refused before anything is saved, on any storage.
#[test]
fn a_refused_write_saves_nothing() {
    let storage = MemoryConfig::new();
    let mut object = ai(&storage);
    let refusals = [
        (P::NOTIFY_TYPE, PropertyValue::Enumerated(9)),
        (P::NOTIFICATION_CLASS, PropertyValue::Boolean(true)),
        (
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            texts(["x".repeat(16 * 1024 + 1).as_str(), "", ""]),
        ),
    ];
    for (property, value) in refusals {
        assert!(object.write_property(property, None, value, None).is_err());
    }
    let long = PropertyValue::CharacterString("y".repeat(16 * 1024 + 1));
    let result = object.write_property(P::EVENT_MESSAGE_TEXTS_CONFIG, Some(2), long, None);
    assert!(refused_with(
        &result,
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
    ));
    assert_eq!(storage.saves(), 0);
}

#[test]
fn a_saved_row_wins_over_its_setter_until_cleared() {
    let storage = MemoryConfig::new();
    let mut object = ai(&storage);
    object
        .write_property(
            P::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(9),
            None,
        )
        .unwrap();
    drop(object);

    // An application configuring the object at each start keeps the write.
    let mut object = ai(&storage);
    object.set_notification_class(3);
    // A row no write set still takes its setter.
    object.set_notify_type(NotifyType::EVENT);
    object.set_event_enable(EventTransitionBits::TO_OFFNORMAL);
    let read = |object: &AnalogInputObject, property| object.read_property(property, None).unwrap();
    assert_eq!(
        read(&object, P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(9)
    );
    assert_eq!(read(&object, P::NOTIFY_TYPE), PropertyValue::Enumerated(1));
    assert_eq!(object.written_reporting().notification_class, Some(9));
    assert_eq!(object.written_reporting().notify_type, None);

    // The clear saves the object with nothing written; the value served
    // stays until the setter changes it.
    object.clear_written_reporting().unwrap();
    assert_eq!(storage.saves(), 2);
    assert_eq!(
        storage.saved(),
        Some(WriteConfigSnapshot::Reporting(
            ReportingWriteConfig::default()
        ))
    );
    assert_eq!(
        read(&object, P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(9)
    );
    object.set_notification_class(3);
    assert_eq!(
        read(&object, P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(3)
    );
    drop(object);
    assert_eq!(
        read(&ai(&storage), P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn a_clear_whose_save_fails_keeps_the_written_rows() {
    let storage = MemoryConfig::new();
    let mut object = ai(&storage);
    object
        .write_property(
            P::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(9),
            None,
        )
        .unwrap();
    storage.fail(true);
    assert!(object.clear_written_reporting().is_err());
    object.set_notification_class(3);
    assert_eq!(object.written_reporting().notification_class, Some(9));
    assert_eq!(
        object.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
        PropertyValue::Unsigned(9)
    );
}

/// Without storage the setters always apply and there is nothing to clear.
#[test]
fn without_storage_the_setters_apply() {
    let mut object = BinaryValueObject::new(1, "BV").unwrap();
    object
        .write_property(
            P::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(9),
            None,
        )
        .unwrap();
    object.set_notification_class(3);
    assert_eq!(
        object.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
        PropertyValue::Unsigned(3)
    );
    object.clear_written_reporting().unwrap();
    assert_eq!(object.written_reporting(), ReportingWriteConfig::default());
    #[cfg(feature = "durable-writes")]
    assert!(object.durable_writes().is_none());
}

/// Storage holding another kind, or what no write leaves, fails the build.
#[test]
fn storage_holding_what_no_write_leaves_fails_the_build() {
    let timer = MemoryConfig::holding(WriteConfigSnapshot::Timer(TimerWriteConfig::default()));
    let unknown = MemoryConfig::holding(WriteConfigSnapshot::Reporting(ReportingWriteConfig {
        notify_type: Some(NotifyType::from_raw(5)),
        ..ReportingWriteConfig::default()
    }));
    for storage in [timer, unknown] {
        let built = AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS)
            .and_then(|object| object.with_reporting_persistence(persistence(&storage)));
        assert!(matches!(built, Err(Error::Encoding(_))));
    }
}

#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
fn class_write(class: u64) -> PendingWrite {
    PendingWrite {
        property: P::NOTIFICATION_CLASS,
        array_index: None,
        value: PropertyValue::Unsigned(class),
    }
}

#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
fn saved_class(storage: &MemoryConfig) -> Option<u32> {
    match storage.saved() {
        Some(WriteConfigSnapshot::Reporting(saved)) => saved.notification_class,
        _ => None,
    }
}

#[cfg(feature = "durable-writes")]
/// A request's writes stage one save before the object serves them; the
/// writes take it without saving again.
#[test]
fn a_staged_write_saves_first_and_the_write_takes_it() {
    let storage = MemoryConfig::new();
    let mut object = ai(&storage);
    let alarm = PendingWrite {
        property: P::NOTIFY_TYPE,
        array_index: None,
        value: PropertyValue::Enumerated(NotifyType::EVENT.to_raw()),
    };
    let wait = staged(object.stage_writes(&[class_write(9), alarm.clone()]));
    wait.block();
    assert_eq!(storage.saves(), 1);
    assert_eq!(saved_class(&storage), Some(9));
    assert_eq!(
        object.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert!(object.has_staged_write());
    for write in [class_write(9), alarm] {
        object
            .write_property(write.property, None, write.value, None)
            .unwrap();
    }
    object.release_staged_write(&wait);
    object.wait_for_reporting_saves();
    assert_eq!(storage.saves(), 1);
    assert!(!object.has_staged_write());
    assert_eq!(
        object.written_reporting().notify_type,
        Some(NotifyType::EVENT)
    );
}

#[cfg(feature = "durable-writes")]
#[test]
fn a_staged_write_whose_save_fails_is_refused() {
    let storage = MemoryConfig::new();
    let mut object = ai(&storage);
    storage.fail(true);
    let wait = staged(object.stage_writes(&[class_write(9)]));
    wait.block();
    let result = object.write_property(
        P::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(9),
        None,
    );
    assert!(refused_with(
        &result,
        ErrorClass::DEVICE,
        ErrorCode::OPERATIONAL_PROBLEM
    ));
    object.release_staged_write(&wait);
    assert_eq!(object.written_reporting(), ReportingWriteConfig::default());
}

#[cfg(feature = "durable-writes")]
/// A staged write its request never makes puts storage back to the rows
/// served: on release, on expiry, as the server stops, and as the object
/// drops.
#[test]
fn a_staged_write_never_made_puts_storage_back() {
    let storage = MemoryConfig::new();
    let mut object = ai(&storage);
    let wait = staged(object.stage_writes(&[class_write(9)]));
    wait.block();
    assert!(matches!(
        object.stage_writes(&[class_write(4)]),
        StageStep::Busy(_)
    ));
    object.release_staged_write(&wait);
    object.wait_for_reporting_saves();
    assert_eq!(saved_class(&storage), None);

    let start = core::time::Duration::from_secs(5);
    staged(object.stage_writes(&[class_write(9)])).block();
    object.expire_staged(start);
    object.expire_staged(start + STAGED_WRITE_LIFETIME);
    object.wait_for_reporting_saves();
    assert!(!object.has_staged_write());
    assert_eq!(saved_class(&storage), None);

    staged(object.stage_writes(&[class_write(9)])).block();
    object.settle_forgotten_writes().unwrap().block();
    assert_eq!(saved_class(&storage), None);

    staged(object.stage_writes(&[class_write(9)])).block();
    drop(object);
    assert_eq!(saved_class(&storage), None);
}

#[cfg(feature = "durable-writes")]
/// A write nobody staged supersedes a staged one: storage goes back to the
/// served rows, then takes the new one.
#[test]
fn a_write_nobody_staged_supersedes_a_staged_one() {
    let storage = MemoryConfig::new();
    let mut object = ai(&storage);
    let wait = staged(object.stage_writes(&[class_write(9)]));
    wait.block();
    object
        .write_property(
            P::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(4),
            None,
        )
        .unwrap();
    // The staged save, the one putting storage back, and the new write's.
    assert_eq!(storage.saves(), 3);
    assert_eq!(saved_class(&storage), Some(4));
    object
        .write_property(
            P::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(9),
            None,
        )
        .unwrap();
    object.release_staged_write(&wait);
    assert_eq!(saved_class(&storage), Some(9));
}

#[cfg(feature = "durable-writes")]
/// Writes of rows the object doesn't keep stage nothing, nor do writes of a
/// row it refuses.
#[test]
fn only_kept_rows_stage() {
    let storage = MemoryConfig::new();
    let mut object = ai(&storage);
    // High_Limit is kept now (#2331 PR 2); COV_Increment isn't.
    let increment = PendingWrite {
        property: P::COV_INCREMENT,
        array_index: None,
        value: PropertyValue::Real(0.5),
    };
    assert!(matches!(object.stage_writes(&[increment]), StageStep::Skip));
    let indexed = PendingWrite {
        array_index: Some(1),
        ..class_write(3)
    };
    assert!(matches!(object.stage_writes(&[indexed]), StageStep::Skip));
    assert_eq!(storage.saves(), 0);
}

/// A long message text the object took before it had storage isn't kept,
/// so it refuses no other row's write; a write that sets the texts, and so
/// would keep it, is refused (#2331).
#[test]
fn a_long_text_taken_before_storage_refuses_no_other_row() {
    let storage = MemoryConfig::new();
    let mut object = AnalogInputObject::new(1, "AI", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    let long = PropertyValue::CharacterString("x".repeat(16 * 1024 + 1));
    object
        .write_property(P::EVENT_MESSAGE_TEXTS_CONFIG, Some(2), long, None)
        .unwrap();
    let mut object = object
        .with_reporting_persistence(persistence(&storage))
        .unwrap();
    object
        .write_property(
            P::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(9),
            None,
        )
        .unwrap();
    assert_eq!(object.written_reporting().event_message_texts_config, None);
    let short = PropertyValue::CharacterString("back".into());
    let result = object.write_property(P::EVENT_MESSAGE_TEXTS_CONFIG, Some(3), short, None);
    assert!(refused_with(
        &result,
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
    ));
    assert_eq!(storage.saves(), 1);
}
