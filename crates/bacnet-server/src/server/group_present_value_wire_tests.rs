//! A Group's Present_Value over B/IP (#1134): a running server rebuilds it
//! from the members on every ReadProperty and ReadPropertyMultiple, reading
//! each member as it would serve that property itself.
//!
//! The element octets are pinned by `handlers::tests::read_rpm::
//! group_present_value`; here the served path must agree with them, read a
//! Device member through the executor's view and follow a member's change.

use super::*;
use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::device::DeviceConfig;
use bacnet_objects::group::GroupObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::EngineeringUnits;
use std::net::Ipv4Addr;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn member(
    object_identifier: ObjectIdentifier,
    properties: &[PropertyIdentifier],
) -> ReadAccessSpecification {
    ReadAccessSpecification {
        object_identifier,
        list_of_property_references: properties
            .iter()
            .map(|&property_identifier| PropertyReference {
                property_identifier,
                property_array_index: None,
            })
            .collect(),
    }
}

/// Split a Present_Value into its ReadAccessResult elements' octets.
fn elements(present_value: &[u8]) -> Vec<Vec<u8>> {
    ReadPropertyMultipleACK::decode(present_value)
        .unwrap()
        .list_of_read_access_results
        .iter()
        .map(|result| {
            let mut encoded = BytesMut::new();
            result.encode(&mut encoded);
            encoded.to_vec()
        })
        .collect()
}

#[tokio::test]
async fn group_present_value_bip_wire_rebuilds_from_members() {
    let device = oid(ObjectType::DEVICE, 100);
    let input = oid(ObjectType::ANALOG_INPUT, 1);
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    ai.set_present_value(21.5);
    let mut group_object = GroupObject::new(7, "GRP-7").unwrap();
    let group = group_object.object_identifier();
    for spec in [
        member(
            input,
            &[
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::DESCRIPTION,
            ],
        ),
        member(
            oid(ObjectType::ANALOG_INPUT, 9),
            &[PropertyIdentifier::PRESENT_VALUE],
        ),
        member(device, &[PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED]),
    ] {
        group_object.add_member(spec).unwrap();
    }
    let mut database = ObjectDatabase::new();
    database
        .add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(100, 0)
                    .expect("valid Device identity")
                    .with_name("DEV-100"),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
    database.add(Box::new(ai)).unwrap();
    database.add(Box::new(group_object)).unwrap();

    let mut server = BACnetServer::bip_builder()
        .device(bacnet_objects::DeviceIdentity::new(100, 0).unwrap())
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

    let present_value = client
        .read_property(&mac, group, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap()
        .property_value
        .to_vec();
    let served = elements(&present_value);
    assert_eq!(served.len(), 3);
    // AI-1 reads REAL 21.5 and an empty Description; AI-9 isn't here.
    assert_eq!(
        served[0],
        [
            0x0C, 0, 0, 0, 1, 0x1E, 0x29, 85, 0x4E, 0x44, 0x41, 0xAC, 0, 0, 0x4F, 0x29, 28, 0x4E,
            0x71, 0, 0x4F, 0x1F,
        ]
    );
    assert_eq!(
        served[1],
        [
            0x0C, 0, 0, 0, 9, 0x1E, 0x29, 85, 0x5E, 0x91, 1, 0x91, 31, 0x5F, 0x1F
        ]
    );
    // The Device member carries the services this server executes, the
    // value a direct read of the Device returns.
    let services = client
        .read_property(
            &mac,
            device,
            PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
            None,
        )
        .await
        .unwrap()
        .property_value;
    let mut expected = vec![0x0C, 0x02, 0, 0, 100, 0x1E, 0x29, 97, 0x4E];
    expected.extend_from_slice(&services);
    expected.extend_from_slice(&[0x4F, 0x1F]);
    assert_eq!(served[2], expected);

    // ReadPropertyMultiple, explicitly and through ALL, and the local read
    // serve the same list.
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![member(
            group,
            &[PropertyIdentifier::PRESENT_VALUE, PropertyIdentifier::ALL],
        )],
    }
    .encode(&mut request)
    .unwrap();
    let response = client
        .confirmed_request(
            &mac,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            &request,
        )
        .await
        .unwrap();
    let ack = ReadPropertyMultipleACK::decode(&response).unwrap();
    let rows: Vec<_> = ack.list_of_read_access_results[0]
        .list_of_results
        .iter()
        .filter(|row| row.property_identifier == PropertyIdentifier::PRESENT_VALUE)
        .collect();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row.property_value.as_deref(), Some(&present_value[..]));
    }
    let PropertyValue::List(local) = server
        .read_local(&group, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap()
    else {
        panic!("Present_Value is a list");
    };
    assert_eq!(
        local
            .iter()
            .map(|element| match element {
                PropertyValue::ApplicationData(bytes) => bytes.clone(),
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>(),
        served
    );

    // A member's change shows on the next read, with nothing stored between.
    client
        .write_property(
            &mac,
            input,
            PropertyIdentifier::DESCRIPTION,
            None,
            vec![0x74, 0, b'h', b'o', b't'],
            None,
        )
        .await
        .unwrap();
    let after = elements(
        &client
            .read_property(&mac, group, PropertyIdentifier::PRESENT_VALUE, None)
            .await
            .unwrap()
            .property_value,
    );
    assert_eq!(
        after[0],
        [
            0x0C, 0, 0, 0, 1, 0x1E, 0x29, 85, 0x4E, 0x44, 0x41, 0xAC, 0, 0, 0x4F, 0x29, 28, 0x4E,
            0x74, 0, b'h', b'o', b't', 0x4F, 0x1F,
        ]
    );
    assert_eq!(after[1..], served[1..]);
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
