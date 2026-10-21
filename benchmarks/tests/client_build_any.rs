//! Configured builders preserve behavior when their transport type is erased.
use bacnet_benchmarks::sc_helpers::*;
use bacnet_client::client::{BACnetClient, ScClientBuilder};
use bacnet_transport::{
    any::AnyTransport,
    bip::{BbmdOptions, BipMode, BipTransport, ForeignDeviceConfig},
    bip6::{Bip6Transport, Bip6Vmac},
    mstp::NoSerial,
    port::TransportPort,
    sc::ScReconnectConfig,
};
use bacnet_types::error::Error;
use std::{
    future::Future,
    net::{Ipv4Addr, Ipv6Addr},
    time::Duration,
};
use tokio::net::{TcpListener, UdpSocket};

#[path = "client_build_any/options.rs"]
mod options;
#[path = "client_build_any/sc_lifecycle.rs"]
mod sc_lifecycle;

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("builder test barrier timed out")
}

fn invalid_sc(builder: ScClientBuilder, case: usize) -> (ScClientBuilder, &'static str) {
    match case {
        0 => (builder.vmac([0; 6]), "unknown VMAC"),
        1 => (builder.vmac([0xff; 6]), "broadcast VMAC"),
        2 => (builder.device_uuid([0; 16]), "device_uuid"),
        3 => (
            builder.reconnect(ScReconnectConfig {
                initial_delay_ms: 0,
                max_delay_ms: 1,
                max_retries: 1,
            }),
            "reconnect",
        ),
        4 => (
            builder.reconnect(ScReconnectConfig {
                initial_delay_ms: 2,
                max_delay_ms: 1,
                max_retries: 0,
            }),
            "reconnect",
        ),
        5 => (builder.cov_channel_capacity(0), "cov-channel-capacity"),
        6 => (builder.event_channel_capacity(0), "event-channel-capacity"),
        7 => (builder.max_segments(Some(1)), "max-segments-accepted"),
        8 => (builder.proposed_window_size(0), "proposed-window-size"),
        9 => (builder.proposed_window_size(128), "proposed-window-size"),
        _ => (
            builder.min_request_interval_ms(3_600_001),
            "min-request-interval",
        ),
    }
}

#[tokio::test]
async fn sc_builders_reject_invalid_options_before_any_tcp_dial() {
    let tls = try_make_node_tls_config(&generate_test_certs()).unwrap();
    for erased in [false, true] {
        for case in 0..11 {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("wss://localhost:{}", listener.local_addr().unwrap().port());
            let builder = BACnetClient::sc_builder()
                .hub_url(&url)
                .tls_config(tls.clone())
                .vmac([2; 6])
                .device_uuid([3; 16]);
            let (builder, expected) = invalid_sc(builder, case);
            let error = bounded(async {
                tokio::select! {
                    result = async {
                        if erased {
                            Box::pin(builder.build_any::<NoSerial>()).await.map(|_| ())
                        } else {
                            Box::pin(builder.build()).await.map(|_| ())
                        }
                    } => result.expect_err("invalid builder must fail"),
                    accepted = listener.accept() => panic!("invalid case {case} reached TCP: {accepted:?}"),
                }
            }).await;
            assert!(
                error.to_string().contains(expected),
                "case {case}: {error:?}"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(10), listener.accept())
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn bip_builders_preserve_foreign_mode_and_registration_status() {
    for erased in [false, true] {
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let builder = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .mode(BipMode::Foreign(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::LOCALHOST,
                bbmd_port: peer.local_addr().unwrap().port(),
                ttl: 75,
                renewal_interval: Some(Duration::from_secs(15)),
            }));
        if erased {
            let client = bounded(Box::pin(builder.build_any::<NoSerial>()))
                .await
                .unwrap();
            foreign_registration(client, &peer).await;
        } else {
            let client = bounded(Box::pin(builder.build())).await.unwrap();
            foreign_registration(client, &peer).await;
        }
    }
}

async fn foreign_registration<T: TransportPort + bacnet_transport::bip::AsBip + 'static>(
    mut client: BACnetClient<T>,
    peer: &UdpSocket,
) {
    assert!(!client.foreign_registration().unwrap().registered);
    let mut buf = [0; 64];
    let (n, from) = bounded(peer.recv_from(&mut buf)).await.unwrap();
    assert_eq!(&buf[..n], &[0x81, 5, 0, 6, 0, 75]);
    peer.send_to(&[0x81, 0, 0, 6, 0, 0], from).await.unwrap();
    bounded(async {
        while !client.foreign_registration().unwrap().registered {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        client
            .foreign_registration()
            .unwrap()
            .next_attempt_in
            .unwrap()
            <= Duration::from_secs(15)
    );
    client.stop().await.unwrap();
    assert!(
        !client
            .transport()
            .as_bip()
            .unwrap()
            .bvlc_client_snapshot()
            .running
    );
}

#[tokio::test]
async fn bip_build_any_keeps_bbmd_policy_and_shared_address_validation() {
    let policy = bacnet_transport::bbmd::ForeignDevicePolicy {
        max_ttl: 123,
        ..Default::default()
    };
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: Some(policy.clone()),
            management_acl: vec![[127, 0, 0, 9]],
            ..Default::default()
        }))
        .build_any::<NoSerial>()
        .await
        .unwrap();
    let AnyTransport::Bip(transport) = client.transport() else {
        panic!("expected B/IP");
    };
    {
        let state = transport.bbmd_state().unwrap().lock().unwrap();
        assert_eq!(state.foreign_device_policy(), Some(&policy));
        assert!(state.is_management_allowed(&[127, 0, 0, 9]));
        assert!(!state.is_management_allowed(&[127, 0, 0, 1]));
    }
    client.stop().await.unwrap();
    for (interface, port) in [(Ipv4Addr::UNSPECIFIED, 47808), (Ipv4Addr::LOCALHOST, 0)] {
        let error = BACnetClient::bip_builder()
            .interface(interface)
            .port(port)
            .share_port_by_address(true)
            .build_any::<NoSerial>()
            .await
            .err()
            .unwrap();
        assert!(
            matches!(error, Error::Transport(ref e) if e.kind() == std::io::ErrorKind::InvalidInput)
        );
    }
}

#[tokio::test]
async fn bip_builders_preserve_shared_address_and_broadcast_endpoint() {
    for erased in [false, true] {
        let reserved = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let builder = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(port)
            .broadcast_address(Ipv4Addr::new(127, 255, 255, 255))
            .share_port_by_address(true);
        if erased {
            let client = builder.build_any::<NoSerial>().await.unwrap();
            shared_address(client, port).await;
        } else {
            let client = builder.build().await.unwrap();
            shared_address(client, port).await;
        }
    }
}

async fn shared_address<T: TransportPort + 'static>(mut client: BACnetClient<T>, port: u16) {
    assert_eq!(
        client.transport().bip_broadcast_endpoint(),
        Some(std::net::SocketAddrV4::new(
            Ipv4Addr::new(127, 255, 255, 255),
            port
        ))
    );
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let peer_port = peer.local_addr().unwrap().port();
    let mut peer_mac = vec![127, 0, 0, 1];
    peer_mac.extend_from_slice(&peer_port.to_be_bytes());
    client
        .transport()
        .send_unicast(&[1, 0, 0x10, 8], &peer_mac)
        .await
        .unwrap();
    let mut buf = [0; 64];
    let (n, from) = bounded(peer.recv_from(&mut buf)).await.unwrap();
    assert_eq!(
        from,
        std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, port))
    );
    assert_eq!(&buf[..n], &[0x81, 10, 0, 8, 1, 0, 0x10, 8]);
    client.stop().await.unwrap();
}

#[tokio::test]
#[allow(clippy::print_stderr)] // Report the observed startup capability, not a blanket success.
async fn bip6_builders_preserve_explicit_and_random_identity_and_interface() {
    for instance in [Some(0x123456), None] {
        let mut outcomes = Vec::new();
        for erased in [false, true] {
            let mut builder = BACnetClient::bip6_builder()
                .interface(Ipv6Addr::LOCALHOST)
                .port(0);
            if let Some(instance) = instance {
                builder = builder.device_instance(instance);
            }
            let outcome = if erased {
                match builder.build_any::<NoSerial>().await {
                    Ok(mut client) => {
                        let AnyTransport::Bip6(transport) = client.transport() else {
                            panic!("expected B/IPv6");
                        };
                        bip6_identity(transport, instance).await;
                        client.stop().await.unwrap();
                        Bip6Startup::WireIdentityVerified
                    }
                    Err(error) => bip6_start_failure(error, instance),
                }
            } else {
                match builder.build().await {
                    Ok(mut client) => {
                        bip6_identity(client.transport(), instance).await;
                        client.stop().await.unwrap();
                        Bip6Startup::WireIdentityVerified
                    }
                    Err(error) => bip6_start_failure(error, instance),
                }
            };
            outcomes.push(outcome);
        }
        assert_eq!(outcomes[0], outcomes[1], "identity {instance:?}");
        eprintln!("B/IPv6 identity {instance:?}, concrete then erased: {outcomes:?}");
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Bip6Startup {
    WireIdentityVerified,
    #[cfg(target_os = "linux")]
    UnsupportedMulticastRoute,
}

fn bip6_start_failure(error: Error, instance: Option<u32>) -> Bip6Startup {
    // Random identity requires a successful multicast collision probe. Linux
    // loopback can reject that send with ENETUNREACH; preserve the exact
    // failure on both builder paths instead of claiming startup/wire coverage.
    #[cfg(target_os = "linux")]
    if instance.is_none()
        && matches!(&error, Error::Transport(error)
            if error.kind() == std::io::ErrorKind::NetworkUnreachable
                && error.raw_os_error() == Some(101))
    {
        return Bip6Startup::UnsupportedMulticastRoute;
    }
    panic!("unexpected B/IPv6 startup failure for identity {instance:?}: {error:?}");
}

async fn bip6_identity(transport: &Bip6Transport, instance: Option<u32>) {
    let vmac = transport.local_vmac().unwrap();
    if instance.is_some() {
        assert_eq!(vmac, Bip6Vmac::new([0x12, 0x34, 0x56]));
    } else {
        assert_eq!(vmac.octets()[0] >> 6, 1, "random VMAC range");
    }
    let address = transport.local_address().unwrap();
    assert_eq!(*address.ip(), Ipv6Addr::LOCALHOST);
    assert_ne!(address.port(), 0);

    // A raw, directed VAR avoids any multicast-route requirement after start.
    // The ACK independently exposes the identity and actual bound source.
    let peer = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let request = [0x82, 6, 0, 7, 0, 0, 1];
    assert_eq!(
        peer.send_to(&request, address).await.unwrap(),
        request.len()
    );
    let mut response = [0; 64];
    let (length, source) = bounded(peer.recv_from(&mut response)).await.unwrap();
    assert_eq!(source, std::net::SocketAddr::V6(address));
    let [a, b, c] = vmac.octets();
    assert_eq!(&response[..length], &[0x82, 7, 0, 10, a, b, c, 0, 0, 1]);
}
