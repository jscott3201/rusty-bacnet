//! CreateObject initializes a few properties WriteProperty can't change
//! afterwards (#1429): Units on Analog Input and Output, and Number_Of_States
//! on the multi-state objects. Each goes through the object's
//! `initialize_property`, keeps its checks, and stays WRITE_ACCESS_DENIED to
//! a later WriteProperty. Number_Of_States applies before the other initial
//! values, so the result doesn't depend on where it stands in the list.
//! State_Text written whole goes the write route and sets the count when
//! the request gives none (#1443); with a count, it has to match it.

use super::create_object_initial_values::{create, initial, read, text};
use super::*;
use bacnet_types::enums::EngineeringUnits;
use PropertyIdentifier as P;

const MULTI_STATE: [ObjectType; 3] = [
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
];

fn units(units: EngineeringUnits) -> PropertyValue {
    PropertyValue::Enumerated(units.to_raw())
}

fn labels(labels: &[&str]) -> PropertyValue {
    PropertyValue::List(labels.iter().map(|label| text(label)).collect())
}

fn wp(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    value: &PropertyValue,
) -> Result<(), Error> {
    let mut octets = BytesMut::new();
    encode_property_value(&mut octets, value).unwrap();
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: index,
        property_value: octets.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn denied(result: Result<(), Error>) -> bool {
    matches!(
        result,
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED
    )
}

/// The refusal of a create that must leave nothing behind.
fn refused(
    db: &mut ObjectDatabase,
    object_type: ObjectType,
    values: Vec<bacnet_services::common::BACnetPropertyValue>,
) -> (ErrorClass, ErrorCode, u32) {
    let before = db.len();
    let refusal = list_refusal(create(db, object_type, values).map(|_| ()));
    assert_eq!(db.len(), before, "{object_type:?}: nothing is created");
    refusal
}

#[test]
fn an_analog_object_takes_units_at_creation_and_keeps_it_read_only() {
    let mut db = make_db_with_device_and_ai();
    for object_type in [ObjectType::ANALOG_INPUT, ObjectType::ANALOG_OUTPUT] {
        let celsius = units(EngineeringUnits::DEGREES_CELSIUS);
        let oid = create(
            &mut db,
            object_type,
            vec![
                initial(
                    P::OBJECT_NAME,
                    None,
                    &text(&format!("{object_type:?} zone")),
                ),
                initial(P::UNITS, None, &celsius),
            ],
        )
        .unwrap_or_else(|error| panic!("{object_type:?}: {error:?}"));
        assert_eq!(read(&db, oid, P::UNITS, None), celsius, "{object_type:?}");
        // A WriteProperty of Units is still refused, and changes nothing.
        let percent = units(EngineeringUnits::PERCENT);
        assert!(denied(wp(&mut db, oid, P::UNITS, None, &percent)));
        assert_eq!(read(&db, oid, P::UNITS, None), celsius, "{object_type:?}");
    }
}

#[test]
fn a_units_initial_value_is_checked_and_a_null_leaves_the_default() {
    let mut db = make_db_with_device_and_ai();
    for (value, expected) in [
        (
            PropertyValue::Unsigned(62),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2),
        ),
        (
            PropertyValue::Enumerated(65_536),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2),
        ),
    ] {
        let values = vec![
            initial(P::DESCRIPTION, None, &text("d")),
            initial(P::UNITS, None, &value),
        ];
        assert_eq!(
            refused(&mut db, ObjectType::ANALOG_INPUT, values),
            expected,
            "{value:?}"
        );
    }
    // An index on Units is refused before the object sees the value.
    let indexed = vec![initial(
        P::UNITS,
        Some(1),
        &units(EngineeringUnits::PERCENT),
    )];
    assert_eq!(
        refused(&mut db, ObjectType::ANALOG_OUTPUT, indexed),
        (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY, 1)
    );
    // A NULL is judged as on the write route: Units keeps the default.
    let oid = create(
        &mut db,
        ObjectType::ANALOG_OUTPUT,
        vec![initial(P::UNITS, None, &PropertyValue::Null)],
    )
    .unwrap();
    assert_eq!(
        read(&db, oid, P::UNITS, None),
        units(EngineeringUnits::NO_UNITS)
    );
}

#[test]
fn number_of_states_and_state_text_give_the_same_object_in_either_order() {
    let mut db = make_db_with_device_and_ai();
    let count = PropertyValue::Unsigned(3);
    let names = labels(&["Off", "Low", "High"]);
    for object_type in MULTI_STATE {
        let mut objects = Vec::new();
        for states_first in [true, false] {
            let mut values = vec![
                initial(P::NUMBER_OF_STATES, None, &count),
                initial(P::STATE_TEXT, None, &names),
            ];
            if !states_first {
                values.reverse();
            }
            let oid = create(&mut db, object_type, values)
                .unwrap_or_else(|error| panic!("{object_type:?} {states_first}: {error:?}"));
            objects.push(oid);
        }
        for oid in objects {
            assert_eq!(read(&db, oid, P::NUMBER_OF_STATES, None), count);
            assert_eq!(read(&db, oid, P::STATE_TEXT, None), names);
            assert_eq!(
                read(&db, oid, P::STATE_TEXT, Some(0)),
                PropertyValue::Unsigned(3)
            );
            // The count stays read-only to WriteProperty; State_Text, whole
            // or by element, doesn't (#1443).
            assert!(denied(wp(&mut db, oid, P::NUMBER_OF_STATES, None, &count)));
            wp(&mut db, oid, P::STATE_TEXT, None, &names).unwrap();
            wp(&mut db, oid, P::STATE_TEXT, Some(3), &text("Max")).unwrap();
            assert_eq!(read(&db, oid, P::STATE_TEXT, Some(3)), text("Max"));
        }
    }
}

#[test]
fn a_single_state_takes_one_label() {
    // One label alone decodes as that CharacterString, not as a list.
    let mut db = make_db_with_device_and_ai();
    for object_type in MULTI_STATE {
        let oid = create(
            &mut db,
            object_type,
            vec![
                initial(P::STATE_TEXT, None, &text("Only")),
                initial(P::NUMBER_OF_STATES, None, &PropertyValue::Unsigned(1)),
            ],
        )
        .unwrap_or_else(|error| panic!("{object_type:?}: {error:?}"));
        assert_eq!(read(&db, oid, P::STATE_TEXT, None), labels(&["Only"]));
    }
}

#[test]
fn values_that_name_a_state_are_judged_against_the_requested_count() {
    // Present_Value 4 and State_Text[4] would be out of range for the two
    // states a new object starts with. Number_Of_States comes last in the
    // list but is applied first, so both are accepted.
    let mut db = make_db_with_device_and_ai();
    let four = PropertyValue::Unsigned(4);
    let mut target = None;
    let mut request_values = vec![
        initial(P::RELINQUISH_DEFAULT, None, &four),
        initial(P::STATE_TEXT, Some(4), &text("Purge")),
        initial(P::NUMBER_OF_STATES, None, &PropertyValue::Unsigned(5)),
    ];
    for object_type in [
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        handle_create_object_observed(
            &mut db,
            &super::create_object_initial_values::request(object_type, request_values.clone()),
            &mut BytesMut::new(),
            &mut target,
            Some(&crate::command_source::test_origin()),
        )
        .unwrap_or_else(|refusal| panic!("{object_type:?}: {refusal:?}"));
        let oid = target.unwrap();
        assert_eq!(read(&db, oid, P::RELINQUISH_DEFAULT, None), four);
        assert_eq!(read(&db, oid, P::PRESENT_VALUE, None), four);
        assert_eq!(read(&db, oid, P::STATE_TEXT, Some(4)), text("Purge"));
        assert_eq!(read(&db, oid, P::STATE_TEXT, Some(5)), text("State 5"));
    }
    // The input has no Relinquish_Default; out of service, its Present_Value
    // takes a state past the default count the same way.
    request_values[0] = initial(P::OUT_OF_SERVICE, None, &PropertyValue::Boolean(true));
    request_values.insert(1, initial(P::PRESENT_VALUE, None, &four));
    let oid = create(&mut db, ObjectType::MULTI_STATE_INPUT, request_values).unwrap();
    assert_eq!(read(&db, oid, P::PRESENT_VALUE, None), four);
}

#[test]
fn a_state_text_of_the_wrong_length_is_refused_at_its_position() {
    let mut db = make_db_with_device_and_ai();
    for object_type in MULTI_STATE {
        // Three labels for four states, wherever the count stands.
        for (values, position) in [
            (
                vec![
                    initial(P::DESCRIPTION, None, &text("d")),
                    initial(P::NUMBER_OF_STATES, None, &PropertyValue::Unsigned(4)),
                    initial(P::STATE_TEXT, None, &labels(&["a", "b", "c"])),
                ],
                3,
            ),
            (
                vec![
                    initial(P::STATE_TEXT, None, &labels(&["a", "b", "c"])),
                    initial(P::NUMBER_OF_STATES, None, &PropertyValue::Unsigned(4)),
                ],
                1,
            ),
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
        // A label that isn't a CharacterString is the wrong datatype.
        let mixed = PropertyValue::List(vec![text("a"), PropertyValue::Unsigned(2)]);
        assert_eq!(
            refused(
                &mut db,
                object_type,
                vec![initial(P::STATE_TEXT, None, &mixed)]
            ),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 1),
            "{object_type:?}"
        );
    }
}

#[test]
fn a_number_of_states_out_of_range_is_refused_at_its_position() {
    let mut db = make_db_with_device_and_ai();
    let past_the_cap = u64::from(bacnet_objects::multistate::MAX_NUMBER_OF_STATES) + 1;
    for object_type in MULTI_STATE {
        for count in [0, past_the_cap, u64::MAX] {
            let values = vec![
                initial(P::DESCRIPTION, None, &text("d")),
                initial(P::NUMBER_OF_STATES, None, &PropertyValue::Unsigned(count)),
            ];
            assert_eq!(
                refused(&mut db, object_type, values),
                (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2),
                "{object_type:?} {count}"
            );
        }
        let real = vec![initial(
            P::NUMBER_OF_STATES,
            None,
            &PropertyValue::Real(3.0),
        )];
        assert_eq!(
            refused(&mut db, object_type, real),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 1),
            "{object_type:?}"
        );
    }
    // Number_Of_States isn't creation-only on an analog object, so it is
    // judged in list order there and refused as WriteProperty refuses it.
    let values = vec![
        initial(P::DESCRIPTION, None, &PropertyValue::Unsigned(1)),
        initial(P::NUMBER_OF_STATES, None, &PropertyValue::Unsigned(3)),
    ];
    assert_eq!(
        refused(&mut db, ObjectType::ANALOG_INPUT, values),
        (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 1)
    );
}

#[test]
fn a_whole_state_text_without_a_count_gives_the_count() {
    let mut db = make_db_with_device_and_ai();
    let three = labels(&["Off", "Low", "High"]);
    for object_type in MULTI_STATE {
        // The labels set three states before the rest apply, wherever they
        // stand, so State_Text[3] is in range ahead of them and then
        // relabels the third state (#1443).
        let oid = create(
            &mut db,
            object_type,
            vec![
                initial(P::STATE_TEXT, Some(3), &text("Max")),
                initial(P::STATE_TEXT, None, &three),
            ],
        )
        .unwrap_or_else(|error| panic!("{object_type:?}: {error:?}"));
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES, None),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            read(&db, oid, P::STATE_TEXT, None),
            labels(&["Off", "Low", "Max"])
        );
        // With several, the last sets the count.
        let oid = create(
            &mut db,
            object_type,
            vec![
                initial(P::STATE_TEXT, None, &three),
                initial(P::STATE_TEXT, None, &text("Only")),
            ],
        )
        .unwrap();
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES, None),
            PropertyValue::Unsigned(1)
        );
    }
    // A value past the count the labels give is the one refused.
    for object_type in [
        ObjectType::MULTI_STATE_OUTPUT,
        ObjectType::MULTI_STATE_VALUE,
    ] {
        let values = vec![
            initial(P::RELINQUISH_DEFAULT, None, &PropertyValue::Unsigned(2)),
            initial(P::STATE_TEXT, None, &text("Only")),
        ];
        assert_eq!(
            refused(&mut db, object_type, values),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 1),
            "{object_type:?}"
        );
    }
}
