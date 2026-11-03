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
                bbmd: (Ipv4Addr::LOCALHOST, peer.local_addr().unwrap().port()).into(),
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
                bbmd: (Ipv4Addr::LOCALHOST, 47808).into(),
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
            bbmd: (Ipv4Addr::LOCALHOST, bbmd.local_addr().unwrap().port()).into(),
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

/// A resolver that answers every name with one address.
struct OneAddress(Ipv4Addr);

impl bacnet_transport::bbmd_lookup::BbmdResolver for OneAddress {
    fn resolve(&self, _host: &str) -> bacnet_transport::bbmd_lookup::BbmdLookup {
        let ip = std::net::IpAddr::V4(self.0);
        Box::pin(async move { Ok(vec![ip]) })
    }
}

/// The builder hands its transport the BBMD resolver and the release of
/// the BBMD a move leaves (#2307): a host name activated on the port moves
/// the client to the BBMD the resolver names, which it registers with, and
/// the BBMD it left is asked to delete its entry.
#[tokio::test]
async fn bip_builder_hands_the_transport_its_resolver_and_release() {
    use bacnet_types::bip_port::{BipPortMode, ForeignDeviceChanges};
    use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
    let socket = || tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0));
    let (old, new) = (socket().await.unwrap(), socket().await.unwrap());
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .mode(BipMode::Foreign(ForeignDeviceConfig {
            bbmd: (Ipv4Addr::LOCALHOST, old.local_addr().unwrap().port()).into(),
            ttl: 60,
            renewal_interval: None,
        }))
        .release_previous_bbmd(true)
        .bbmd_resolver(Arc::new(OneAddress(Ipv4Addr::LOCALHOST)))
        .build()
        .await
        .unwrap();
    let mut packet = [0; 64];
    let (len, _) = old.recv_from(&mut packet).await.unwrap();
    assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 60]);
    let Some(BipPortMode::Foreign { registration }) =
        client.transport().bip_port().map(|port| port.mode)
    else {
        panic!("a foreign device's port");
    };
    let mut changes = ForeignDeviceChanges::default();
    changes.bbmd = Some(BACnetHostNPort {
        host: BACnetHostAddress::Name("bbmd.test".into()),
        port: new.local_addr().unwrap().port(),
    });
    registration.activate(&changes).unwrap();
    let wait = Duration::from_secs(5);
    let (len, _) = timeout(wait, new.recv_from(&mut packet))
        .await
        .expect("a registration with the new BBMD")
        .unwrap();
    assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 60]);
    let own = client.transport().local_mac().to_vec();
    let (len, _) = timeout(wait, old.recv_from(&mut packet))
        .await
        .expect("a release of the old BBMD")
        .unwrap();
    assert_eq!(&packet[..4], &[0x81, 0x08, 0, 10]);
    assert_eq!(&packet[4..len], own);
    let registration = client.foreign_registration().unwrap();
    assert_eq!(
        (registration.previous_releases, registration.bbmd_lookups),
        (1, 1)
    );
    client.stop().await.unwrap();
}

/// A resolver that answers every name with the IPv6 loopback address.
#[cfg(feature = "ipv6")]
struct Loopback6;

#[cfg(feature = "ipv6")]
impl bacnet_transport::bbmd_lookup::BbmdResolver for Loopback6 {
    fn resolve(&self, _host: &str) -> bacnet_transport::bbmd_lookup::BbmdLookup {
        Box::pin(async { Ok(vec![std::net::IpAddr::V6(Ipv6Addr::LOCALHOST)]) })
    }
}

/// The B/IPv6 builder hands its transport the BBMD resolver and the
/// release of the BBMD a move leaves (#2338), as the B/IP one does: a host
/// name activated on the port moves the client to the BBMD the resolver
/// names, which it registers with, and the BBMD it left is asked to delete
/// the entry of the client's own B/IPv6 address.
#[cfg(feature = "ipv6")]
#[tokio::test]
async fn bip6_builder_hands_the_transport_its_resolver_and_release() {
    use bacnet_transport::bip6::Bip6ForeignDeviceConfig;
    use bacnet_types::bip_port::{BipPortMode, ForeignDeviceChanges};
    use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
    let socket = || tokio::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0));
    let (old, new) = (socket().await.unwrap(), socket().await.unwrap());
    let mut client = BACnetClient::bip6_builder()
        .interface(Ipv6Addr::LOCALHOST)
        .port(0)
        .device_instance(0x12_3456)
        .foreign_device(Bip6ForeignDeviceConfig {
            bbmd_ip: Ipv6Addr::LOCALHOST,
            bbmd_port: old.local_addr().unwrap().port(),
            ttl: 90,
        })
        .release_previous_bbmd(true)
        .bbmd_resolver(Arc::new(Loopback6))
        .build()
        .await
        .unwrap();
    // Register-Foreign-Device from the instance's VMAC, for 90 s.
    let registration = [0x82, 0x09, 0, 9, 0x12, 0x34, 0x56, 0, 90];
    let wait = Duration::from_secs(5);
    let mut packet = [0; 64];
    let (len, _) = timeout(wait, old.recv_from(&mut packet))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&packet[..len], registration);
    let Some(BipPortMode::Foreign { registration: lent }) =
        client.transport().bip6_port().map(|port| port.mode)
    else {
        panic!("a foreign device's port");
    };
    let mut changes = ForeignDeviceChanges::default();
    changes.bbmd = Some(BACnetHostNPort {
        host: BACnetHostAddress::Name("bbmd.test".into()),
        port: new.local_addr().unwrap().port(),
    });
    lent.activate(&changes).unwrap();
    let (len, _) = timeout(wait, new.recv_from(&mut packet))
        .await
        .expect("a registration with the new BBMD")
        .unwrap();
    assert_eq!(&packet[..len], registration);
    // Delete-Foreign-Device-Table-Entry: the VMAC, then the entry, the
    // client's own address and port (U.2.11).
    let (len, _) = timeout(wait, old.recv_from(&mut packet))
        .await
        .expect("a release of the old BBMD")
        .unwrap();
    assert_eq!(&packet[..7], [0x82, 0x0A, 0, 25, 0x12, 0x34, 0x56]);
    let own = client.transport().local_address().unwrap();
    assert_eq!(&packet[7..23], own.ip().octets());
    assert_eq!(&packet[23..len], own.port().to_be_bytes());
    let transport = client.transport();
    assert_eq!(
        (
            transport.previous_bbmd_releases().releases,
            transport.bbmd_lookup_counters().lookups
        ),
        (1, 1)
    );
    client.stop().await.unwrap();
}
