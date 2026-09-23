//! RB-15 basic session tests (Loopback, deterministic, no sleeps).
//!
//! Covers: one start/one stop, client-only, server-only, both roles,
//! concurrent request/response with equal inbound/outbound IDs.

use std::time::Duration;

use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

const WAIT: Duration = Duration::from_secs(2);

fn session_config() -> SessionConfig {
    SessionConfig {
        queue_capacity: 16,
        apdu_timeout_ms: 1_000,
        apdu_retries: 0,
        max_apdu_length: 480,
    }
}

fn database_with_analog(instance: u32, value: f32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut analog = AnalogInputObject::new(instance, "test-input", 0).unwrap();
    analog.set_present_value(value);
    db.add(Box::new(analog)).unwrap();
    db
}

fn object_id(instance: u32) -> bacnet_types::primitives::ObjectIdentifier {
    bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}

#[tokio::test]
async fn start_once_stop_once() {
    let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session = EndpointSession::new(transport, SessionRole::Both, session_config()).unwrap();
    session.start().await.unwrap();
    // Second start fails (start-once).
    assert!(session.start().await.is_err());
    session.stop().await.unwrap();
    // Second stop fails (stop-once).
    assert!(session.stop().await.is_err());
}

#[tokio::test]
async fn client_only_and_server_only_compose() {
    // Client-only session has a client handle, no server handle.
    let (client_transport, server_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client =
        EndpointSession::new(client_transport, SessionRole::ClientOnly, session_config()).unwrap();
    let mut server =
        EndpointSession::new(server_transport, SessionRole::ServerOnly, session_config())
            .unwrap()
            .with_database(database_with_analog(7, 42.0));
    client.start().await.unwrap();
    server.start().await.unwrap();
    assert!(client.client().is_some());
    assert!(client.server().is_none());
    assert!(server.server().is_some());
    assert!(server.client().is_none());

    let ack = tokio::time::timeout(
        WAIT,
        client.client().unwrap().read_property(
            &[0x02],
            object_id(7),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ),
    )
    .await
    .expect("client read timed out")
    .expect("client read failed");
    assert_eq!(ack.object_identifier, object_id(7));

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[tokio::test]
async fn both_roles_round_trip() {
    let (transport_a, transport_b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session_a = EndpointSession::new(transport_a, SessionRole::Both, session_config())
        .unwrap()
        .with_database(database_with_analog(1, 11.0));
    let mut session_b = EndpointSession::new(transport_b, SessionRole::Both, session_config())
        .unwrap()
        .with_database(database_with_analog(2, 22.0));
    session_a.start().await.unwrap();
    session_b.start().await.unwrap();

    let ack = tokio::time::timeout(
        WAIT,
        session_a.client().unwrap().read_property(
            &[0x02],
            object_id(2),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ),
    )
    .await
    .expect("A->B timed out")
    .expect("A->B failed");
    assert_eq!(ack.object_identifier, object_id(2));

    let ack = tokio::time::timeout(
        WAIT,
        session_b.client().unwrap().read_property(
            &[0x01],
            object_id(1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ),
    )
    .await
    .expect("B->A timed out")
    .expect("B->A failed");
    assert_eq!(ack.object_identifier, object_id(1));

    session_a.stop().await.unwrap();
    session_b.stop().await.unwrap();
}

#[tokio::test]
async fn concurrent_equal_inbound_outbound_ids_stay_unambiguous() {
    // Both sessions start with empty coordinators, so both outbound requests
    // use numeric invoke ID 0 while each also serves an inbound request with
    // wire ID 0. The classifier (request vs terminal by PDU type) + coordinator
    // owner (Requester vs Notification) keeps them unambiguous.
    let (transport_a, transport_b) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut session_a = EndpointSession::new(transport_a, SessionRole::Both, session_config())
        .unwrap()
        .with_database(database_with_analog(1, 1.0));
    let mut session_b = EndpointSession::new(transport_b, SessionRole::Both, session_config())
        .unwrap()
        .with_database(database_with_analog(1, 2.0));
    session_a.start().await.unwrap();
    session_b.start().await.unwrap();
    assert_eq!(session_a.active_leases(), 0);
    assert_eq!(session_b.active_leases(), 0);

    let client_a = session_a.cloned_client_handle().unwrap();
    let client_b = session_b.cloned_client_handle().unwrap();
    let (result_a, result_b) = tokio::join!(
        client_a.read_property(
            &[0x02],
            object_id(1),
            PropertyIdentifier::PRESENT_VALUE,
            None
        ),
        client_b.read_property(
            &[0x01],
            object_id(1),
            PropertyIdentifier::PRESENT_VALUE,
            None
        ),
    );
    // Both use invoke ID 0 on the wire; both must deliver (no cross-talk).
    assert!(result_a.is_ok(), "A->B failed: {result_a:?}");
    assert!(result_b.is_ok(), "B->A failed: {result_b:?}");
    assert_eq!(session_a.active_leases(), 0);
    assert_eq!(session_b.active_leases(), 0);

    session_a.stop().await.unwrap();
    session_b.stop().await.unwrap();
}
