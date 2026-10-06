use super::*;

#[tokio::test]
async fn registered_port_invalid_selection_profile_and_identity_fail_before_publication() {
    for selected in [
        oid(ObjectType::NETWORK_PORT, 0),
        oid(ObjectType::NETWORK_PORT, 256),
        oid(ObjectType::NETWORK_PORT, 3),
        device(),
    ] {
        let id = identity();
        let mut endpoint =
            crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
                .identity(id.clone())
                .database(id.build_database().unwrap())
                .registered_network_port(selected)
                .build_session()
                .unwrap();
        assert!(endpoint.start().await.is_err());
        assert!(endpoint.bip_local_address().is_none());
    }
    // BBMD and foreign-device links register in their own mode (#939); a
    // link configured as both, a wildcard interface and an identity mismatch
    // are refused.
    for mode in 1..4 {
        let id = if mode == 2 {
            crate::DeviceIdentity::new(785, 555)
                .unwrap()
                .with_bip_port(2, 17, Ipv4Addr::UNSPECIFIED, 0)
                .unwrap()
        } else {
            identity()
        };
        let interface = if mode == 2 {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        let db = id.build_database().unwrap();
        let mut builder = crate::bip::BipEndpointBuilder::new(interface, 0, Ipv4Addr::BROADCAST)
            .database(db)
            .registered_network_port(port());
        builder = match mode {
            1 => builder
                .identity(id)
                .enable_bbmd(vec![])
                .register_as_foreign_device(bacnet_transport::bip::ForeignDeviceConfig {
                    bbmd_ip: Ipv4Addr::LOCALHOST,
                    bbmd_port: 47808,
                    ttl: 60,
                }),
            2 => builder.identity(id),
            _ => builder.identity(
                crate::DeviceIdentity::new(785, 555)
                    .unwrap()
                    .with_bip_port(2, 18, Ipv4Addr::LOCALHOST, 0)
                    .unwrap(),
            ),
        };
        let mut endpoint = builder.build_session().unwrap();
        let error = endpoint.start().await.unwrap_err().to_string();
        if mode == 1 {
            assert!(
                error.contains("a BBMD that also registers as a foreign device"),
                "{error}"
            );
        }
        assert!(endpoint.bip_local_address().is_none());
        assert!(endpoint
            .database
            .as_ref()
            .unwrap()
            .write()
            .await
            .remove(&port())
            .unwrap()
            .is_some());
    }
    let id = identity();
    let (transport, _peer) = bacnet_transport::loopback::LoopbackTransport::pair(vec![1], vec![2]);
    let mut endpoint =
        EndpointSession::new(transport, SessionRole::ServerOnly, SessionConfig::default())
            .unwrap()
            .with_database(id.build_database().unwrap())
            .with_identity(id)
            .with_registered_network_port(port());
    assert!(endpoint.start().await.is_err());
}

/// A BBMD or foreign-device endpoint registers its port in that mode and
/// serves the mode's rows (#939).
#[tokio::test]
async fn registered_port_publishes_the_bbmd_or_foreign_mode() {
    for bbmd in [true, false] {
        let stand_in = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let id = identity();
        let db = id.build_database().unwrap();
        let mut builder =
            crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
                .role(SessionRole::ServerOnly)
                .identity(id)
                .database(db)
                .registered_network_port(port());
        builder = if bbmd {
            builder.enable_bbmd(vec![])
        } else {
            builder.register_as_foreign_device(bacnet_transport::bip::ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::LOCALHOST,
                bbmd_port: stand_in.local_addr().unwrap().port(),
                ttl: 60,
            })
        };
        let mut endpoint = builder.build_session().unwrap();
        endpoint.start().await.unwrap();
        let address = endpoint.bip_local_address().unwrap();
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (_, mode) = read(&peer, address, port(), P::BACNET_IP_MODE).await;
        assert_eq!(mode, PropertyValue::Enumerated(if bbmd { 2 } else { 1 }));
        let (property, expected) = if bbmd {
            (
                P::BBMD_ACCEPT_FD_REGISTRATIONS,
                PropertyValue::Boolean(false),
            )
        } else {
            (P::FD_SUBSCRIPTION_LIFETIME, PropertyValue::Unsigned(60))
        };
        assert_eq!(read(&peer, address, port(), property).await.1, expected);
        endpoint.stop().await.unwrap();
    }
}

#[tokio::test]
async fn registered_port_capacity_stays_independent_of_device_limit_and_row_order() {
    for full in [true, false] {
        let id = crate::DeviceIdentity::new(785, 555)
            .unwrap()
            .with_max_apdu(480)
            .unwrap()
            .with_bip_port(2, 17, Ipv4Addr::LOCALHOST, 0)
            .unwrap()
            .with_bip_port(1, 91, Ipv4Addr::LOCALHOST, 11)
            .unwrap();
        let db = id.build_database().unwrap();
        let mut owner = if full {
            let mut config = id.server_config();
            config.registered_network_port = Some(port());
            Owner::Server(Box::new(
                bacnet_server::server::BACnetServer::start(
                    config,
                    db,
                    BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
                )
                .await
                .unwrap(),
            ))
        } else {
            let mut endpoint =
                crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
                    .identity(id)
                    .database(db)
                    .registered_network_port(port())
                    .build_session()
                    .unwrap();
            endpoint.start().await.unwrap();
            Owner::Endpoint(Box::new(endpoint))
        };
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = owner.address();
        assert_eq!(
            read(&peer, address, wildcard(), P::APDU_LENGTH).await,
            (port(), PropertyValue::Unsigned(1476))
        );
        assert_eq!(
            read(&peer, address, device(), P::MAX_APDU_LENGTH_ACCEPTED)
                .await
                .1,
            PropertyValue::Unsigned(480)
        );
        owner.stop().await;
    }
}
