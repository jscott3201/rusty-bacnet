//! The session's registered Network Port on the full server it builds
//! (#2617): the session reserves the port before its link starts and
//! publishes it after, the server serves it for wildcard reads, the number
//! the session's Number owner learns lands in the server's database, where
//! the server reads it, and written values wait for activation only when
//! the builder has a ReinitializeDevice handler to activate them.
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

/// A multicast group the port can change, so the Network Port lists
/// BACnet_IP_Multicast_Address as writable while its owner activates
/// written values.
struct Group;

impl bacnet_types::bip_port::IpMulticast for Group {
    fn group(&self) -> [u8; 4] {
        [0; 4]
    }
    fn check(&self, _: [u8; 4]) -> Result<(), Error> {
        Ok(())
    }
    fn activate(&self, _: [u8; 4]) -> Result<(), Error> {
        Ok(())
    }
    fn can_change(&self) -> bool {
        true
    }
}

/// A session of both roles on a capture link like [`capture`]'s, whose
/// B/IP port lends [`Group`].
fn multicast_endpoint() -> Endpoint {
    let (inbound, input) = mpsc::channel(32);
    let (outbound, sent) = mpsc::channel(32);
    let port = bacnet_transport::port::BipPort::new(
        host(SELF),
        bacnet_types::bip_port::BipPortMode::Normal,
    )
    .with_multicast(Some(Arc::new(Group)));
    let transport = TestTransport::builder()
        .local_mac(&mac(SELF))
        .inbound(input)
        .number_controls()
        .bip_port(port)
        .on_bip_broadcast_endpoint(|| Some(host(255)))
        .on_send(move |frame| {
            outbound
                .try_send(Sent {
                    link: frame.mac,
                    npdu: frame.npdu,
                })
                .expect("bounded send observation");
            async { Ok(()) }
        })
        .build();
    Endpoint {
        session: EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())
            .unwrap(),
        inbound,
        sent,
    }
}

/// A peer's write of the port's BACnet_IP_Multicast_Address waits for
/// activation when the builder has a ReinitializeDevice handler, as on a
/// server's own link; without one the row is read-only.
#[tokio::test(start_paused = true)]
async fn written_port_values_follow_the_builders_reinitialize_handler() {
    use bacnet_services::write_property::WritePropertyRequest;
    let port = oid(ObjectType::NETWORK_PORT, 2);
    for handler in [true, false] {
        let identity = crate::DeviceIdentity::new(123, 42)
            .unwrap()
            .with_bip_port(2, 0, *host(SELF).ip(), PORT)
            .unwrap();
        let db = identity.build_database().unwrap();
        let builder = BACnetServer::generic_builder();
        let builder = if handler {
            builder.on_reinitialize(|_, _| Ok(()))
        } else {
            builder
        };
        let mut endpoint = multicast_endpoint();
        endpoint.session = endpoint
            .session
            .with_database(db)
            .device(identity)
            .with_registered_network_port(port)
            .with_server(builder);
        endpoint.start(false).await;
        let mut service = BytesMut::new();
        WritePropertyRequest {
            object_identifier: port,
            property_identifier: PropertyIdentifier::BACNET_IP_MULTICAST_ADDRESS,
            property_array_index: None,
            // An application-tagged OctetString, 239.1.2.3.
            property_value: vec![0x64, 239, 1, 2, 3],
            priority: None,
        }
        .encode(&mut service)
        .unwrap();
        let write = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 9,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: service.freeze(),
        });
        endpoint.deliver_apdu(write, mac(PEER), None).await;
        let answer = decode_apdu(decode_npdu(endpoint.next().await.npdu).unwrap().payload).unwrap();
        if handler {
            assert!(
                matches!(&answer, Apdu::SimpleAck(ack) if ack.invoke_id == 9),
                "{answer:?}"
            );
        } else {
            assert!(
                matches!(&answer, Apdu::Error(error) if error.error_code
                    == bacnet_types::enums::ErrorCode::WRITE_ACCESS_DENIED),
                "{answer:?}"
            );
        }
        endpoint.stop().await;
    }
}
