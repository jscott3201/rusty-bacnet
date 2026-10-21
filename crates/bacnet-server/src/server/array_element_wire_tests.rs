//! Per-index reads of the Structured View and Command arrays over B/IP
//! (#1135): ReadProperty and ReadPropertyMultiple from a real client.
//!
//! The element octets are pinned by `handlers::tests::read_rpm::{group,
//! command}`; here the wire reply must match the handler's, each element
//! must read alone as the octets the whole-array read concatenates, and an
//! index past the end must fail in both services.

use super::*;
use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_objects::command::CommandObject;
use bacnet_objects::group::StructuredViewObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetDeviceObjectReference, PropertyReference,
    ReadAccessSpecification,
};
use std::net::Ipv4Addr;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A Structured View with three subordinates, one in another device.
fn structured_view() -> StructuredViewObject {
    let mut view = StructuredViewObject::new(7, "SV-7").unwrap();
    view.add_subordinate(oid(ObjectType::ANALOG_INPUT, 1), "a")
        .unwrap();
    view.add_subordinate(
        BACnetDeviceObjectReference {
            device_identifier: Some(oid(ObjectType::DEVICE, 9)),
            object_identifier: oid(ObjectType::BINARY_INPUT, 1),
        },
        "b",
    )
    .unwrap();
    view.add_subordinate(oid(ObjectType::ANALOG_VALUE, 2), "c")
        .unwrap();
    view
}

/// A Command with three action lists, the middle one empty.
fn command() -> CommandObject {
    let write = |instance, value| BACnetActionCommand {
        device_identifier: None,
        object_identifier: oid(ObjectType::ANALOG_OUTPUT, instance),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: PropertyValue::Real(value),
        priority: Some(8),
        post_delay: None,
        quit_on_failure: false,
        write_successful: true,
    };
    let mut command = CommandObject::new(7, "CMD-7").unwrap();
    command
        .set_action(vec![
            BACnetActionList {
                commands: vec![write(1, 50.0)],
            },
            BACnetActionList::default(),
            BACnetActionList {
                commands: vec![write(1, 0.0), write(2, 21.5)],
            },
        ])
        .unwrap();
    command
}

fn is_invalid_array_index(error: &Error) -> bool {
    matches!(error, Error::Protocol { class, code }
        if *class == ErrorClass::PROPERTY
            && *code == ErrorCode::INVALID_ARRAY_INDEX)
}

#[tokio::test]
async fn array_elements_bip_wire_rp_and_rpm_serve_each_index() {
    let view = structured_view();
    let command = command();
    let targets = [
        (
            view.object_identifier(),
            PropertyIdentifier::SUBORDINATE_LIST,
        ),
        (
            view.object_identifier(),
            PropertyIdentifier::SUBORDINATE_ANNOTATIONS,
        ),
        (command.object_identifier(), PropertyIdentifier::ACTION),
    ];
    let indexes = [
        None,
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(u32::MAX),
    ];
    let mut database = ObjectDatabase::new();
    database.add(Box::new(view)).unwrap();
    database.add(Box::new(command)).unwrap();

    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: [targets[0].0, targets[2].0]
            .into_iter()
            .map(|object_identifier| ReadAccessSpecification {
                object_identifier,
                list_of_property_references: targets
                    .iter()
                    .filter(|&&(target, _)| target == object_identifier)
                    .flat_map(|&(_, property_identifier)| {
                        indexes
                            .iter()
                            .map(move |&property_array_index| PropertyReference {
                                property_identifier,
                                property_array_index,
                            })
                    })
                    .collect(),
            })
            .collect(),
    }
    .encode(&mut request)
    .unwrap();
    let mut expected = BytesMut::new();
    handlers::handle_read_property_multiple(&database, &request, &mut expected).unwrap();

    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(database)
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mac = server.local_mac().to_vec();

    // ReadProperty, one index at a time.
    let mut read = Vec::new();
    for &(object, property) in &targets {
        let mut values = Vec::new();
        for &index in &indexes {
            let reply = client.read_property(&mac, object, property, index).await;
            values.push(reply.map(|ack| {
                assert_eq!(ack.property_array_index, index);
                ack.property_value.to_vec()
            }));
        }
        let [whole, size, first, second, third, past_end, max] = values.try_into().unwrap();
        let [whole, size, first, second, third] =
            [whole, size, first, second, third].map(Result::unwrap);
        assert_eq!(
            decode_application_value(&size, 0).unwrap(),
            (PropertyValue::Unsigned(3), size.len()),
            "{property:?}[0]"
        );
        assert_eq!(
            whole,
            [&first[..], &second[..], &third[..]].concat(),
            "{property:?}"
        );
        for error in [past_end, max] {
            assert!(is_invalid_array_index(&error.unwrap_err()), "{property:?}");
        }
        read.push([whole, size, first, second, third]);
    }

    // One ReadPropertyMultiple over every coordinate: the same octets, and
    // the index echoed on errors too.
    let response = client
        .confirmed_request(
            &mac,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            &request,
        )
        .await
        .unwrap();
    assert_eq!(response.as_ref(), &expected[..]);
    let ack = ReadPropertyMultipleACK::decode(&response).unwrap();
    let results: Vec<_> = ack
        .list_of_read_access_results
        .iter()
        .flat_map(|access| &access.list_of_results)
        .collect();
    assert_eq!(results.len(), targets.len() * indexes.len());
    for ((&(_, property), values), rows) in
        targets.iter().zip(&read).zip(results.chunks(indexes.len()))
    {
        for (row, &index) in rows.iter().zip(&indexes) {
            assert_eq!(row.property_identifier, property);
            assert_eq!(row.property_array_index, index);
        }
        for (row, value) in rows.iter().zip(values) {
            assert_eq!(
                row.property_value.as_deref(),
                Some(&value[..]),
                "{property:?}"
            );
        }
        for row in &rows[values.len()..] {
            assert_eq!(
                row.error,
                Some((ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX)),
                "{property:?}"
            );
        }
    }
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
