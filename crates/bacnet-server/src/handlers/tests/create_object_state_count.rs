//! Where a multi-state create's Number_Of_States applies, and what the
//! values that name a state are judged against (#1429).
//!
//! A count that passes its own checks applies before the other initial
//! values; one that fails them keeps its place in the list, so a bad value
//! before it is the one named. Alarm_Values entries are states too: one
//! past the count is VALUE_OUT_OF_RANGE at its element, over CreateObject
//! in either order and over WriteProperty.

use super::create_object_initial_values::{create, initial, read, text};
use super::*;
use PropertyIdentifier as P;
use bacnet_services::common::BACnetPropertyValue;

fn unsigned(value: u64) -> PropertyValue {
    PropertyValue::Unsigned(value)
}

fn count(value: u64) -> BACnetPropertyValue {
    initial(P::NUMBER_OF_STATES, None, &unsigned(value))
}

fn alarms(states: &[u64]) -> BACnetPropertyValue {
    let list = PropertyValue::List(states.iter().copied().map(unsigned).collect());
    initial(P::ALARM_VALUES, None, &list)
}

/// The class, code and position a create is refused with; nothing is left.
fn refused(
    db: &mut ObjectDatabase,
    object_type: ObjectType,
    values: Vec<BACnetPropertyValue>,
) -> (ErrorClass, ErrorCode, u32) {
    let before = db.len();
    let refusal = list_refusal(create(db, object_type, values).map(|_| ()));
    assert_eq!(db.len(), before, "{object_type:?}: nothing is created");
    refusal
}

#[test]
fn a_count_that_fails_its_own_checks_keeps_its_place() {
    let mut db = make_db_with_device_and_ai();
    let bad_description = initial(P::DESCRIPTION, None, &unsigned(1));
    let msv = ObjectType::MULTI_STATE_VALUE;
    for (what, values, expected) in [
        (
            "a bad value before a bad count is named first",
            vec![bad_description.clone(), count(0)],
            (ErrorCode::INVALID_DATA_TYPE, 1),
        ),
        (
            "a bad count before a bad value is named first",
            vec![count(0), bad_description.clone()],
            (ErrorCode::VALUE_OUT_OF_RANGE, 1),
        ),
        (
            "a count of the wrong datatype keeps its place too",
            vec![
                bad_description.clone(),
                initial(P::NUMBER_OF_STATES, None, &PropertyValue::Real(3.0)),
            ],
            (ErrorCode::INVALID_DATA_TYPE, 1),
        ),
        (
            "an indexed count keeps its place",
            vec![
                bad_description.clone(),
                initial(P::NUMBER_OF_STATES, Some(1), &unsigned(3)),
            ],
            (ErrorCode::INVALID_DATA_TYPE, 1),
        ),
        // A good count applies first, so a default past it is the value
        // refused, at its own position.
        (
            "a default past a smaller count",
            vec![initial(P::RELINQUISH_DEFAULT, None, &unsigned(2)), count(1)],
            (ErrorCode::VALUE_OUT_OF_RANGE, 1),
        ),
        (
            "a bad second count is named at its place",
            vec![
                count(5),
                initial(P::DESCRIPTION, None, &text("d")),
                count(0),
            ],
            (ErrorCode::VALUE_OUT_OF_RANGE, 3),
        ),
    ] {
        assert_eq!(
            refused(&mut db, msv, values),
            (ErrorClass::PROPERTY, expected.0, expected.1),
            "{what}"
        );
    }

    // A default inside the count is taken wherever the count stands.
    let oid = create(
        &mut db,
        msv,
        vec![initial(P::RELINQUISH_DEFAULT, None, &unsigned(4)), count(5)],
    )
    .unwrap();
    assert_eq!(read(&db, oid, P::RELINQUISH_DEFAULT, None), unsigned(4));
}

#[test]
fn of_several_good_counts_the_last_sets_the_states() {
    // Every count that passes applies ahead of the rest, in request order,
    // so the last one is the count the other values are judged against.
    let mut db = make_db_with_device_and_ai();
    let oid = create(
        &mut db,
        ObjectType::MULTI_STATE_VALUE,
        vec![
            count(3),
            initial(P::STATE_TEXT, Some(5), &text("Five")),
            count(5),
        ],
    )
    .unwrap();
    assert_eq!(read(&db, oid, P::NUMBER_OF_STATES, None), unsigned(5));
    assert_eq!(read(&db, oid, P::STATE_TEXT, Some(5)), text("Five"));
    // Judged against the last count, a label past it is out of range.
    assert_eq!(
        refused(
            &mut db,
            ObjectType::MULTI_STATE_VALUE,
            vec![
                count(5),
                initial(P::STATE_TEXT, Some(5), &text("Five")),
                count(3),
            ],
        ),
        (ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX, 2)
    );
}

#[test]
fn a_null_count_or_state_text_leaves_the_defaults() {
    let mut db = make_db_with_device_and_ai();
    let null = PropertyValue::Null;
    for object_type in [
        ObjectType::MULTI_STATE_INPUT,
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        let oid = create(
            &mut db,
            object_type,
            vec![
                initial(P::STATE_TEXT, None, &null),
                initial(P::NUMBER_OF_STATES, None, &null),
                initial(P::DESCRIPTION, None, &text("after the NULLs")),
            ],
        )
        .unwrap_or_else(|error| panic!("{object_type:?}: {error:?}"));
        assert_eq!(read(&db, oid, P::NUMBER_OF_STATES, None), unsigned(2));
        assert_eq!(
            read(&db, oid, P::STATE_TEXT, None),
            PropertyValue::List(vec![text("State 1"), text("State 2")])
        );
        assert_eq!(
            read(&db, oid, P::DESCRIPTION, None),
            text("after the NULLs")
        );
    }
}

#[test]
fn an_alarm_state_past_the_count_is_refused_in_either_order() {
    let mut db = make_db_with_device_and_ai();
    for object_type in [ObjectType::MULTI_STATE_INPUT, ObjectType::MULTI_STATE_VALUE] {
        for (values, position) in [
            (vec![alarms(&[1, 7]), count(3)], 1),
            (vec![count(3), alarms(&[1, 7])], 2),
            // With no count in the request, the default two states.
            (vec![alarms(&[3])], 1),
        ] {
            assert_eq!(
                refused(&mut db, object_type, values),
                (
                    ErrorClass::PROPERTY,
                    ErrorCode::VALUE_OUT_OF_RANGE,
                    position
                ),
                "{object_type:?}"
            );
        }
        // Inside the requested count, in either order.
        for values in [
            vec![alarms(&[1, 5]), count(5)],
            vec![count(5), alarms(&[1, 5])],
        ] {
            let oid = create(&mut db, object_type, values).unwrap();
            assert_eq!(
                read(&db, oid, P::ALARM_VALUES, None),
                PropertyValue::List(vec![unsigned(1), unsigned(5)])
            );
            assert_eq!(
                read(&db, oid, P::RELIABILITY, None),
                PropertyValue::Enumerated(
                    bacnet_types::enums::Reliability::NO_FAULT_DETECTED.to_raw()
                )
            );
            // And over WriteProperty, a state past the count is refused at
            // its element and changes nothing.
            let mut octets = BytesMut::new();
            for state in [2, 6] {
                encode_property_value(&mut octets, &unsigned(state)).unwrap();
            }
            let mut request = BytesMut::new();
            WritePropertyRequest {
                object_identifier: oid,
                property_identifier: P::ALARM_VALUES,
                property_array_index: None,
                property_value: octets.to_vec(),
                priority: None,
            }
            .encode(&mut request)
            .unwrap();
            assert_eq!(
                list_refusal(handle_write_property(&mut db, &request).map(|_| ())),
                (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2),
                "{object_type:?}"
            );
            assert_eq!(
                read(&db, oid, P::ALARM_VALUES, None),
                PropertyValue::List(vec![unsigned(1), unsigned(5)])
            );
        }
    }
}
