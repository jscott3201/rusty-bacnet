//! State_Text written whole, or its size at index 0, over WriteProperty and
//! WritePropertyMultiple sets a multi-state object's Number_Of_States
//! (#1443), and a shrink that would strand a state the object holds is
//! VALUE_OUT_OF_RANGE.

use super::*;
use PropertyIdentifier as P;
use bacnet_objects::multistate::{
    MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::error::RemoteError;

fn octets(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}

fn labels(count: usize) -> PropertyValue {
    PropertyValue::List(
        (1..=count)
            .map(|state| PropertyValue::CharacterString(format!("Mode {state}")))
            .collect(),
    )
}

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: P) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

fn wp(db: &mut ObjectDatabase, oid: ObjectIdentifier, value: &PropertyValue) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: P::STATE_TEXT,
        property_array_index: None,
        property_value: octets(value),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn wpm(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    values: &[PropertyValue],
) -> Result<(), Error> {
    let whole: Vec<_> = values.iter().map(|value| (None, value.clone())).collect();
    wpm_at(db, oid, &whole)
}

/// State_Text writes at the indexes given, in one WritePropertyMultiple.
fn wpm_at(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    values: &[(Option<u32>, PropertyValue)],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: values
                .iter()
                .map(|(index, value)| BACnetPropertyValue {
                    property_identifier: P::STATE_TEXT,
                    property_array_index: *index,
                    value: octets(value),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_multiple(db, &request).map(|_| ())
}

/// A WriteProperty of State_Text's size, index 0.
fn size(db: &mut ObjectDatabase, oid: ObjectIdentifier, count: u64) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: P::STATE_TEXT,
        property_array_index: Some(0),
        property_value: octets(&PropertyValue::Unsigned(count)),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn label(db: &ObjectDatabase, oid: ObjectIdentifier, index: u32) -> PropertyValue {
    db.get(&oid)
        .unwrap()
        .read_property(P::STATE_TEXT, Some(index))
        .unwrap()
}

fn out_of_range(result: Result<(), Error>) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
        if class == ErrorClass::PROPERTY
            && code == ErrorCode::VALUE_OUT_OF_RANGE)
}

fn db() -> (ObjectDatabase, [ObjectIdentifier; 3]) {
    let mut db = ObjectDatabase::new();
    let mut input = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    input.set_present_value(3);
    let mut output = MultiStateOutputObject::new(1, "MSO-1", 4).unwrap();
    output.set_relinquish_default(3).unwrap();
    let mut value = MultiStateValueObject::new(1, "MSV-1", 4).unwrap();
    value.set_alarm_values(vec![3]);
    let oids = [
        input.object_identifier(),
        output.object_identifier(),
        value.object_identifier(),
    ];
    db.add(Box::new(input)).unwrap();
    db.add(Box::new(output)).unwrap();
    db.add(Box::new(value)).unwrap();
    (db, oids)
}

#[test]
fn a_whole_state_text_write_resizes_the_count() {
    let (mut db, oids) = db();
    for oid in oids {
        wp(&mut db, oid, &labels(6)).unwrap();
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES),
            PropertyValue::Unsigned(6)
        );
        assert_eq!(read(&db, oid, P::STATE_TEXT), labels(6));
        // Each holds state 3, so three states are the fewest it takes.
        assert!(out_of_range(wp(&mut db, oid, &labels(2))), "{oid:?}");
        assert_eq!(read(&db, oid, P::STATE_TEXT), labels(6), "{oid:?}");
        wp(&mut db, oid, &labels(3)).unwrap();
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES),
            PropertyValue::Unsigned(3)
        );
    }
}

#[test]
fn write_property_multiple_resizes_in_order_and_stops_at_the_refusal() {
    let (mut db, oids) = db();
    for oid in oids {
        // The last of several whole writes sets the count.
        wpm(&mut db, oid, &[labels(5), labels(4)]).unwrap();
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES),
            PropertyValue::Unsigned(4)
        );
        // The first write applies, and the refused second changes nothing.
        assert!(out_of_range(wpm(&mut db, oid, &[labels(7), labels(1)])));
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES),
            PropertyValue::Unsigned(7)
        );
        assert_eq!(read(&db, oid, P::STATE_TEXT), labels(7));
    }
}

#[test]
fn a_count_written_at_index_0_resizes_over_wp_and_wpm() {
    let (mut db, oids) = db();
    let max = u64::from(bacnet_objects::multistate::MAX_NUMBER_OF_STATES);
    for oid in oids {
        // A grow appends a new object's default labels.
        size(&mut db, oid, 6).unwrap();
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES),
            PropertyValue::Unsigned(6)
        );
        assert_eq!(label(&db, oid, 0), PropertyValue::Unsigned(6));
        assert_eq!(
            label(&db, oid, 6),
            PropertyValue::CharacterString("State 6".into())
        );
        // A shrink truncates.
        size(&mut db, oid, 3).unwrap();
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(label(&db, oid, 0), PropertyValue::Unsigned(3));
        // Each holds state 3; no states, and more than the most, are refused.
        for count in [2, 0, max + 1] {
            assert!(out_of_range(size(&mut db, oid, count)), "{oid:?} {count}");
            assert_eq!(
                read(&db, oid, P::NUMBER_OF_STATES),
                PropertyValue::Unsigned(3)
            );
        }
        // In WritePropertyMultiple, a whole write and then a size, in order.
        wpm_at(
            &mut db,
            oid,
            &[(None, labels(4)), (Some(0), PropertyValue::Unsigned(5))],
        )
        .unwrap();
        assert_eq!(
            read(&db, oid, P::NUMBER_OF_STATES),
            PropertyValue::Unsigned(5)
        );
        assert_eq!(
            label(&db, oid, 4),
            PropertyValue::CharacterString("Mode 4".into())
        );
        assert_eq!(
            label(&db, oid, 5),
            PropertyValue::CharacterString("State 5".into())
        );
        assert!(out_of_range(wpm_at(
            &mut db,
            oid,
            &[(Some(0), PropertyValue::Unsigned(2))]
        )));
    }
}
