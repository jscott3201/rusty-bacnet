//! The session's registered Network Port on the full server it builds
//! (#2617): the session reserves the port before its link starts and
//! publishes it after, the server serves it for wildcard reads, and the
//! number the session's Number owner learns lands in the server's
//! database, where the server reads it.
use super::*;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_server::server::BACnetServer;

/// The Network Port instance a read names to mean the port the request
/// came in on.
const WILDCARD: u32 = ObjectIdentifier::MAX_INSTANCE;

/// A ReadProperty of the wildcard Network Port's `property` under
/// `invoke_id`.
fn wildcard_read(property: PropertyIdentifier, invoke_id: u8) -> Apdu {
    let mut service = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::NETWORK_PORT, WILDCARD),
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut service);
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: service.freeze(),
    })
}

/// `value` as a ReadPropertyACK carries it.
fn encoded(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

impl Endpoint {
    /// The peer reads the wildcard port's `property`: the value the node's
    /// answer carries.
    async fn read_port(&mut self, property: PropertyIdentifier, invoke_id: u8) -> Vec<u8> {
        self.deliver_apdu(wildcard_read(property, invoke_id), mac(PEER), None)
            .await;
        let sent = self.next().await;
        assert_eq!(sent.link, mac(PEER));
        let npdu = decode_npdu(sent.npdu).unwrap();
        let Apdu::ComplexAck(ack) = decode_apdu(npdu.payload).unwrap() else {
            panic!("a ComplexAck for {property:?}");
        };
        assert_eq!(ack.invoke_id, invoke_id);
        ReadPropertyACK::decode(&ack.service_ack)
            .unwrap()
            .property_value
    }
}

#[tokio::test(start_paused = true)]
async fn the_full_server_serves_the_sessions_registered_port() {
    let port = oid(ObjectType::NETWORK_PORT, 2);
    // No configured number: the session learns it.
    let identity = crate::DeviceIdentity::new(123, 42)
        .unwrap()
        .with_bip_port(2, 0, *host(SELF).ip(), PORT)
        .unwrap();
    let db = identity.build_database().unwrap();
    let mut endpoint = Endpoint::new(SessionRole::Both);
    endpoint.session = endpoint
        .session
        .with_database(db)
        .device(identity)
        .with_registered_network_port(port)
        .with_server(BACnetServer::generic_builder());
    endpoint.start(true).await;
    assert!(endpoint.session.server().is_some());
    assert_eq!(
        endpoint
            .read_port(PropertyIdentifier::OBJECT_IDENTIFIER, 5)
            .await,
        encoded(PropertyValue::ObjectIdentifier(port)),
        "the wildcard names the session's port"
    );
    assert_eq!(
        endpoint
            .read_port(PropertyIdentifier::NETWORK_NUMBER, 6)
            .await,
        encoded(PropertyValue::Unsigned(THIS_NETWORK.into())),
        "the learned number is in the server's database"
    );
    let counters = endpoint.session.policy_counters().await;
    assert_eq!(counters.no_server_role, 0);
    endpoint.stop().await;
}
