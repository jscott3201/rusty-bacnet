use std::net::{Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use bacnet_types::error::Error;
use tokio::net::UdpSocket;

use super::vmac_table::derive_vmac_from_device_instance;
use super::{
    encode_address_resolution_ack, generate_random_vmac, Bip6ForeignDeviceConfig, Bip6Transport,
    Bip6Vmac, MAX_VMAC_RETRIES,
};
use crate::port::TransportPort;

#[test]
fn generate_random_vmac_produces_3_bytes() {
    let vmac = generate_random_vmac().unwrap();
    assert_eq!(vmac.len(), 3);
    assert_eq!(vmac[0] & 0xC0, 0x40);
}

#[test]
fn generate_random_vmac_is_nondeterministic() {
    let vmacs: Vec<Bip6Vmac> = (0..10).map(|_| generate_random_vmac().unwrap()).collect();
    let all_same = vmacs.windows(2).all(|window| window[0] == window[1]);
    assert!(!all_same, "10 random VMACs should not all be identical");
}

#[tokio::test]
async fn out_of_range_device_instance_fails_before_bip6_startup() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(0x40_0000));
    assert!(transport.start().await.is_err());
}

#[tokio::test]
async fn random_vmac_foreign_device_fails_before_bip6_startup() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: Ipv6Addr::LOCALHOST,
        bbmd_port: 47_808,
        ttl: 60,
    });

    let error = transport.start().await.unwrap_err();
    assert!(
        matches!(error, Error::Transport(error) if error.kind() == std::io::ErrorKind::InvalidInput)
    );
    assert!(transport.socket.is_none());
}

#[tokio::test]
async fn peer_probe_collision_fails_startup_without_publishing_socket() {
    let reservation = UdpSocket::bind("[::1]:0").await.unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let device_instance = 0x12_3456;
    let vmac = derive_vmac_from_device_instance(device_instance);
    let destination = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);
    let peer = UdpSocket::bind("[::1]:0").await.unwrap();
    let peer_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let probe = encode_address_resolution_ack(&vmac, &vmac);
        for _ in 0..20 {
            let _ = peer.send_to(&probe, destination).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, port, Some(device_instance));
    let error = transport.start().await.unwrap_err();
    peer_task.abort();
    let _ = peer_task.await;

    assert!(
        matches!(error, Error::Transport(error) if error.kind() == std::io::ErrorKind::AddrInUse)
    );
    assert!(transport.socket.is_none());
    let send_error = transport.send_broadcast(&[0x01, 0x00]).await.unwrap_err();
    assert!(
        matches!(send_error, Error::Transport(error) if error.kind() == std::io::ErrorKind::NotConnected)
    );
}

#[test]
fn max_vmac_retries_constant() {
    const { assert!(MAX_VMAC_RETRIES >= 1, "must allow at least one retry") };
}
