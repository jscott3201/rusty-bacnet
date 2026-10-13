//! The reservation follows the final network owner even when stop cannot unwrap it.
use super::*;
use bacnet_objects::network_port::{BipPortConfig, NetworkPortObject};

#[path = "network_port_bip_mode_tests.rs"]
mod network_port_bip_mode_tests;
#[path = "network_port_mode_tests.rs"]
mod network_port_mode_tests;
#[path = "sc_network_number_tests.rs"]
mod sc_network_number_tests;

#[tokio::test]
async fn registered_port_bare_drop_and_retained_network_refusal_keep_lease() {
    for stop_first in [false, true] {
        let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
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
        let config = ServerConfig {
            registered_network_port: Some(oid),
            ..Default::default()
        };
        let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut server = BACnetServer::start(config, db, transport).await.unwrap();
        let db = server.database().clone();
        let network = server.test_network().clone();
        let address = network.transport().bip_port().unwrap().endpoint;
        if stop_first {
            assert!(server.stop().await.is_err());
        }
        drop(server);
        assert!(db.write().await.remove(&oid).is_err());
        assert!(std::net::UdpSocket::bind(address).is_err());
        drop(network);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match db.write().await.remove(&oid) {
                    Ok(Some(_)) => break,
                    Err(_) => tokio::task::yield_now().await,
                    other => panic!("unexpected removal result: {}", other.is_ok()),
                }
            }
        })
        .await
        .unwrap();
        // Lease cannot disappear before the socket's final destruction.
        let _reused = std::net::UdpSocket::bind(address).unwrap();
    }
}

#[tokio::test]
async fn network_number_server_held_learning_preserves_audit_ack_progress() {
    use bacnet_endpoint_core::coordinator::CanonicalPeer;
    let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
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
    let mut server = BACnetServer::start(
        ServerConfig {
            registered_network_port: Some(oid),
            ..Default::default()
        },
        db,
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255)),
    )
    .await
    .unwrap();
    let address = server
        .test_network()
        .transport()
        .bip_port()
        .unwrap()
        .endpoint;
    let peer = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mac =
        bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], peer.local_addr().unwrap().port());
    let (operation, acknowledged) = server
        .notification_transactions
        .reserve(
            CanonicalPeer::direct(&mac),
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        )
        .unwrap();
    let db = server.database().clone();
    let guard = db.read().await;
    peer.send_to(
        &[
            0x81, 4, 0, 16, 10, 1, 1, 8, 0xba, 0xc0, 1, 0x80, 0x13, 0, 19, 1,
        ],
        address,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while db.try_read().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    peer.send_to(
        &[
            0x81,
            10,
            0,
            9,
            1,
            0,
            0x20,
            operation.invoke_id(),
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION.to_raw(),
        ],
        address,
    )
    .await
    .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), acknowledged)
            .await
            .unwrap()
            .unwrap(),
        CovAckResult::Ack
    ));
    drop(operation);
    // Stop must abort the blocked writer without waiting for this DB reader.
    tokio::time::timeout(Duration::from_secs(1), server.stop())
        .await
        .unwrap()
        .unwrap();
    drop(guard);
    assert!(db.write().await.remove(&oid).unwrap().is_some());
}
