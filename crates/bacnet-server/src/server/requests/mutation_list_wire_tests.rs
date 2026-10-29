//! AddListElement and RemoveListElement errors on the wire (#1026, #1027).
//! Both services answer every error with a ChangeList-Error (Clause 21): an
//! opening tag 0 (0x0E), the class and code as one-octet application
//! Enumerated values (0x91), a closing tag 0 (0x0F), and the First Failed
//! Element Number as context tag 1 (0x19 for one octet). It is zero when the
//! request or its target is refused, and the element's position, counted from
//! 1, when an element is.

use super::mutation_tests::{Fixture, oid};
use super::*;
use crate::mutation::MutationPolicy;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_services::write_property::WritePropertyRequest;

pub(super) const ADD: ConfirmedServiceChoice = ConfirmedServiceChoice::ADD_LIST_ELEMENT;
pub(super) const REMOVE: ConfirmedServiceChoice = ConfirmedServiceChoice::REMOVE_LIST_ELEMENT;

/// A raw request, so malformed and indexed fixtures reach the server as built.
pub(super) fn list_request(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    elements: &[u8],
) -> Bytes {
    let mut bytes = BytesMut::new();
    bacnet_encoding::primitives::encode_ctx_object_id(&mut bytes, 0, &object);
    bacnet_encoding::primitives::encode_ctx_enumerated(&mut bytes, 1, property.to_raw());
    if let Some(index) = index {
        bacnet_encoding::primitives::encode_ctx_unsigned(&mut bytes, 2, u64::from(index));
    }
    bytes.extend_from_slice(&[0x3e]);
    bytes.extend_from_slice(elements);
    bytes.extend_from_slice(&[0x3f]);
    bytes.freeze()
}

fn alarm_values(elements: &[u8]) -> Bytes {
    list_request(
        oid(ObjectType::MULTI_STATE_INPUT, 1),
        PropertyIdentifier::ALARM_VALUES,
        None,
        elements,
    )
}

/// Replace the fixture's Multi-state Input with one that has a state for
/// every value up to 2048 and holds `values` as its Alarm_Values, so a full
/// list stays in range (#1429).
pub(super) async fn fill_msi_alarm_values(fixture: &Fixture, values: std::ops::Range<u32>) {
    let mut msi = bacnet_objects::multistate::MultiStateInputObject::new(1, "msi", 2048).unwrap();
    msi.set_alarm_values(values.collect());
    let mut db = fixture.db.write().await;
    db.remove(&oid(ObjectType::MULTI_STATE_INPUT, 1)).unwrap();
    db.add(Box::new(msi)).unwrap();
}

/// The response APDU exactly as the server encoded it.
pub(super) async fn wire(
    fixture: &Fixture,
    service: ConfirmedServiceChoice,
    request: Bytes,
) -> Vec<u8> {
    let npdu = fixture
        .dispatch(service, request, 5)
        .await
        .expect("response");
    decode_npdu(npdu).unwrap().payload.to_vec()
}

/// The Error PDU header for invoke ID 5, then the ChangeList-Error body.
pub(super) fn change_list_error(
    service: ConfirmedServiceChoice,
    class: u8,
    code: u8,
    element: u8,
) -> Vec<u8> {
    vec![
        0x50,
        5,
        service.to_raw(),
        0x0E,
        0x91,
        class,
        0x91,
        code,
        0x0F,
        0x19,
        element,
    ]
}

async fn msi_alarm_values(fixture: &Fixture) -> PropertyValue {
    fixture
        .read(
            oid(ObjectType::MULTI_STATE_INPUT, 1),
            PropertyIdentifier::ALARM_VALUES,
        )
        .await
}

#[tokio::test]
async fn list_element_refusals_go_out_as_change_list_errors() {
    // OBJECT 1, PROPERTY 2, RESOURCES 3, SERVICES 5. INVALID_DATA_TYPE 9,
    // NO_SPACE_TO_ADD_LIST_ELEMENT 19, PROPERTY_IS_NOT_A_LIST 22,
    // UNKNOWN_OBJECT 31, PROPERTY_IS_NOT_AN_ARRAY 50, LIST_ELEMENT_NOT_FOUND 81.
    let fixture = Fixture::new(None);
    fixture
        .db
        .write()
        .await
        .add(Box::new(BinaryValueObject::new(9, "nine").unwrap()))
        .unwrap();
    let missing = oid(ObjectType::MULTI_STATE_INPUT, 99);
    for (what, service, request, expected) in [
        (
            "an unknown object",
            ADD,
            list_request(missing, PropertyIdentifier::ALARM_VALUES, None, &[0x21, 2]),
            change_list_error(ADD, 1, 31, 0),
        ),
        (
            "a target that is not a list",
            REMOVE,
            list_request(
                oid(ObjectType::BINARY_VALUE, 9),
                PropertyIdentifier::PRESENT_VALUE,
                None,
                &[0x91, 0],
            ),
            change_list_error(REMOVE, 5, 22, 0),
        ),
        (
            "an index on a list",
            ADD,
            list_request(
                oid(ObjectType::MULTI_STATE_INPUT, 1),
                PropertyIdentifier::ALARM_VALUES,
                Some(1),
                &[0x21, 2],
            ),
            change_list_error(ADD, 2, 50, 0),
        ),
        (
            "a second element whose content is cut",
            ADD,
            alarm_values(&[0x21, 2, 0xD1, 0]),
            change_list_error(ADD, 2, 9, 2),
        ),
        (
            "an element of another datatype",
            REMOVE,
            alarm_values(&[0x21, 1, 0x11]),
            change_list_error(REMOVE, 2, 9, 2),
        ),
        (
            "an absent third element",
            REMOVE,
            alarm_values(&[0x21, 1, 0x21, 1, 0x21, 3]),
            change_list_error(REMOVE, 5, 81, 3),
        ),
    ] {
        assert_eq!(wire(&fixture, service, request).await, expected, "{what}");
        assert_eq!(
            msi_alarm_values(&fixture).await,
            PropertyValue::List(vec![PropertyValue::Unsigned(1)]),
            "{what} changed the list"
        );
    }

    // The list is full: present elements take no space, so the first new one
    // (Unsigned 7, the third element) is the one that does not fit.
    fill_msi_alarm_values(&fixture, 100..1124).await;
    assert_eq!(
        wire(
            &fixture,
            ADD,
            alarm_values(&[0x21, 100, 0x21, 101, 0x21, 7])
        )
        .await,
        change_list_error(ADD, 3, 19, 3)
    );
}

#[tokio::test]
async fn list_element_duplicates_succeed_and_absent_elements_change_nothing() {
    let fixture = Fixture::new(None);
    // [1] gains 2 once; the repeated 2 and the present 1 are left as they are.
    assert_eq!(
        wire(&fixture, ADD, alarm_values(&[0x21, 2, 0x21, 1, 0x21, 2])).await,
        vec![0x20, 5, ADD.to_raw()]
    );
    let both = PropertyValue::List(vec![PropertyValue::Unsigned(1), PropertyValue::Unsigned(2)]);
    assert_eq!(msi_alarm_values(&fixture).await, both);
    // 2 is present but 3 is not: nothing is removed.
    assert_eq!(
        wire(&fixture, REMOVE, alarm_values(&[0x21, 2, 0x21, 3])).await,
        change_list_error(REMOVE, 5, 81, 2)
    );
    assert_eq!(msi_alarm_values(&fixture).await, both);
    assert_eq!(
        wire(&fixture, REMOVE, alarm_values(&[0x21, 2])).await,
        vec![0x20, 5, REMOVE.to_raw()]
    );
    assert_eq!(
        msi_alarm_values(&fixture).await,
        PropertyValue::List(vec![PropertyValue::Unsigned(1)])
    );
}

#[tokio::test]
async fn list_element_policy_denial_is_a_change_list_error_naming_no_element() {
    let mut fixture = Fixture::new(None);
    fixture.config.mutation_policy = MutationPolicy::DenyAll;
    // SERVICES 5 / SERVICE_REQUEST_DENIED 29.
    for service in [ADD, REMOVE] {
        assert_eq!(
            wire(&fixture, service, alarm_values(&[0x21, 1])).await,
            change_list_error(service, 5, 29, 0)
        );
    }
}

#[tokio::test]
async fn other_services_keep_the_plain_error_form() {
    let fixture = Fixture::new(None);
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid(ObjectType::MULTI_STATE_INPUT, 99),
        property_identifier: PropertyIdentifier::ALARM_VALUES,
        property_array_index: None,
        property_value: vec![0x21, 1],
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    // OBJECT / UNKNOWN_OBJECT and nothing after the code.
    assert_eq!(
        wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            request.freeze()
        )
        .await,
        vec![0x50, 5, 15, 0x91, 1, 0x91, 31]
    );
}
