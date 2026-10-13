use super::*;
use bacnet_server::server::ReinitializeContext;
use bacnet_types::enums::{ErrorClass, ErrorCode, ReinitializedState};
use std::net::Ipv4Addr;

const PASSWORD: &str = "reinit-pw";

type Builder = crate::bip::BipEndpointBuilder;
type Client = bacnet_client::client::BACnetClient<bacnet_transport::bip::BipTransport>;
type Received = Arc<std::sync::Mutex<Vec<ReinitializeContext>>>;

/// A handler that records the context of each request it carries out.
fn recording(
    received: &Received,
) -> impl Fn(&ReinitializeContext, &mut ObjectDatabase) -> Result<(), Error> + Send + Sync + 'static
{
    let recorded = Arc::clone(received);
    move |context: &ReinitializeContext, _: &mut ObjectDatabase| {
        recorded.lock().unwrap().push(context.clone());
        Ok(())
    }
}

fn states(received: &Received) -> Vec<ReinitializedState> {
    received.lock().unwrap().iter().map(|c| c.state).collect()
}

/// A running B/IP endpoint for Device 123, composed by `configure`, a client
/// to reach it, and the endpoint's B/IP address.
async fn running(
    configure: impl FnOnce(Builder) -> Builder,
) -> (
    EndpointSession<bacnet_transport::bip::BipTransport>,
    Client,
    [u8; 6],
) {
    let identity = crate::DeviceIdentity::new(123, 42).unwrap();
    let db = crate::identity::build_database_with_extra(&identity, vec![]).unwrap();
    let builder = Builder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ServerOnly)
        .database(db)
        .identity(identity);
    let mut endpoint = configure(builder).build_session().unwrap();
    endpoint.start().await.unwrap();
    // The endpoint bound port 0; the client talks to the port it actually got.
    let port = endpoint.bip_local_address().unwrap().port();
    let client = Client::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mac = bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], port);
    (endpoint, client, mac)
}

/// The Device's Protocol_Services_Supported, as bit positions.
async fn advertised(client: &Client, mac: &[u8; 6]) -> Vec<usize> {
    let ack = client
        .read_property(
            mac,
            oid(123),
            PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
            None,
        )
        .await
        .unwrap();
    let (profile, _) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    services(profile)
}

async fn reinitialize(
    client: &Client,
    mac: &[u8; 6],
    state: ReinitializedState,
    password: Option<&str>,
) -> Result<(), Error> {
    tokio::time::timeout(
        Duration::from_secs(3),
        client.reinitialize_device(mac, state, password.map(str::to_owned)),
    )
    .await
    .expect("the endpoint answers")
}

fn assert_protocol_error(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    let error = result.unwrap_err();
    assert!(
        matches!(error, Error::Protocol { class: c, code: e }
            if c == class.to_raw() as u32 && e == code.to_raw() as u32),
        "got {error:?}"
    );
}

/// `start` refuses with an error naming `expected`.
async fn assert_refused(
    session: &mut EndpointSession<impl TransportPort + 'static>,
    expected: &str,
) {
    let error = session.start().await.unwrap_err();
    assert!(error.to_string().contains(expected), "got {error}");
}

#[tokio::test]
async fn reinitialize_device_reaches_the_handler() {
    let received = Received::default();
    let (mut endpoint, mut client, mac) = running(|builder| {
        builder
            .reinitialize(recording(&received))
            .reinit_password(PASSWORD)
    })
    .await;

    reinitialize(
        &client,
        &mac,
        ReinitializedState::START_BACKUP,
        Some(PASSWORD),
    )
    .await
    .unwrap();
    assert_eq!(states(&received), vec![ReinitializedState::START_BACKUP]);
    let context = received.lock().unwrap()[0].clone();
    assert_eq!(context.source_mac.len(), 6);
    assert_eq!(context.source_network, None);

    // The Device advertises exactly what the responder executes.
    assert_eq!(advertised(&client, &mac).await, vec![12, 20]);

    client.stop().await.unwrap();
    endpoint.stop().await.unwrap();
}

#[tokio::test]
async fn a_wrong_password_never_reaches_the_handler() {
    let received = Received::default();
    let (mut endpoint, mut client, mac) = running(|builder| {
        builder
            .reinitialize(recording(&received))
            .reinit_password(PASSWORD)
    })
    .await;

    assert_protocol_error(
        reinitialize(
            &client,
            &mac,
            ReinitializedState::START_BACKUP,
            Some("wrong"),
        )
        .await,
        ErrorClass::SECURITY,
        ErrorCode::PASSWORD_FAILURE,
    );
    assert!(received.lock().unwrap().is_empty());

    client.stop().await.unwrap();
    endpoint.stop().await.unwrap();
}

/// With no password configured, any request reaches the handler, with or
/// without a password of its own.
#[tokio::test]
async fn without_a_password_any_request_reaches_the_handler() {
    let received = Received::default();
    let (mut endpoint, mut client, mac) =
        running(|builder| builder.reinitialize(recording(&received))).await;

    reinitialize(&client, &mac, ReinitializedState::WARMSTART, None)
        .await
        .unwrap();
    reinitialize(
        &client,
        &mac,
        ReinitializedState::ACTIVATE_CHANGES,
        Some("anything"),
    )
    .await
    .unwrap();
    assert_eq!(
        states(&received),
        vec![
            ReinitializedState::WARMSTART,
            ReinitializedState::ACTIVATE_CHANGES
        ]
    );

    client.stop().await.unwrap();
    endpoint.stop().await.unwrap();
}

/// With Device writes and ReinitializeDevice both enabled, the Device
/// advertises ReadProperty, WriteProperty and ReinitializeDevice, and
/// executes each.
#[tokio::test]
async fn read_write_and_reinitialize_are_advertised_together() {
    let received = Received::default();
    let (mut endpoint, mut client, mac) = running(|builder| {
        builder
            .device_writes(Arc::new(|_| true))
            .reinitialize(recording(&received))
    })
    .await;

    assert_eq!(advertised(&client, &mac).await, vec![12, 15, 20]);
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut value, "both").unwrap();
    client
        .write_property(
            &mac,
            oid(123),
            PropertyIdentifier::DESCRIPTION,
            None,
            value.to_vec(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        read(&endpoint, PropertyIdentifier::DESCRIPTION).await,
        PropertyValue::CharacterString("both".into())
    );
    reinitialize(&client, &mac, ReinitializedState::COLDSTART, None)
        .await
        .unwrap();
    assert_eq!(states(&received), vec![ReinitializedState::COLDSTART]);

    client.stop().await.unwrap();
    endpoint.stop().await.unwrap();
}

/// A panicking handler is answered SERVICES / OTHER, and the session's
/// dispatch survives it: a ReadProperty afterwards still succeeds.
#[tokio::test]
async fn a_panicking_handler_is_answered_and_the_session_keeps_serving() {
    let (mut endpoint, mut client, mac) = running(|builder| {
        builder.reinitialize(|_: &ReinitializeContext, _: &mut ObjectDatabase| {
            panic!("reinitialize handler panic")
        })
    })
    .await;

    assert_protocol_error(
        reinitialize(&client, &mac, ReinitializedState::WARMSTART, None).await,
        ErrorClass::SERVICES,
        ErrorCode::OTHER,
    );
    let name = tokio::time::timeout(
        Duration::from_secs(3),
        client.read_property(&mac, oid(123), PropertyIdentifier::OBJECT_NAME, None),
    )
    .await
    .expect("the session still answers")
    .unwrap();
    assert!(!name.property_value.is_empty());

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

    assert_refused(
        &mut session,
        "Endpoint identity services do not match the responder's services",
    )
    .await;
}

#[tokio::test]
async fn reinitialize_needs_a_server_role() {
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session =
        EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())
            .unwrap()
            .with_reinitialize(|_: &ReinitializeContext, _: &mut ObjectDatabase| Ok(()));
    assert_refused(&mut session, "ReinitializeDevice requires a server role").await;

    // With Device writes enabled too, the refusal names both.
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session =
        EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())
            .unwrap()
            .with_device_writes(Arc::new(|_| true))
            .with_reinitialize(|_: &ReinitializeContext, _: &mut ObjectDatabase| Ok(()));
    assert_refused(
        &mut session,
        "Device writes and ReinitializeDevice require a server role",
    )
    .await;
}

/// A password with no handler would protect nothing, so startup refuses it,
/// on a session and through the B/IP builder alike.
#[tokio::test]
async fn a_password_without_a_handler_is_refused() {
    const REFUSAL: &str = "a ReinitializeDevice password requires a ReinitializeDevice handler";
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session =
        EndpointSession::new(transport, SessionRole::ServerOnly, SessionConfig::default())
            .unwrap()
            .with_database(database(&[123]))
            .with_reinit_password(PASSWORD);
    assert_refused(&mut session, REFUSAL).await;

    let mut session = Builder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ServerOnly)
        .database(database(&[123]))
        .reinit_password(PASSWORD)
        .build_session()
        .unwrap();
    assert_refused(&mut session, REFUSAL).await;

    let Err(error) = Builder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .reinit_password(PASSWORD)
        .build_transport()
    else {
        panic!("a bare transport cannot keep a ReinitializeDevice password")
    };
    assert!(
        error
            .to_string()
            .contains("ReinitializeDevice and its password require build_session()"),
        "got {error}"
    );
}
