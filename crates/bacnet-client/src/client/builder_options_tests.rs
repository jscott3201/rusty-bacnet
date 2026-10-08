use super::*;
use bacnet_transport::loopback::LoopbackTransport;
use std::net::Ipv4Addr;

#[tokio::test]
async fn bip_builder_foreign_device_starts_without_waiting_for_bbmd_acceptance() {
    let peer = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mut client = timeout(
        Duration::from_secs(1),
        BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .foreign_device(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::LOCALHOST,
                bbmd_port: peer.local_addr().unwrap().port(),
                ttl: 60,
                renewal_interval: Some(Duration::from_secs(20)),
            })
            .build(),
    )
    .await
    .expect("builder must not wait for a BBMD result")
    .unwrap();
    let mut packet = [0; 64];
    let (len, _) = peer.recv_from(&mut packet).await.unwrap();
    assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 60]);
    let snapshot = client.transport().bvlc_client_snapshot();
    assert!(snapshot.running);
    assert_eq!(snapshot.foreign_registration.accepted, 0);
    assert_eq!(snapshot.foreign_registration.last_ttl, Some(60));
    client.stop().await.unwrap();
    assert!(!client.transport().bvlc_client_snapshot().running);
}

#[tokio::test]
async fn bip_builder_foreign_device_validates_settings_before_io() {
    for (ttl, interval) in [
        (0, None),
        (60, Some(Duration::ZERO)),
        (60, Some(Duration::from_secs(60))),
    ] {
        let result = BACnetClient::bip_builder()
            .interface(Ipv4Addr::new(192, 0, 2, 1))
            .port(0)
            .foreign_device(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::LOCALHOST,
                bbmd_port: 47808,
                ttl,
                renewal_interval: interval,
            })
            .build()
            .await;
        match result {
            Err(Error::Encoding(message)) => assert!(message.contains("renewal interval")),
            Err(other) => panic!("expected local configuration error, got {other}"),
            Ok(mut client) => {
                client.stop().await.unwrap();
                panic!("invalid foreign config accepted");
            }
        }
    }
}

fn assert_apdu_tuning(config: &ClientConfig) {
    assert_eq!(config.apdu_retries, 7);
    assert_eq!(config.max_segments, Some(8));
    assert!(!config.segmented_response_accepted);
    assert_eq!(config.proposed_window_size, 4);
}

#[tokio::test]
async fn generic_builder_sets_apdu_tuning_options() {
    let (transport, _peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .apdu_retries(7)
        .max_segments(Some(8))
        .segmented_response_accepted(false)
        .proposed_window_size(4)
        .build()
        .await
        .unwrap();

    assert_apdu_tuning(&client.config);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn bip_builder_sets_apdu_tuning_options() {
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_retries(7)
        .max_segments(Some(8))
        .segmented_response_accepted(false)
        .proposed_window_size(4)
        .build()
        .await
        .unwrap();

    assert_apdu_tuning(&client.config);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn builder_rejects_invalid_proposed_window_size() {
    let (transport, _peer_transport) = LoopbackTransport::pair(vec![0x10], vec![0x11]);
    let result = BACnetClient::generic_builder()
        .transport(transport)
        .proposed_window_size(0)
        .build()
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn builder_rejects_max_segments_below_protocol_minimum() {
    for invalid in [0, 1] {
        let (transport, _peer_transport) = LoopbackTransport::pair(vec![0x10], vec![0x11]);
        let result = BACnetClient::generic_builder()
            .transport(transport)
            .max_segments(Some(invalid))
            .build()
            .await;

        match result {
            Err(Error::Encoding(message)) => {
                assert!(message.contains("max-segments-accepted"));
            }
            Err(other) => panic!("expected encoding error, got {other}"),
            Ok(mut client) => {
                client.stop().await.unwrap();
                panic!("max_segments={invalid} must be rejected");
            }
        }
    }
}

/// The B/IP builder hands `share_port_by_address` to its transport, which
/// refuses it without an explicit interface (#1538).
#[tokio::test]
async fn bip_builder_shares_the_port_by_address_only_with_an_address() {
    let built = BACnetClient::bip_builder()
        .interface(Ipv4Addr::UNSPECIFIED)
        .port(0xBAC0)
        .share_port_by_address(true)
        .build()
        .await;
    let Err(Error::Transport(refused)) = built else {
        panic!("build must refuse");
    };
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
}
