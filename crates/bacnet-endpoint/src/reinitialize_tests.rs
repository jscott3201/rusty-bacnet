use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode, ReinitializedState};
use std::net::{Ipv4Addr, UdpSocket};

const PASSWORD: &str = "reinit-pw";

type Received = Arc<std::sync::Mutex<Vec<ReinitializedState>>>;

/// A running endpoint whose ReinitializeDevice handler records each state it is asked for,
/// a client to reach it, and the endpoint's B/IP address.
async fn reinitializing_endpoint() -> (
    EndpointSession<bacnet_transport::bip::BipTransport>,
    bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>,
    [u8; 6],
    Received,
) {
    let reservation = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let identity = crate::DeviceIdentity::new(123, 42).unwrap();
    let db = crate::identity::build_database_with_extra(&identity, vec![]).unwrap();
    let received: Received = Arc::default();
    let recorded = Arc::clone(&received);

    let mut endpoint =
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::BROADCAST)
            .role(SessionRole::ServerOnly)
            .database(db)
            .identity(identity)
            .reinitialize(move |state, _database| {
                recorded.lock().unwrap().push(state);
                Ok(())
            })
            .reinit_password(PASSWORD)
            .build_session()
            .unwrap();
    endpoint.start().await.unwrap();

    let client = bacnet_client::client::BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mac = bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], port);

    (endpoint, client, mac, received)
}

#[tokio::test]
async fn reinitialize_device_reaches_the_handler() {
    let (mut endpoint, mut client, mac, received) = reinitializing_endpoint().await;

    client
        .reinitialize_device(
            &mac,
            ReinitializedState::START_BACKUP,
            Some(PASSWORD.to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(
        *received.lock().unwrap(),
        vec![ReinitializedState::START_BACKUP]
    );

    // The Device advertises exactly what the responder executes.
    let profile = client
        .read_property(
            &mac,
            oid(123),
            PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
            None,
        )
        .await
        .unwrap();
    let (profile, _) =
        bacnet_encoding::primitives::decode_application_value(&profile.property_value, 0).unwrap();
    assert_eq!(services(profile), vec![12, 20]);

    client.stop().await.unwrap();
    endpoint.stop().await.unwrap();
}

#[tokio::test]
async fn a_wrong_password_never_reaches_the_handler() {
    let (mut endpoint, mut client, mac, received) = reinitializing_endpoint().await;

    let error = client
        .reinitialize_device(
            &mac,
            ReinitializedState::START_BACKUP,
            Some("wrong".to_owned()),
        )
        .await
        .unwrap_err();

    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::SECURITY.to_raw() as u32
                && code == ErrorCode::PASSWORD_FAILURE.to_raw() as u32),
        "got {error:?}"
    );
    assert!(received.lock().unwrap().is_empty());

    client.stop().await.unwrap();
    endpoint.stop().await.unwrap();
}

/// An identity may not advertise ReinitializeDevice unless the session executes it.
#[tokio::test]
async fn an_identity_advertising_reinitialize_needs_a_handler() {
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let identity = crate::DeviceIdentity::new(123, 42)
        .unwrap()
        .with_services(&[
            ServiceSupported::READ_PROPERTY,
            ServiceSupported::REINITIALIZE_DEVICE,
        ]);
    let db = crate::identity::build_database_with_extra(&identity, vec![]).unwrap();
    let mut session =
        EndpointSession::new(transport, SessionRole::ServerOnly, SessionConfig::default())
            .unwrap()
            .with_database(db)
            .with_identity(identity);

    assert!(session.start().await.is_err());
}

#[tokio::test]
async fn reinitialize_needs_a_server_role() {
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session =
        EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())
            .unwrap()
            .with_reinitialize(|_, _| Ok(()));

    assert!(session.start().await.is_err());
}
