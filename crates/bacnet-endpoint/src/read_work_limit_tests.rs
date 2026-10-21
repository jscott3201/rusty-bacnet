//! The server role's ReadProperty work limit is the session's
//! `read_work_limit` (#1215): a Group's Present_Value counts its own row plus
//! each member row, and a read past the limit is aborted with
//! OUT_OF_RESOURCES, as the full server aborts it.
use std::net::Ipv4Addr;

use bacnet_objects::group::GroupObject;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::AbortReason;

use super::*;
use crate::bip::BipEndpointBuilder;
use crate::identity::DeviceIdentity;

const DEVICE: u32 = 123;

fn group(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::GROUP, instance).unwrap()
}

/// Group 1 reports two Device properties, so its Present_Value is three rows;
/// Group 2 reports one, two rows.
fn database(identity: &DeviceIdentity) -> ObjectDatabase {
    let mut db = identity.build_database().unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, DEVICE).unwrap();
    for (instance, properties) in [
        (
            1,
            &[
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::OBJECT_TYPE,
            ][..],
        ),
        (2, &[PropertyIdentifier::OBJECT_NAME][..]),
    ] {
        let mut object = GroupObject::new(instance, format!("GRP-{instance}")).unwrap();
        object
            .add_member(ReadAccessSpecification {
                object_identifier: device,
                list_of_property_references: properties
                    .iter()
                    .map(|&property_identifier| PropertyReference {
                        property_identifier,
                        property_array_index: None,
                    })
                    .collect(),
            })
            .unwrap();
        db.add(Box::new(object)).unwrap();
    }
    db
}

/// ReadProperty of each Group's Present_Value from a server-only endpoint,
/// with `limit` set on the builder when given.
async fn read_groups(limit: Option<usize>) -> Vec<Result<(), Error>> {
    let identity = DeviceIdentity::new(DEVICE, 42).unwrap();
    let mut builder = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ServerOnly)
        .database(database(&identity))
        .device(identity);
    if let Some(limit) = limit {
        builder = builder.read_work_limit(limit);
    }
    let mut endpoint = builder.build_session().unwrap();
    endpoint.start().await.unwrap();
    let port = endpoint.bip_local_address().unwrap().port();
    let mut client = bacnet_client::client::BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mac = bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], port);
    let mut results = Vec::new();
    for instance in [1, 2] {
        results.push(
            client
                .read_property(
                    &mac,
                    group(instance),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                )
                .await
                .map(|_| ()),
        );
    }
    client.stop().await.unwrap();
    endpoint.stop().await.unwrap();
    results
}

#[tokio::test]
async fn endpoint_group_read_past_the_read_work_limit_aborts_out_of_resources() {
    let results = read_groups(Some(2)).await;
    assert!(
        matches!(
            results[0],
            Err(Error::Abort { reason }) if reason == AbortReason::OUT_OF_RESOURCES
        ),
        "{results:?}"
    );
    assert!(results[1].is_ok(), "{results:?}");
    // The default limit, 256, serves both.
    assert!(read_groups(None).await.iter().all(Result::is_ok));
}

#[test]
fn endpoint_read_work_limit_defaults_to_256_and_refuses_zero() {
    assert_eq!(SessionConfig::default().read_work_limit, 256);
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let zero = SessionConfig {
        read_work_limit: 0,
        ..SessionConfig::default()
    };
    assert!(matches!(
        EndpointSession::new(transport, SessionRole::ServerOnly, zero),
        Err(Error::Encoding(message)) if message.contains("read work limit")
    ));
    let identity = DeviceIdentity::new(DEVICE, 42).unwrap();
    assert!(
        BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .database(database(&identity))
            .device(identity)
            .read_work_limit(0)
            .build_session()
            .is_err()
    );
}
