//! The server guide's object limit (#1989): CreateObject from the network
//! stops at 10,000 objects in the database, with RESOURCES /
//! NO_SPACE_FOR_OBJECT. Objects the application adds itself aren't capped.
#![forbid(unsafe_code)]

use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::object_mgmt::ObjectSpecifier;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::RemoteError;

const STATION: u8 = 0x05;

#[tokio::test]
async fn network_creation_stops_at_ten_thousand_objects() {
    let identity = bacnet_objects::DeviceIdentity::new(1001, 0).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    // The application may hold more; it fills the database to one short.
    for instance in 1..9_999 {
        db.add(Box::new(
            AnalogValueObject::new(
                instance,
                format!("Value {instance}"),
                EngineeringUnits::NO_UNITS,
            )
            .unwrap(),
        ))
        .unwrap();
    }
    assert_eq!(db.len(), 9_999);

    let (client_end, server_end) = LoopbackTransport::pair(vec![0x0A], vec![STATION]);
    let mut server = BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(server_end)
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_end)
        .build()
        .await
        .unwrap();

    let create = || {
        client.create_object(
            &[STATION],
            ObjectSpecifier::Type(ObjectType::ANALOG_INPUT),
            Vec::new(),
        )
    };
    // The 10,000th object is created; the next is refused.
    create().await.unwrap();
    assert_eq!(server.database().read().await.len(), 10_000);
    let refused = create().await.unwrap_err();
    assert_eq!(
        refused.remote(),
        Some(RemoteError::Error {
            class: ErrorClass::RESOURCES,
            code: ErrorCode::NO_SPACE_FOR_OBJECT,
        }),
        "{refused:?}"
    );
    assert_eq!(server.database().read().await.len(), 10_000);

    // The application itself can still add one past the limit.
    server
        .database()
        .write()
        .await
        .add(Box::new(
            AnalogValueObject::new(
                10_000,
                "Added by the application",
                EngineeringUnits::NO_UNITS,
            )
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(server.database().read().await.len(), 10_001);

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
