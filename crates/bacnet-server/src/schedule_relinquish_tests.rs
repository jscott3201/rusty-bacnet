//! A Schedule's NULL on its targets (#1416): it relinquishes the Schedule's
//! slot in a commandable Present_Value, and on a property that isn't
//! commandable and has no NULL in its datatype it is the no-op WriteProperty
//! makes of it, the target counting as one that took the write. A target
//! that refuses for any other reason still fails.
//!
//! A reference's array index is checked as WriteProperty checks it, ahead of
//! the value and the NULL rule (#1426): an index the property can't take
//! fails that target and leaves the property as it is. Such a refusal, like
//! a missing object or property, is reported as a reference the target can't
//! write (#1433).

use std::borrow::Cow;
use std::sync::Mutex;

use ScheduleTargetOutcome::{Accepted, Failed, ReferenceRefused};
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use super::*;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn sch() -> ObjectIdentifier {
    oid(ObjectType::SCHEDULE, 1)
}

fn ai() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

fn ao() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_OUTPUT, 2)
}

/// How the targets took each of a Schedule's writes, in order.
type Outcomes = Arc<Mutex<Vec<Vec<ScheduleTargetOutcome>>>>;

/// A Schedule that owes the writes it was built with, and keeps how the
/// targets took each.
struct OwingSchedule {
    owed: Vec<ScheduleWrite>,
    outcomes: Outcomes,
}

impl BACnetObject for OwingSchedule {
    fn object_identifier(&self) -> ObjectIdentifier {
        sch()
    }

    fn object_name(&self) -> &str {
        "SCH-1"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(sch()))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString("SCH-1".into()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::SCHEDULE.to_raw()))
            }
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }

    fn take_owed_schedule_writes(&mut self) -> Vec<ScheduleWrite> {
        std::mem::take(&mut self.owed)
    }

    fn complete_schedule_write(
        &mut self,
        _write: &ScheduleWrite,
        outcomes: &[ScheduleTargetOutcome],
    ) -> bool {
        self.outcomes.lock().unwrap().push(outcomes.to_vec());
        false
    }
}

fn reference(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(object, property.to_raw())
}

fn read(
    db: &ObjectDatabase,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    db.get(&object)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

#[tokio::test]
async fn a_schedule_null_relinquishes_a_commandable_target_and_leaves_the_others_as_they_are() {
    use PropertyIdentifier as P;
    use bacnet_types::enums::EngineeringUnits;
    let outcomes = Outcomes::default();
    // Both writes are owed ones, which go out without a clock: a Real at
    // priority 9, then the NULL that relinquishes it.
    let write = |value, references| ScheduleWrite {
        value,
        priority: 9,
        references,
        retry: false,
    };
    let schedule = OwingSchedule {
        owed: vec![
            write(
                PropertyValue::Real(5.0),
                vec![
                    reference(ao(), P::PRESENT_VALUE),
                    reference(ai(), P::COV_INCREMENT),
                ],
            ),
            write(
                PropertyValue::Null,
                vec![
                    // Commandable: the slot is relinquished.
                    reference(ao(), P::PRESENT_VALUE),
                    // Not commandable, and a REAL has no NULL: left as it is.
                    reference(ai(), P::COV_INCREMENT),
                    // Read-only: still refused.
                    reference(ai(), P::STATUS_FLAGS),
                    // No such object: a reference it can't write.
                    reference(oid(ObjectType::ANALOG_INPUT, 9), P::PRESENT_VALUE),
                ],
            ),
        ],
        outcomes: Arc::clone(&outcomes),
    };
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogOutputObject::new(2, "AO-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(schedule)).unwrap();
    let db = Arc::new(RwLock::new(db));

    tick_schedules(&db).await;

    assert_eq!(
        *outcomes.lock().unwrap(),
        [
            vec![Accepted, Accepted],
            vec![Accepted, Accepted, Failed, ReferenceRefused]
        ]
    );
    let db = db.read().await;
    assert_eq!(
        read(&db, ao(), P::PRIORITY_ARRAY, Some(9)),
        PropertyValue::Null
    );
    assert_eq!(
        read(&db, ao(), P::PRESENT_VALUE, None),
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        read(&db, ai(), P::COV_INCREMENT, None),
        PropertyValue::Real(5.0)
    );
}

fn indexed(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: u32,
) -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new_indexed(object, property.to_raw(), index)
}

#[tokio::test]
async fn a_schedule_target_index_is_checked_as_write_property_checks_it() {
    use PropertyIdentifier as P;
    use bacnet_types::enums::EngineeringUnits;
    let outcomes = Outcomes::default();
    let msv = oid(ObjectType::MULTI_STATE_VALUE, 1);
    let bv = oid(ObjectType::BINARY_VALUE, 1);
    let write = |value, references| ScheduleWrite {
        value,
        priority: 9,
        references,
        retry: false,
    };
    let schedule = OwingSchedule {
        owed: vec![
            write(
                PropertyValue::CharacterString("scheduled".into()),
                vec![
                    // Description is one string, not an array.
                    indexed(ai(), P::DESCRIPTION, 1),
                    indexed(msv, P::STATE_TEXT, 2),
                    // MSV-1 has three states.
                    indexed(msv, P::STATE_TEXT, 9),
                    // An Analog Input has no State_Text.
                    indexed(ai(), P::STATE_TEXT, 1),
                ],
            ),
            // A datatype Description refuses, and a NULL it would take as a
            // no-op: the index answers first, so the first is a reference
            // refusal and the second isn't taken.
            write(
                PropertyValue::Real(5.0),
                vec![indexed(ai(), P::DESCRIPTION, 1)],
            ),
            write(PropertyValue::Null, vec![indexed(ai(), P::DESCRIPTION, 1)]),
            write(
                PropertyValue::Enumerated(1),
                vec![indexed(bv, P::PRESENT_VALUE, 1)],
            ),
        ],
        outcomes: Arc::clone(&outcomes),
    };
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    db.add(Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()))
        .unwrap();
    db.add(Box::new(schedule)).unwrap();
    let left_alone = |db: &ObjectDatabase| {
        [
            read(db, ai(), P::DESCRIPTION, None),
            read(db, bv, P::PRESENT_VALUE, None),
            read(db, msv, P::STATE_TEXT, Some(1)),
            read(db, msv, P::STATE_TEXT, Some(3)),
        ]
    };
    let before = left_alone(&db);
    let db = Arc::new(RwLock::new(db));

    tick_schedules(&db).await;

    assert_eq!(
        *outcomes.lock().unwrap(),
        [
            vec![
                ReferenceRefused,
                Accepted,
                ReferenceRefused,
                ReferenceRefused
            ],
            vec![ReferenceRefused],
            vec![ReferenceRefused],
            vec![ReferenceRefused]
        ]
    );
    let db = db.read().await;
    assert_eq!(left_alone(&db), before);
    assert_eq!(
        read(&db, msv, P::STATE_TEXT, Some(2)),
        PropertyValue::CharacterString("scheduled".into())
    );
}
