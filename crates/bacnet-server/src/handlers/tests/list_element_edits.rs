//! AddListElement and RemoveListElement on lists of application-tagged values
//! (Clauses 15.1.2 and 15.2.2, #1027). Elements compare whole. AddListElement
//! leaves an element already present as it is, and RemoveListElement refuses
//! an element of another datatype or one not in the list. A refused request
//! changes nothing, and an element refusal names its position, counted from 1
//! (#1026).

use super::*;
use bacnet_objects::elevator::EscalatorObject;
use bacnet_objects::multistate::MultiStateInputObject;

fn request(oid: ObjectIdentifier, property: PropertyIdentifier, elements: &[u8]) -> Vec<u8> {
    request_indexed(oid, property, None, elements)
}

/// A raw request, so malformed and index-zero fixtures reach the handler.
fn request_indexed(
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    elements: &[u8],
) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_object_id(&mut encoded, 0, &oid);
    bacnet_encoding::primitives::encode_ctx_enumerated(&mut encoded, 1, property.to_raw());
    if let Some(index) = index {
        bacnet_encoding::primitives::encode_ctx_unsigned(&mut encoded, 2, u64::from(index));
    }
    encoded.extend_from_slice(&[0x3e]);
    encoded.extend_from_slice(elements);
    encoded.extend_from_slice(&[0x3f]);
    encoded.to_vec()
}

/// Application Unsigned elements, one octet each.
fn unsigned(values: &[u8]) -> Vec<u8> {
    values.iter().flat_map(|value| [0x21, *value]).collect()
}

fn msi_db(alarm_values: Vec<u32>) -> (ObjectDatabase, ObjectIdentifier) {
    msi_db_with_states(3, alarm_values)
}

fn msi_db_with_states(
    number_of_states: u32,
    alarm_values: Vec<u32>,
) -> (ObjectDatabase, ObjectIdentifier) {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", number_of_states).unwrap();
    msi.set_alarm_values(alarm_values).unwrap();
    let oid = msi.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(msi)).unwrap();
    (db, oid)
}

fn alarm_values(db: &ObjectDatabase, oid: ObjectIdentifier) -> PropertyValue {
    db.get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::ALARM_VALUES, None)
        .unwrap()
}

fn list_of(values: impl IntoIterator<Item = u64>) -> PropertyValue {
    PropertyValue::List(values.into_iter().map(PropertyValue::Unsigned).collect())
}

fn add(db: &mut ObjectDatabase, oid: ObjectIdentifier, elements: &[u8]) -> Result<(), Error> {
    handle_add_list_element(
        db,
        &request(oid, PropertyIdentifier::ALARM_VALUES, elements),
    )
}

fn remove(db: &mut ObjectDatabase, oid: ObjectIdentifier, elements: &[u8]) -> Result<(), Error> {
    handle_remove_list_element(
        db,
        &request(oid, PropertyIdentifier::ALARM_VALUES, elements),
    )
}

#[test]
fn add_and_remove_list_element_mutate_msi_alarm_values() {
    let (mut db, oid) = msi_db(vec![]);
    add(&mut db, oid, &unsigned(&[2])).unwrap();
    assert_eq!(alarm_values(&db, oid), list_of([2]));
    remove(&mut db, oid, &unsigned(&[2])).unwrap();
    assert_eq!(alarm_values(&db, oid), list_of([]));
}

#[test]
fn add_list_element_leaves_elements_already_present() {
    let (mut db, oid) = msi_db(vec![1, 2]);
    // 2 and 1 are present, and the second 3 is present once the first is
    // added: each is the same whole element, so the list gains only one 3.
    add(&mut db, oid, &unsigned(&[2, 3, 3, 1])).unwrap();
    assert_eq!(alarm_values(&db, oid), list_of([1, 2, 3]));
    // A request of present elements only succeeds and changes nothing.
    add(&mut db, oid, &unsigned(&[3, 1])).unwrap();
    assert_eq!(alarm_values(&db, oid), list_of([1, 2, 3]));
    // A non-minimal encoding of 2 (two content octets) is still 2.
    add(&mut db, oid, &[0x22, 0, 2]).unwrap();
    assert_eq!(alarm_values(&db, oid), list_of([1, 2, 3]));
}

#[test]
fn remove_list_element_refuses_an_absent_element_and_removes_nothing() {
    let (mut db, oid) = msi_db(vec![1, 2, 3]);
    for (elements, position) in [(&[5][..], 1), (&[1, 5, 2], 2), (&[1, 2, 3, 4], 4)] {
        assert_eq!(
            list_refusal(remove(&mut db, oid, &unsigned(elements))),
            (
                ErrorClass::SERVICES,
                ErrorCode::LIST_ELEMENT_NOT_FOUND,
                position
            ),
            "{elements:?}"
        );
        assert_eq!(alarm_values(&db, oid), list_of([1, 2, 3]), "{elements:?}");
    }
    // Every element present: all of them go, a repeated one once.
    remove(&mut db, oid, &unsigned(&[3, 1, 3])).unwrap();
    assert_eq!(alarm_values(&db, oid), list_of([2]));
}

#[test]
fn remove_list_element_checks_the_element_datatype_before_presence() {
    let (mut db, oid) = msi_db(vec![1, 2]);
    // Alarm_Values holds Unsigned, so Boolean true (0x11) and Enumerated 2
    // (0x91 0x02) are the wrong datatype even where the value would match.
    // Elements are checked in request order, so an absent Unsigned 9 ahead of
    // the Boolean is the failure reported.
    let wrong_type = (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE);
    let absent = (ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND);
    for (elements, (class, code), position) in [
        (vec![0x11], wrong_type, 1),
        (vec![0x21, 1, 0x91, 2], wrong_type, 2),
        (vec![0x21, 1, 0x21, 9, 0x11], absent, 2),
    ] {
        assert_eq!(
            list_refusal(remove(&mut db, oid, &elements)),
            (class, code, position),
            "{elements:02x?}"
        );
        assert_eq!(alarm_values(&db, oid), list_of([1, 2]));
    }
    // An empty list shows no datatype; any element is simply not there.
    let (mut db, oid) = msi_db(vec![]);
    assert_eq!(
        list_refusal(remove(&mut db, oid, &[0x11])),
        (ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND, 1)
    );
}

#[test]
fn add_list_element_malformed_tail_errors_without_partial_commit() {
    let (mut db, oid) = msi_db(vec![1]);
    // Unsigned 2, Unsigned 3, then an application tag whose content is cut.
    assert_eq!(
        list_refusal(add(&mut db, oid, &[0x21, 2, 0x21, 3, 0xD1, 0])),
        (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 3)
    );
    assert_eq!(alarm_values(&db, oid), list_of([1]));
}

#[test]
fn remove_list_element_malformed_tail_errors_without_partial_commit() {
    let (mut db, oid) = msi_db(vec![2, 3]);
    assert_eq!(
        list_refusal(remove(&mut db, oid, &[0x21, 2, 0xD1, 0])),
        (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2)
    );
    assert_eq!(alarm_values(&db, oid), list_of([2, 3]));
}

#[test]
fn add_list_element_over_cap_returns_the_clause_15_1_error() {
    // Fill to MAX_ALARM_VALUES (1024) so any new element trips the cap. The
    // object has a state for every value, so each is in range (#1429).
    let (mut db, oid) = msi_db_with_states(2048, (1..=1024).collect());
    // Unsigned 2000 (0x22 0x07 0xD0) is not in the list. Clause 15.1 names
    // AddListElement's own error, not WriteProperty's
    // NO_SPACE_TO_WRITE_PROPERTY, and the element that did not fit.
    const NEW: [u8; 3] = [0x22, 0x07, 0xD0];
    assert_eq!(
        list_refusal(add(&mut db, oid, &NEW)),
        (
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
            1
        )
    );
    // Present elements take no space, so the new one is still the element
    // that does not fit.
    assert_eq!(
        list_refusal(add(&mut db, oid, &[&unsigned(&[7, 9])[..], &NEW].concat())),
        (
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
            3
        )
    );
    // Present elements alone still fit.
    add(&mut db, oid, &unsigned(&[7, 9])).unwrap();
    assert_eq!(alarm_values(&db, oid), list_of(1..=1024));
}

#[test]
fn add_list_element_names_the_new_element_the_object_refuses() {
    let mut db = ObjectDatabase::new();
    let escalator = EscalatorObject::new(1, "ESC-1").unwrap();
    let oid = escalator.object_identifier();
    db.add(Box::new(escalator)).unwrap();
    let fault_signals = |db: &ObjectDatabase| {
        db.get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::FAULT_SIGNALS, None)
            .unwrap()
    };
    // Enumerated 1, DRIVE_AND_MOTOR_FAULT.
    handle_add_list_element(
        &mut db,
        &request(oid, PropertyIdentifier::FAULT_SIGNALS, &[0x91, 1]),
    )
    .unwrap();
    // The object refuses fault 20 as out of range. Fault 1 is present, so 20
    // is the only element the list would gain.
    assert_eq!(
        list_refusal(handle_add_list_element(
            &mut db,
            &request(oid, PropertyIdentifier::FAULT_SIGNALS, &[0x91, 1, 0x91, 20]),
        )),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2)
    );
    // A Real is the wrong datatype for the object.
    assert_eq!(
        list_refusal(handle_add_list_element(
            &mut db,
            &request(
                oid,
                PropertyIdentifier::FAULT_SIGNALS,
                &[0x91, 1, 0x44, 0x3F, 0x80, 0, 0],
            ),
        )),
        (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2)
    );
    // Repeating a present fault is not a duplicate entry: it is ignored.
    handle_add_list_element(
        &mut db,
        &request(oid, PropertyIdentifier::FAULT_SIGNALS, &[0x91, 1, 0x91, 1]),
    )
    .unwrap();
    assert_eq!(
        fault_signals(&db),
        PropertyValue::List(vec![PropertyValue::Enumerated(1)])
    );
}

#[test]
fn add_list_element_names_an_alarm_state_past_the_count() {
    // The Multi-state Input has three states; state 1 is present, so 4 is the
    // element the object refuses, and nothing is added (#1429).
    let (mut db, oid) = msi_db(vec![1]);
    assert_eq!(
        list_refusal(add(&mut db, oid, &unsigned(&[1, 2, 4]))),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 3)
    );
    assert_eq!(alarm_values(&db, oid), list_of([1]));
}

#[test]
fn add_list_element_rejects_array_index_on_alarm_values() {
    let (mut db, oid) = msi_db(vec![]);
    let request = request_indexed(oid, PropertyIdentifier::ALARM_VALUES, Some(1), &[0x21, 2]);
    // A refusal of the target, not of an element: element number 0.
    assert_eq!(
        list_refusal(handle_add_list_element(&mut db, &request)),
        (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY, 0)
    );
    assert_eq!(alarm_values(&db, oid), list_of([]));
}

#[test]
fn whole_list_write_property_decodes_all_elements() {
    let (mut db, oid) = msi_db(vec![]);
    let write = |db: &mut ObjectDatabase, property_value: Vec<u8>| {
        let mut encoded = BytesMut::new();
        WritePropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: None,
            property_value,
            priority: None,
        }
        .encode(&mut encoded)
        .unwrap();
        handle_write_property(db, &encoded)
    };
    // #182: WriteProperty loop-decodes the whole payload, so the whole-list
    // write lands with per-element validation in the arm. A BACnetLIST is
    // consecutive application-tagged elements.
    write(&mut db, unsigned(&[2, 3])).unwrap();
    assert_eq!(alarm_values(&db, oid), list_of([2, 3]));

    // Per-element validation still applies: one non-Unsigned member refuses
    // the whole write, names its element (#1048), and leaves the list
    // untouched.
    assert_eq!(
        list_refusal(write(&mut db, vec![0x21, 3, 0x11]).map(|_| ())),
        (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2)
    );
    assert_eq!(
        alarm_values(&db, oid),
        list_of([2, 3]),
        "refused write leaves the list unchanged"
    );
    // A state the object doesn't have (it has three) is out of range at its
    // element (#1429).
    assert_eq!(
        list_refusal(write(&mut db, unsigned(&[1, 7])).map(|_| ())),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2)
    );
    assert_eq!(alarm_values(&db, oid), list_of([2, 3]));
}
