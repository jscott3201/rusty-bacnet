//! A registered Network Port over a real `BipTransport` in BBMD and FOREIGN
//! modes (#939): startup publishes the mode the transport runs in, and a
//! BBMD's tables are the transport's own, so a change made through
//! `bbmd_state()` shows on the next ReadProperty.

use super::network_port_mode_tests::{host, port_oid, property_list, rp};
use super::*;
use bacnet_encoding::constructed::{decode_bdt_entry_list, decode_fdt_entry_list};
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::bip::{BbmdOptions, ForeignDeviceConfig};
use bacnet_transport::bvll::BvlcResultCode;
use bacnet_types::constructed::BACnetBDTEntry;
use bacnet_types::enums::PropertyIdentifier as P;

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(
            1,
            "selected",
            BipPortConfig {
                ip_address: [127, 0, 0, 1],
                udp_port: 0,
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    db
}

fn registered() -> ServerConfig {
    ServerConfig {
        registered_network_port: Some(port_oid()),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_bbmd_transport_lends_its_own_tables_to_the_registered_port() {
    let mut transport =
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
    transport.enable_bbmd(Vec::new());
    let mut server = BACnetServer::start(registered(), database(), transport)
        .await
        .unwrap();
    let bound = server
        .test_network()
        .transport()
        .bip_port()
        .unwrap()
        .endpoint
        .port();
    assert_ne!(bound, 0);
    assert_eq!(rp(&server, P::BACNET_IP_MODE).await.unwrap(), [0x91, 2]);
    let own = BACnetBDTEntry {
        bbmd_address: host([127, 0, 0, 1], bound),
        broadcast_mask: Some([0xFF; 4]),
    };
    let bdt = |bytes: Vec<u8>| decode_bdt_entry_list(&bytes).unwrap();
    assert_eq!(
        bdt(rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
            .await
            .unwrap()),
        std::slice::from_ref(&own)
    );
    assert_eq!(
        rp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS).await.unwrap(),
        [0x10]
    );
    assert!(
        rp(&server, P::BBMD_FOREIGN_DEVICE_TABLE)
            .await
            .unwrap()
            .is_empty()
    );

    {
        let mut state = server
            .test_network()
            .transport()
            .bbmd_state()
            .unwrap()
            .lock()
            .unwrap();
        state
            .set_bdt(vec![BdtEntry {
                ip: [10, 0, 0, 2],
                port: 0xBAC0,
                broadcast_mask: [0xFF; 4],
            }])
            .unwrap();
        state.set_foreign_device_policy(Some(ForeignDevicePolicy::default()));
        assert_eq!(
            state.register_foreign_device([10, 0, 0, 9], 0xBAC0, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    assert_eq!(
        bdt(rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
            .await
            .unwrap()),
        [
            BACnetBDTEntry {
                bbmd_address: host([10, 0, 0, 2], 0xBAC0),
                broadcast_mask: Some([0xFF; 4]),
            },
            own,
        ]
    );
    assert_eq!(
        rp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS).await.unwrap(),
        [0x11]
    );
    let fdt =
        decode_fdt_entry_list(&rp(&server, P::BBMD_FOREIGN_DEVICE_TABLE).await.unwrap()).unwrap();
    assert_eq!(fdt.len(), 1);
    assert_eq!(fdt[0].time_to_live, 60);
    // Registered just now: TTL plus grace, less any second that has passed.
    assert!(
        (89..=90).contains(&fdt[0].remaining_time_to_live),
        "{fdt:?}"
    );
    let listed = property_list(&server).await;
    for property in [
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        P::BBMD_ACCEPT_FD_REGISTRATIONS,
        P::BBMD_FOREIGN_DEVICE_TABLE,
    ] {
        assert!(listed.contains(&property), "{property} listed");
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn a_foreign_device_transport_publishes_its_bbmd_and_lifetime() {
    let bbmd = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let bbmd_port = bbmd.local_addr().unwrap().port();
    let mut transport =
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port,
        renewal_interval: None,
        ttl: 120,
    });
    let mut server = BACnetServer::start(registered(), database(), transport)
        .await
        .unwrap();
    assert_eq!(rp(&server, P::BACNET_IP_MODE).await.unwrap(), [0x91, 1]);
    let mut expected = vec![0x0E, 0x1C, 127, 0, 0, 1, 0x0F, 0x1A];
    expected.extend_from_slice(&bbmd_port.to_be_bytes());
    assert_eq!(rp(&server, P::FD_BBMD_ADDRESS).await.unwrap(), expected);
    assert_eq!(
        rp(&server, P::FD_SUBSCRIPTION_LIFETIME).await.unwrap(),
        [0x21, 120]
    );
    let listed = property_list(&server).await;
    assert!(listed.contains(&P::FD_BBMD_ADDRESS));
    assert!(!listed.contains(&P::BBMD_BROADCAST_DISTRIBUTION_TABLE));
    server.stop().await.unwrap();
}

/// A transport set up both as a BBMD and as a foreign device has no single
/// mode, so the server refuses to start with a port registered on it, says
/// why, and leaves the port free.
#[tokio::test]
async fn a_bbmd_that_also_registers_as_a_foreign_device_fails_to_start() {
    let mut transport =
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
    transport.enable_bbmd(Vec::new());
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: 47808,
        renewal_interval: None,
        ttl: 120,
    });
    let error = match BACnetServer::start(registered(), database(), transport).await {
        Ok(_) => panic!("a BBMD that is also a foreign device must not register"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("a BBMD that also registers as a foreign device"),
        "{error}"
    );
}

/// The B/IP builder's BBMD mode (#938) starts a BBMD with the options it
/// carries, and the registered port reports it.
#[tokio::test]
async fn the_builders_bbmd_mode_runs_a_bbmd_the_registered_port_reports() {
    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::new(127, 255, 255, 255))
        .database(database())
        .registered_network_port(port_oid())
        .mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: Some(ForeignDevicePolicy::default()),
            ..BbmdOptions::default()
        }))
        .build()
        .await
        .unwrap();
    assert_eq!(rp(&server, P::BACNET_IP_MODE).await.unwrap(), [0x91, 2]);
    assert_eq!(
        rp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS).await.unwrap(),
        [0x11]
    );
    assert_eq!(server.foreign_registration().unwrap().attempts, 0);
    server.stop().await.unwrap();
}

/// The builder's foreign mode registers, reports the BBMD's refusal through
/// the server (#938), and stops reporting once the server stops.
#[tokio::test]
async fn the_builders_foreign_mode_reports_a_refused_registration() {
    let bbmd = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(database())
        .registered_network_port(port_oid())
        .mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port: bbmd.local_addr().unwrap().port(),
            renewal_interval: None,
            ttl: 120,
        }))
        .build()
        .await
        .unwrap();
    assert_eq!(rp(&server, P::BACNET_IP_MODE).await.unwrap(), [0x91, 1]);
    let mut packet = [0; 64];
    let (len, from) = bbmd.recv_from(&mut packet).await.unwrap();
    assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 120]);
    bbmd.send_to(&[0x81, 0, 0, 6, 0, 0x30], from).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while server.foreign_registration().unwrap().refused == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the refusal reached the server's status");
    let registration = server.foreign_registration().unwrap();
    assert!(!registration.registered);
    assert_eq!(
        registration.last_result,
        Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)
    );
    server.stop().await.unwrap();
    assert!(server.foreign_registration().is_err());
}
