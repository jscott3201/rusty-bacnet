//! The format's parameter table against the objects (#2331). The table
//! (`snapshot::write_config::parameters`) makes the file's side of a row:
//! its bit, its octets, what it refuses and which types keep it. The
//! object's side is written by hand, per detector and per object: lending
//! the value to the store, taking a saved one back, and the write path a
//! staged step runs. This walks the table, so a row a type keeps there
//! and its object doesn't lend, restore or write fails here.

use alloc::sync::Arc;

use bacnet_types::bitstring::LimitEnable;
use bacnet_types::enums::{BinaryPV, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::parameters_tests::{holding, read, saved, write};
use super::test_storage::MemoryConfig;
use super::{
    AlgorithmWriteConfig, access_parameters_tests, enrollment_parameters_tests,
    state_parameters_tests,
};
use crate::snapshot::write_config::parameters as format;
use crate::traits::BACnetObject;

/// The thirteen object types with a reporting store.
const REPORTERS: [ObjectType; 13] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::ACCESS_DOOR,
    ObjectType::ACCESS_ZONE,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::ALERT_ENROLLMENT,
];

/// Object 1 of `kind`, a type that keeps a parameter, as its family's
/// tests build it.
fn try_open(
    kind: ObjectType,
    storage: Option<&Arc<MemoryConfig>>,
) -> Result<Box<dyn BACnetObject>, Error> {
    match kind {
        ObjectType::ANALOG_INPUT | ObjectType::ANALOG_OUTPUT | ObjectType::ANALOG_VALUE => {
            super::parameters_tests::try_open(kind, storage)
        }
        ObjectType::ACCESS_DOOR | ObjectType::ACCESS_ZONE => {
            access_parameters_tests::try_open(kind, storage)
        }
        ObjectType::EVENT_ENROLLMENT => enrollment_parameters_tests::try_open(storage)
            .map(|enrollment| Box::new(enrollment) as Box<dyn BACnetObject>),
        _ => state_parameters_tests::try_open(kind, storage),
    }
}

/// A value for the row of `property` on an object of `kind` that the object
/// doesn't start with: as storage holds it, and as the object serves it.
/// A row the table gains has no value here until it is given one, with its
/// object's side.
fn sample(kind: ObjectType, property: P) -> (AlgorithmWriteConfig, PropertyValue) {
    let mut stored = AlgorithmWriteConfig::default();
    let served = match property {
        P::TIME_DELAY => {
            stored.time_delay = Some(11);
            PropertyValue::Unsigned(11)
        }
        P::TIME_DELAY_NORMAL => {
            stored.time_delay_normal = Some(13);
            PropertyValue::Unsigned(13)
        }
        P::HIGH_LIMIT => {
            stored.high_limit = Some(77.5);
            PropertyValue::Real(77.5)
        }
        P::LOW_LIMIT => {
            stored.low_limit = Some(-3.25);
            PropertyValue::Real(-3.25)
        }
        P::DEADBAND => {
            stored.deadband = Some(1.75);
            PropertyValue::Real(1.75)
        }
        P::LIMIT_ENABLE => {
            stored.limit_enable = Some(LimitEnable::LOW_LIMIT_ENABLE);
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::LOW_LIMIT_ENABLE.to_bacnet()],
            }
        }
        P::ALARM_VALUE => {
            stored.alarm_value = Some(BinaryPV::INACTIVE);
            PropertyValue::Enumerated(0)
        }
        P::ALARM_VALUES => match kind {
            ObjectType::ACCESS_DOOR => {
                stored.alarm_values = Some(vec![4, 300]);
                access_parameters_tests::states(&[4, 300])
            }
            ObjectType::ACCESS_ZONE => {
                stored.alarm_values = Some(vec![3, 100]);
                access_parameters_tests::states(&[3, 100])
            }
            _ => {
                stored.alarm_values = Some(vec![4, 5]);
                state_parameters_tests::states(&[4, 5])
            }
        },
        P::FAULT_VALUES => {
            stored.fault_values = Some(vec![6, 301]);
            access_parameters_tests::states(&[6, 301])
        }
        P::MASKED_ALARM_VALUES => {
            stored.masked_alarm_values = Some(vec![7]);
            access_parameters_tests::states(&[7])
        }
        P::EVENT_PARAMETERS => {
            let parameters = enrollment_parameters_tests::state_parameters();
            let served = enrollment_parameters_tests::event_value(&parameters);
            stored.event_parameters = Some(parameters);
            served
        }
        P::FAULT_PARAMETERS => {
            let parameters = enrollment_parameters_tests::range_fault();
            let served = enrollment_parameters_tests::fault_value(&parameters);
            stored.fault_parameters = Some(parameters);
            served
        }
        P::STATE_TEXT => {
            let (_, served, kept) = state_parameters_tests::state_text_row();
            stored = kept;
            served
        }
        other => panic!("the table's row for {other:?} has no sample here"),
    };
    (stored, served)
}

/// Every row of the table, on every type the table keeps it for, goes
/// through that type's object all three ways. A saved value is served by an
/// object built on it. A save the object makes for another row still holds
/// it, read back from the object. And a write of it is taken, and saves
/// that row and no other.
#[test]
fn every_row_the_table_keeps_goes_through_its_objects() {
    let mut pairs = 0;
    for kind in REPORTERS {
        for &(bit, property) in format::ROWS {
            if format::kept(kind) & bit == 0 {
                continue;
            }
            pairs += 1;
            let what = format!("{kind} {property:?}");
            let (stored, value) = sample(kind, property);
            assert_eq!(format::mask(&stored), bit, "{what}");
            let plain = try_open(kind, None).unwrap();
            assert_ne!(read(plain.as_ref(), property), value, "{what}: no change");

            let storage = holding(stored.clone());
            let mut object = try_open(kind, Some(&storage))
                .unwrap_or_else(|error| panic!("{what}: a saved row doesn't load: {error:?}"));
            assert_eq!(read(object.as_ref(), property), value, "{what}: restored");
            write(
                object.as_mut(),
                P::NOTIFICATION_CLASS,
                PropertyValue::Unsigned(9),
            );
            assert_eq!(saved(&storage).algorithm, stored, "{what}: lent");
            drop(object);

            let storage = MemoryConfig::new();
            let mut object = try_open(kind, Some(&storage)).unwrap();
            object
                .write_property(property, None, value.clone(), None)
                .unwrap_or_else(|error| panic!("{what}: a write is refused: {error:?}"));
            assert_eq!(storage.saves(), 1, "{what}: written");
            assert_eq!(saved(&storage).algorithm, stored, "{what}: written");
            assert_eq!(read(object.as_ref(), property), value, "{what}: written");
        }
    }
    // Six rows on the three analog types, three on the binary Input and
    // Value, four on the multi-state ones (State_Text, #2414), two on the
    // Binary Output and three on the Multi-state Output, five on a door,
    // three on a zone and three on an Event Enrollment.
    assert_eq!(pairs, 3 * 6 + 2 * 3 + 2 * 4 + 2 + 3 + 5 + 3 + 3);
}

/// The one reporter the table keeps nothing for is the Alert Enrollment,
/// which has no algorithm parameter a peer writes, and no other type keeps
/// a row at all.
#[test]
fn the_table_keeps_rows_for_twelve_types() {
    let keepers = REPORTERS
        .into_iter()
        .filter(|kind| format::kept(*kind) != 0);
    assert_eq!(keepers.count(), 12);
    assert_eq!(format::kept(ObjectType::ALERT_ENROLLMENT), 0);
    for raw in 0..1024 {
        let kind = ObjectType::from_raw(raw);
        if !REPORTERS.contains(&kind) {
            assert_eq!(format::kept(kind), 0, "{kind}");
        }
    }
}
