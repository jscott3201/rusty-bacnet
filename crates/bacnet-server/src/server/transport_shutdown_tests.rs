use super::*;
use crate::server::port_retry::{bind_eventually, rerun_on_lost_port};
use bacnet_objects::device::{DeviceConfig, DeviceObject};

async fn bip_server() -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig::default()).unwrap(),
    ))
    .unwrap();
    BACnetServer::start(
        ServerConfig::default(),
        db,
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn retained_bip_broadcaster_refuses_after_stop() {
    let mut server = bip_server().await;
    let broadcaster = server.i_am_broadcaster();
    let retained = broadcaster.clone();
    broadcaster.broadcast_i_am().await.unwrap();
    server.stop().await.unwrap();
    assert!(
        retained.broadcast_i_am().await.is_err(),
        "stopped server still sends"
    );
    assert!(server.broadcast_i_am().await.is_err());
    server.stop().await.unwrap();
}

#[tokio::test]
async fn stopped_bip_server_releases_udp_port_with_retained_broadcaster() {
    rerun_on_lost_port(async || {
        let mut server = bip_server().await;
        let mac = server.local_mac();
        let port = u16::from_be_bytes([mac[4], mac[5]]);
        let retained = server.i_am_broadcaster();
        server.stop().await.unwrap();
        // A successful stop releases the server's UDP socket.
        let rebound = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))?;
        assert!(retained.broadcast_i_am().await.is_err());
        drop(rebound);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn dropped_bip_server_retires_retained_broadcaster() {
    rerun_on_lost_port(async || {
        let server = bip_server().await;
        let mac = server.local_mac();
        let port = u16::from_be_bytes([mac[4], mac[5]]);
        let retained = server.i_am_broadcaster();
        drop(server);
        assert!(
            retained.broadcast_i_am().await.is_err(),
            "dropped server still sends"
        );
        // The aborted owned frames finally release the UDP port.
        bind_eventually(Duration::from_secs(2), || {
            std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))
        })
        .await
        .map(drop)
    })
    .await;
}
