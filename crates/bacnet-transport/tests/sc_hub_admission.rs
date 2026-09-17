//! External-consumer acceptance of RB-12 constrained hub admission:
//! validated limits, admin deny, split caps, and the bounded status.
//! Real mutual-TLS peers against public startup APIs only.
#![cfg(feature = "sc-tls")]

mod hub_tls_support;

use std::{panic::AssertUnwindSafe, time::Duration};

use bacnet_transport::sc_hub::{
    ScHub, ScHubAdmissionDecision, ScHubAdmissionLimits, ScHubHandshakeTimeouts, ScHubTlsConfig,
};
use futures_util::{FutureExt, SinkExt, StreamExt};
use hub_tls_support::*;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

fn typed_config(f: &Fixture) -> ScHubTlsConfig {
    ScHubTlsConfig::from_der(
        vec![f.ca.clone()],
        vec![f.server.cert.clone(), f.ca.clone()],
        f.server.key.clone_key(),
    )
    .unwrap()
}

async fn start_api(
    api: u8,
    address: &str,
    config: ScHubTlsConfig,
    vmac: [u8; 6],
    uuid: [u8; 16],
) -> Result<ScHub, bacnet_types::error::Error> {
    match api {
        0 => ScHub::start(address, config, vmac, uuid).await,
        1 => ScHub::start_with_uuid(address, config, vmac, uuid).await,
        2 => {
            ScHub::start_with_uuid_and_timeouts(
                address,
                config,
                vmac,
                uuid,
                ScHubHandshakeTimeouts::default(),
            )
            .await
        }
        3 => {
            ScHub::start_with_tls_config(
                address,
                config,
                vmac,
                uuid,
                ScHubHandshakeTimeouts::default(),
            )
            .await
        }
        _ => unreachable!(),
    }
}

/// Fixed-shape Connect-Request with explicit claims (peer VMAC/UUID are
/// independent of the hub identity asserted in the Accept).
async fn send_claim(peer: &mut Peer, id: u8, vmac: [u8; 6], uuid: [u8; 16]) {
    let mut request = vec![6, 0, 0x22, id];
    request.extend_from_slice(&vmac);
    request.extend_from_slice(&uuid);
    request.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
    bounded(peer.send(Message::Binary(request.into())))
        .await
        .unwrap();
}

async fn expect_accept(peer: &mut Peer, id: u8) {
    let Message::Binary(accepted) = bounded(peer.next()).await.unwrap().unwrap() else {
        panic!("expected SC Connect-Accept");
    };
    assert_eq!(&accepted[..4], &[7, 0, 0x22, id]);
    assert_eq!(&accepted[4..10], &HUB_VMAC);
    assert_eq!(&accepted[10..26], &HUB_UUID);
    assert_eq!(accepted.len(), 30);
}

/// Admin-deny and at-capacity share the RESOURCES/OTHER NAK family.
fn expect_resources_other_nak(wire: &[u8], id: u8) {
    assert_eq!(wire, &[0, 0, 0x22, id, 6, 1, 0, 0, 3, 0, 0]);
}

async fn read_binary(peer: &mut Peer) -> Vec<u8> {
    let Message::Binary(wire) = bounded(peer.next()).await.unwrap().unwrap() else {
        panic!("expected SC binary frame");
    };
    wire.to_vec()
}

#[test]
fn checked_limits_reject_zero_and_overflow() {
    for (clients, handshakes) in [(0, 1), (1, 0), (0, 0)] {
        let error = ScHubAdmissionLimits::new(clients, handshakes).unwrap_err();
        assert!(
            matches!(error, bacnet_types::error::Error::Encoding(ref message) if message.contains("admission")),
            "{error}"
        );
    }
    assert!(ScHubAdmissionLimits::new(usize::MAX, 1).is_err());
    assert!(ScHubAdmissionLimits::new(256, 256).is_ok());
    assert_eq!(ScHubAdmissionLimits::default().total_active(), 512);
}

#[tokio::test]
async fn admission_limits_rejected_before_bind_on_every_start_api() {
    let fixture = Fixture::new();
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap().to_string();
    let invalid = [
        ScHubAdmissionLimits {
            max_clients: 0,
            max_handshakes: 256,
        },
        ScHubAdmissionLimits {
            max_clients: 256,
            max_handshakes: 0,
        },
        ScHubAdmissionLimits {
            max_clients: usize::MAX,
            max_handshakes: 1,
        },
    ];
    for api in 0..4 {
        for limits in invalid {
            // The address is occupied: a bind-first implementation would
            // report a bind failure. The limits error proves validation
            // runs before binding on every startup API.
            let error = bounded(start_api(
                api,
                &address,
                typed_config(&fixture).with_admission_limits(limits),
                HUB_VMAC,
                HUB_UUID,
            ))
            .await
            .err()
            .expect("invalid admission limits must fail");
            assert!(
                matches!(error, bacnet_types::error::Error::Encoding(ref text) if text.contains("admission")),
                "API {api}, limits {limits:?}: expected admission error, got {error}"
            );
        }
    }
}

#[tokio::test]
async fn cloned_config_shares_policy_but_not_deny_counters() {
    let f = Fixture::new();
    let config = typed_config(&f).with_admission_policy(|_| ScHubAdmissionDecision::Deny);
    let mut hub_a = bounded(ScHub::start(
        "127.0.0.1:0",
        config.clone(),
        HUB_VMAC,
        HUB_UUID,
    ))
    .await
    .unwrap();
    let mut hub_b = bounded(ScHub::start("127.0.0.1:0", config, HUB_VMAC, HUB_UUID))
        .await
        .unwrap();
    let outcome = AssertUnwindSafe(async {
        for (address, id) in [
            (hub_a.local_addr().unwrap(), 1),
            (hub_b.local_addr().unwrap(), 2),
        ] {
            let mut peer =
                websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
            send_claim(&mut peer, id, [id; 6], [id; 16]).await;
            expect_resources_other_nak(&read_binary(&mut peer).await, id);
        }
        assert_eq!(hub_a.status().await.admin_denied, 1);
        assert_eq!(hub_b.status().await.admin_denied, 1);
        assert_eq!(hub_a.status().await.client_count, 0);
        assert_eq!(hub_b.status().await.client_count, 0);
    })
    .catch_unwind()
    .await;
    finish(hub_a, outcome).await;
    bounded(hub_b.stop()).await;
}

#[tokio::test]
async fn handshake_cap_rejects_before_client_cap_is_reached() {
    let f = Fixture::new();
    let config = typed_config(&f).with_admission_limits(ScHubAdmissionLimits {
        max_clients: 4,
        max_handshakes: 2,
    });
    let hub = bounded(ScHub::start("127.0.0.1:0", config, HUB_VMAC, HUB_UUID))
        .await
        .unwrap();
    let outcome = AssertUnwindSafe(async {
        let address = hub.local_addr().unwrap();
        let mut registered =
            websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        connect(&mut registered, 1, HUB_UUID).await;
        // Two idle handshakes fill the handshake bound while the total
        // (1 + 2 = 3 of 6) and client (1 of 4) bounds still have room.
        let _idle_b = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        let _idle_c = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        let status = hub.status().await;
        assert!(status.listening);
        assert_eq!(status.client_count, 1);
        assert_eq!(status.handshake_count, 2);
        assert_eq!(status.admin_denied, 0);
        let mut refused = bounded(TcpStream::connect(address)).await.unwrap();
        assert!(matches!(
            bounded(tokio::io::AsyncReadExt::read(&mut refused, &mut [0; 1])).await,
            Ok(0) | Err(_)
        ));
        let settled = hub.status().await;
        assert_eq!((settled.client_count, settled.handshake_count), (1, 2));
    })
    .catch_unwind()
    .await;
    finish(hub, outcome).await;
}

#[tokio::test]
async fn client_cap_naks_but_same_uuid_replacement_still_wins() {
    let f = Fixture::new();
    let config = typed_config(&f).with_admission_limits(ScHubAdmissionLimits {
        max_clients: 1,
        max_handshakes: 8,
    });
    let hub = bounded(ScHub::start("127.0.0.1:0", config, HUB_VMAC, HUB_UUID))
        .await
        .unwrap();
    let outcome = AssertUnwindSafe(async {
        let address = hub.local_addr().unwrap();
        let mut first = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        connect(&mut first, 1, HUB_UUID).await;
        // New UUID at capacity: RESOURCES/OTHER, no deny count.
        let mut full = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        send_claim(&mut full, 2, [2; 6], [0x22; 16]).await;
        expect_resources_other_nak(&read_binary(&mut full).await, 2);
        assert_eq!(hub.status().await.admin_denied, 0);
        // Same UUID as the incumbent ([1; 16] is the claim behind id 1):
        // AB.6.2.3 replacement wins over capacity.
        let mut replacement =
            websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        send_claim(&mut replacement, 3, [3; 6], [1; 16]).await;
        expect_accept(&mut replacement, 3).await;
        assert!(matches!(
            bounded(first.next()).await.unwrap().unwrap(),
            Message::Close(_)
        ));
        // The superseded slot is released; exactly one client remains.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let status = hub.status().await;
            if status.client_count == 1 && status.handshake_count == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "stale snapshot: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Redaction tripwire: the connected VMAC must not appear in the
        // snapshot Debug (counts and kind labels only).
        let debug = format!("{:?}", hub.status().await);
        assert!(debug.contains("client_count: 1"), "{debug}");
        assert!(!debug.contains("[3, 3, 3, 3, 3, 3]"), "{debug}");
        assert!(!debug.contains("BEGIN CERTIFICATE"), "{debug}");
        assert!(!debug.contains("PRIVATE KEY"), "{debug}");
    })
    .catch_unwind()
    .await;
    finish(hub, outcome).await;
}

#[tokio::test]
async fn admin_deny_over_tls_leaves_incumbent_live() {
    let f = Fixture::new();
    let config = typed_config(&f).with_admission_policy(|input| {
        if input.claimed_vmac == [9; 6] {
            ScHubAdmissionDecision::Deny
        } else {
            ScHubAdmissionDecision::Allow
        }
    });
    let hub = bounded(ScHub::start("127.0.0.1:0", config, HUB_VMAC, HUB_UUID))
        .await
        .unwrap();
    let outcome = AssertUnwindSafe(async {
        let address = hub.local_addr().unwrap();
        let mut sender = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        let mut recipient =
            websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        connect(&mut sender, 1, HUB_UUID).await;
        connect(&mut recipient, 2, HUB_UUID).await;
        // Same-UUID claim ([1; 16] is the claim behind id 1) that would
        // otherwise replace the sender: denied, with the RESOURCES/OTHER
        // wire signal and no incumbent mutation.
        let mut denied = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        send_claim(&mut denied, 9, [9; 6], [1; 16]).await;
        expect_resources_other_nak(&read_binary(&mut denied).await, 9);
        let status = hub.status().await;
        assert_eq!(status.admin_denied, 1);
        assert_eq!(status.client_count, 2);
        // Both incumbents still relay after the deny.
        relay(&mut sender, &mut recipient, 7).await;
    })
    .catch_unwind()
    .await;
    finish(hub, outcome).await;
}
