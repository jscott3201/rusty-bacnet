//! AddListElement and RemoveListElement of a Schedule's
//! List_Of_Object_Property_References on a running server (#1121), and
//! references whose Device member names this device (#1122).
//!
//! Both list services edit the list and hand the result to the Schedule's
//! whole-list write, so they share its refusals and its effect on the
//! targets: an added target gets the current value at once and a removed one
//! has its slot relinquished. A member naming Device 856, the harness's own
//! Device, is stored as the local reference it stands for; one naming any
//! other device is refused (Clause 12.24.10).
//!
//! The harness is the one in `schedule_write_tests`: Tuesday 29 September
//! 2026, 15:00; SCH-5 commands AV-1's Present_Value at priority 16 and
//! defaults to 10.0, written at start-up; the COV subscription watches AV-1.
//! AV-2 and AV-3 are spare Analog Value targets with nothing commanded.
//!
//! PROPERTY 2, RESOURCES 3, SERVICES 5; INVALID_DATA_TYPE 9,
//! NO_SPACE_TO_ADD_LIST_ELEMENT 19, OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED 45,
//! LIST_ELEMENT_NOT_FOUND 81.
use super::cov_wire_test_support::*;
use super::schedule_reference_write_tests::{LIST, av2, bv2, present_values, slot};
use super::schedule_write_tests::{
    from_three, read, sch5, schedule, write_property, write_property_multiple,
};
use super::*;
use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_services::list_manipulation::{ChangeListError, ListElementRequest};
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType};

const ADD: ConfirmedServiceChoice = ConfirmedServiceChoice::ADD_LIST_ELEMENT;
const REMOVE: ConfirmedServiceChoice = ConfirmedServiceChoice::REMOVE_LIST_ELEMENT;

fn av3() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 3).unwrap()
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

/// The harness's own Device.
fn local() -> Option<ObjectIdentifier> {
    Some(device(856))
}

/// A Device this server is not.
fn remote() -> Option<ObjectIdentifier> {
    Some(device(9))
}

/// The encoded members referencing each object's Present_Value, each with
/// the Device identifier given.
fn members(members: &[(ObjectIdentifier, Option<ObjectIdentifier>)]) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    for (object, device) in members {
        encode_device_object_property_reference(
            &mut bytes,
            &BACnetDeviceObjectPropertyReference {
                object_identifier: *object,
                property_identifier: PV.to_raw(),
                property_array_index: None,
                device_identifier: *device,
            },
        );
    }
    bytes.to_vec()
}

/// An application-tagged Real 1.0: not a reference.
const REAL: [u8; 5] = [0x44, 0x3F, 0x80, 0, 0];

async fn start() -> Harness {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        schedule(db);
        db.add(Box::new(
            AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
        db.add(Box::new(
            AnalogValueObject::new(3, "AV-3", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
        db.add(Box::new(BinaryValueObject::new(2, "BV-2").unwrap()))
            .unwrap();
    })
    .await;
    h.subscribe_cov().await;
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(10.0)
    );
    h
}

/// AddListElement or RemoveListElement of `elements` on SCH-5's list: the
/// SimpleACK, or the ChangeList-Error's class, code and element number.
async fn list_element(
    h: &mut Harness,
    service: ConfirmedServiceChoice,
    elements: Vec<u8>,
) -> Result<(), (ErrorClass, ErrorCode, u32)> {
    let mut body = BytesMut::new();
    ListElementRequest {
        object_identifier: sch5(),
        property_identifier: LIST,
        property_array_index: None,
        list_of_elements: elements,
    }
    .encode(&mut body)
    .unwrap();
    h.request(service, body).await;
    error_response(h).await.map_err(|pdu| {
        let error = ChangeListError::try_from(&pdu).expect("a ChangeList-Error");
        (
            error.error_class,
            error.error_code,
            error.first_failed_element_number,
        )
    })
}

/// SCH-5's list as a read returns it: local references to these objects.
async fn assert_list(h: &Harness, objects: &[ObjectIdentifier], what: &str) {
    assert_eq!(
        read(h, sch5(), LIST).await,
        PropertyValue::ApplicationData(present_values(objects)),
        "{what}"
    );
}

#[tokio::test(start_paused = true)]
async fn add_commands_the_new_target_and_remove_relinquishes_the_dropped_one() {
    let mut h = start().await;
    list_element(&mut h, ADD, members(&[(av2(), None)]))
        .await
        .unwrap();
    assert_list(&h, &[av1(), av2()], "AV-2 added").await;
    assert_eq!(slot(&h, av2(), 16).await, PropertyValue::Real(10.0));
    // AV-1 is sent its unchanged value again: no notification.
    h.no_notification().await;

    // AV-1 leaves the list: its slot 16 is relinquished and it falls to
    // Relinquish_Default.
    list_element(&mut h, REMOVE, members(&[(av1(), None)]))
        .await
        .unwrap();
    assert_list(&h, &[av2()], "AV-1 removed").await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(0.0));
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );

    // AV-1 no longer follows the Schedule; AV-2 does.
    write_property(
        &mut h,
        PropertyIdentifier::WEEKLY_SCHEDULE,
        Some(2),
        from_three(21.5),
    )
    .await
    .unwrap();
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(21.5));
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(0.0));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn duplicates_succeed_and_absent_or_foreign_members_change_nothing() {
    let mut h = start().await;
    // AV-1 is held already, and a repeat within the request is one member.
    list_element(&mut h, ADD, members(&[(av1(), None), (av1(), None)]))
        .await
        .unwrap();
    assert_list(&h, &[av1()], "a duplicate add").await;
    for (what, service, elements, expected) in [
        (
            "removing a held member and one the list doesn't hold",
            REMOVE,
            members(&[(av1(), None), (bv2(), None)]),
            (ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND, 2),
        ),
        (
            "removing a held member and a Real",
            REMOVE,
            [members(&[(av1(), None)]), REAL.to_vec()].concat(),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2),
        ),
        (
            "adding a member and a Real",
            ADD,
            [members(&[(av2(), None)]), REAL.to_vec()].concat(),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2),
        ),
        (
            "adding a member and an object identifier without its property",
            ADD,
            [
                members(&[(av2(), None)]),
                members(&[(av3(), None)])[..5].to_vec(),
            ]
            .concat(),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, 2),
        ),
    ] {
        assert_eq!(
            list_element(&mut h, service, elements).await,
            Err(expected),
            "{what}"
        );
        assert_list(&h, &[av1()], what).await;
    }
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(0.0));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn add_list_element_names_the_member_that_does_not_fit() {
    let mut h = start().await;
    // AV-1 and 1,022 references to objects the device doesn't have: one
    // short of the 1,024-member cap.
    let stored: Vec<_> = std::iter::once(av1())
        .chain(
            (100..1122)
                .map(|instance| ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()),
        )
        .collect();
    h.server
        .database()
        .write()
        .await
        .get_mut(&sch5())
        .unwrap()
        .write_property(
            LIST,
            None,
            PropertyValue::ApplicationData(present_values(&stored)),
            None,
        )
        .unwrap();
    for (what, elements, element) in [
        (
            "two new members",
            members(&[(av2(), None), (av3(), None)]),
            2,
        ),
        (
            "a held member, then two new ones",
            members(&[(av1(), None), (av2(), None), (av3(), None)]),
            3,
        ),
    ] {
        assert_eq!(
            list_element(&mut h, ADD, elements).await,
            Err((
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
                element
            )),
            "{what}"
        );
        assert_list(&h, &stored, what).await;
    }
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(0.0));

    // The last free place takes one more.
    list_element(&mut h, ADD, members(&[(av2(), None)]))
        .await
        .unwrap();
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(10.0));
}

#[tokio::test(start_paused = true)]
async fn a_member_naming_this_device_is_written_as_a_local_reference() {
    let mut h = start().await;
    // WriteProperty: a local member and AV-2 named with Device 856.
    write_property(
        &mut h,
        LIST,
        None,
        members(&[(av1(), None), (av2(), local())]),
    )
    .await
    .unwrap();
    assert_list(&h, &[av1(), av2()], "WriteProperty").await;
    assert_eq!(slot(&h, av2(), 16).await, PropertyValue::Real(10.0));
    h.no_notification().await;

    // WritePropertyMultiple of AV-2 alone, named the same way: AV-1 is
    // relinquished.
    write_property_multiple(&mut h, vec![(LIST, members(&[(av2(), local())]))])
        .await
        .unwrap();
    assert_list(&h, &[av2()], "WritePropertyMultiple").await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );

    // write_local, mixing both forms: AV-1 is commanded again.
    h.server
        .write_local(
            &sch5(),
            LIST,
            None,
            PropertyValue::ApplicationData(members(&[(av1(), local()), (av3(), None)])),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_list(&h, &[av1(), av3()], "write_local").await;
    assert_eq!(slot(&h, av2(), 16).await, PropertyValue::Null);
    assert_eq!(slot(&h, av3(), 16).await, PropertyValue::Real(10.0));
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(10.0)
    );
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn the_list_services_match_a_member_naming_this_device_to_its_local_form() {
    let mut h = start().await;
    list_element(&mut h, ADD, members(&[(av2(), local())]))
        .await
        .unwrap();
    assert_list(&h, &[av1(), av2()], "AV-2 added with the Device").await;
    assert_eq!(slot(&h, av2(), 16).await, PropertyValue::Real(10.0));

    // AV-1 named with the Device is the member already held.
    list_element(&mut h, ADD, members(&[(av1(), local())]))
        .await
        .unwrap();
    assert_list(&h, &[av1(), av2()], "a duplicate with the Device").await;

    // And a removal finds it.
    list_element(&mut h, REMOVE, members(&[(av1(), local())]))
        .await
        .unwrap();
    assert_list(&h, &[av2()], "AV-1 removed with the Device").await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn a_member_in_another_device_is_refused_in_a_mixed_list() {
    let mut h = start().await;
    // A local member, one naming this device, then AV-2 in Device 9.
    let mixed = members(&[(av2(), None), (av3(), local()), (av2(), remote())]);
    assert_eq!(
        write_property(&mut h, LIST, None, mixed.clone()).await,
        Err(ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED)
    );
    assert_eq!(
        write_property_multiple(&mut h, vec![(LIST, mixed.clone())]).await,
        Err(ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED)
    );
    assert_eq!(
        list_element(&mut h, ADD, mixed).await,
        Err((
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            3
        ))
    );
    // A member in another device is never stored, so no removal finds one.
    assert_eq!(
        list_element(&mut h, REMOVE, members(&[(av1(), remote())])).await,
        Err((ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND, 1))
    );
    assert_list(&h, &[av1()], "after the refusals").await;
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(0.0));
    assert_eq!(read(&h, av3(), PV).await, PropertyValue::Real(0.0));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));

    // Without the remote member the mixed list is accepted.
    list_element(&mut h, ADD, members(&[(av2(), None), (av3(), local())]))
        .await
        .unwrap();
    assert_list(&h, &[av1(), av2(), av3()], "the local members").await;
    assert_eq!(read(&h, av3(), PV).await, PropertyValue::Real(10.0));
    h.no_notification().await;
}
