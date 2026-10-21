use super::*;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BbmdOptions, ForeignDeviceConfig, ForeignRegistrationOutcome};
use bacnet_transport::bvll::BvlcResultCode;
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
            .mode(BipMode::Foreign(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::LOCALHOST,
                bbmd_port: peer.local_addr().unwrap().port(),
                ttl: 60,
                renewal_interval: Some(Duration::from_secs(20)),
            }))
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
            .mode(BipMode::Foreign(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::LOCALHOST,
                bbmd_port: 47808,
                ttl,
                renewal_interval: interval,
            }))
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

/// A foreign client whose BBMD, a bare socket, answers every registration
/// with `code`, returned once the client has seen that answer. An attempt
/// the answer comes too late for is retried and answered, so a slow runner
/// only delays it.
async fn foreign_client_answered_with(
    code: BvlcResultCode,
    renewal_interval: Option<Duration>,
) -> BACnetClient<BipTransport> {
    let bbmd = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port: bbmd.local_addr().unwrap().port(),
            ttl: 60,
            renewal_interval,
        }))
        .build()
        .await
        .unwrap();
    let [high, low] = code.to_raw().to_be_bytes();
    tokio::spawn(async move {
        let mut packet = [0; 64];
        while let Ok((len, from)) = bbmd.recv_from(&mut packet).await {
            if packet[..len] == [0x81, 5, 0, 6, 0, 60] {
                let _ = bbmd.send_to(&[0x81, 0, 0, 6, high, low], from).await;
            }
        }
    });
    timeout(Duration::from_secs(20), async {
        while client.foreign_registration().unwrap().last_result != Some(code) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the client saw the BBMD's answer");
    client
}

/// The client reports its registration (#938): the lease a success proves,
/// renewed on the interval the builder was given rather than half the TTL.
#[tokio::test]
async fn bip_builder_foreign_mode_reports_the_lease_and_renews_on_its_interval() {
    let mut client = foreign_client_answered_with(
        BvlcResultCode::SUCCESSFUL_COMPLETION,
        Some(Duration::from_secs(10)),
    )
    .await;
    let registration = client.foreign_registration().unwrap();
    assert!(registration.registered);
    assert_eq!(
        registration.last_outcome,
        Some(ForeignRegistrationOutcome::Accepted)
    );
    assert!(registration.lease_remaining.unwrap() > Duration::from_secs(50));
    // Half the 60 s TTL would put it 30 s out.
    assert!(registration.next_attempt_in.unwrap() <= Duration::from_secs(10));
    client.stop().await.unwrap();
}

/// A BBMD's refusal reaches the client's registration status (#938).
#[tokio::test]
async fn bip_builder_foreign_mode_reports_a_refused_registration() {
    let mut client =
        foreign_client_answered_with(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK, None).await;
    let registration = client.foreign_registration().unwrap();
    assert!(!registration.registered);
    assert_eq!(registration.refused, 1);
    assert_eq!(
        registration.last_result,
        Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)
    );
    assert_eq!(
        registration.last_outcome,
        Some(ForeignRegistrationOutcome::Refused)
    );
    client.stop().await.unwrap();
}

/// BBMD mode starts a BBMD with every option the mode carries (#938).
#[tokio::test]
async fn bip_builder_bbmd_mode_starts_a_bbmd_with_its_options() {
    let policy = ForeignDevicePolicy {
        max_ttl: 120,
        ..ForeignDevicePolicy::default()
    };
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: Some(policy.clone()),
            management_acl: vec![[127, 0, 0, 9]],
            ..BbmdOptions::default()
        }))
        .build()
        .await
        .unwrap();
    {
        let state = client.transport().bbmd_state().unwrap().lock().unwrap();
        assert_eq!(state.foreign_device_policy(), Some(&policy));
        assert!(state.is_management_allowed(&[127, 0, 0, 9]));
        assert!(!state.is_management_allowed(&[127, 0, 0, 1]));
    }
    // Not a foreign device: no registration runs.
    let registration = client.foreign_registration().unwrap();
    assert_eq!((registration.attempts, registration.registered), (0, false));
    client.stop().await.unwrap();
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
            Err(Error::Range(refused)) => {
                assert_eq!(
                    refused.to_string(),
                    format!(
                        "value out of range: invalid max-segments-accepted {invalid}; \
                         expected 2..=255 or unspecified"
                    )
                );
            }
            Err(other) => panic!("expected a range error, got {other}"),
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

/// The B/IP builder hands its multicast group to the transport, which
/// refuses it without an explicit interface (#941).
#[tokio::test]
async fn bip_builder_joins_a_multicast_group_only_with_an_interface() {
    let group_port = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let group = bacnet_transport::bip::BipMulticast::new(Ipv4Addr::new(239, 255, 96, 1))
        .unwrap()
        .with_port(group_port)
        .unwrap();
    let Err(refused) = BACnetClient::bip_builder()
        .port(0)
        .multicast(Some(group))
        .build()
        .await
    else {
        panic!("build must refuse a wildcard interface");
    };
    assert!(
        refused.to_string().contains("explicit interface"),
        "{refused}"
    );
    // The transport at the group's port, as Windows requires.
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(group_port)
        .multicast(Some(group))
        .build()
        .await
        .unwrap();
    assert_eq!(client.transport().multicast(), Some(group));
    client.stop().await.unwrap();
}
