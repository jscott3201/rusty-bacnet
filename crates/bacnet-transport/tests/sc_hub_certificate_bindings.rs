//! Installation-policy certificate-to-claim registration, over real mutual TLS.
#![cfg(feature = "sc-tls")]
// The shared TLS fixture includes helpers/negative identities used by sibling suites.
#[allow(dead_code)]
mod hub_tls_support;
use bacnet_transport::sc_hub::{
    ScHub, ScHubCertificateBinding, ScHubCertificateBindings, ScHubTlsConfig,
};
use futures_util::{FutureExt, SinkExt, StreamExt};
use hub_tls_support::*;
use std::panic::AssertUnwindSafe;
use tokio_tungstenite::tungstenite::Message;

fn tls(f: &Fixture) -> ScHubTlsConfig {
    ScHubTlsConfig::from_der(
        vec![f.ca.clone()],
        vec![f.server.cert.clone()],
        f.server.key.clone_key(),
    )
    .unwrap()
}

async fn claim(peer: &mut Peer, vmac: u8, uuid: u8) -> Vec<u8> {
    let mut wire = vec![6, 0, 0x22, 1];
    wire.extend_from_slice(&[vmac; 6]);
    wire.extend_from_slice(&[uuid; 16]);
    wire.extend_from_slice(&[5, 0xc4, 5, 0xc4]);
    bounded(peer.send(Message::Binary(wire.into())))
        .await
        .unwrap();
    let Message::Binary(wire) = bounded(peer.next()).await.unwrap().unwrap() else {
        panic!("expected binary Connect response")
    };
    wire.to_vec()
}

#[tokio::test]
async fn unauthorized_same_ca_leaf_cannot_claim_offline_identity() {
    let f = Fixture::new();
    let config = tls(&f).with_certificate_bindings(
        ScHubCertificateBindings::new(vec![
            ScHubCertificateBinding::new([1; 16], vec![[1; 6]], vec![digest(&f.good)]).unwrap(),
        ])
        .unwrap(),
    );
    let hub = bounded(ScHub::start("127.0.0.1:0", config, HUB_VMAC, HUB_UUID))
        .await
        .unwrap();
    let result = AssertUnwindSafe(async {
        let mut peer = websocket(
            hub.local_addr().unwrap(),
            f.client(Some(&f.server), &rustls::version::TLS13),
        )
        .await;
        assert_eq!(
            claim(&mut peer, 1, 1).await,
            [0, 0, 0x22, 1, 6, 1, 0, 0, 3, 0, 0]
        );
        let mut unreserved = websocket(
            hub.local_addr().unwrap(),
            f.client(Some(&f.server), &rustls::version::TLS13),
        )
        .await;
        denied(claim(&mut unreserved, 4, 4).await);
    })
    .catch_unwind()
    .await;
    finish(hub, result).await;
}

fn digest(identity: &Identity) -> [u8; 32] {
    aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, identity.cert.as_ref())
        .as_ref()
        .try_into()
        .unwrap()
}

fn binding(uuid: u8, vmacs: &[u8], leaves: &[[u8; 32]]) -> ScHubCertificateBinding {
    ScHubCertificateBinding::new(
        [uuid; 16],
        vmacs.iter().map(|v| [*v; 6]).collect(),
        leaves.to_vec(),
    )
    .unwrap()
}
fn mapped(f: &Fixture) -> ScHubTlsConfig {
    tls(f).with_certificate_bindings(
        ScHubCertificateBindings::new(vec![
            binding(1, &[1, 3], &[digest(&f.good)]),
            binding(2, &[2], &[digest(&f.server)]),
        ])
        .unwrap(),
    )
}
fn denied(wire: Vec<u8>) {
    assert_eq!(wire, [0, 0, 0x22, 1, 6, 1, 0, 0, 3, 0, 0]);
}
fn accepted(wire: Vec<u8>) {
    assert_eq!(&wire[..4], &[7, 0, 0x22, 1]);
}

#[test]
fn certificate_bindings_validate_whole_map_and_redact() {
    for (uuid, vmacs, leaves) in [
        ([0; 16], vec![[1; 6]], vec![[1; 32]]),
        ([1; 16], vec![], vec![[1; 32]]),
        ([1; 16], vec![[1; 6]], vec![]),
        ([1; 16], vec![[0; 6]], vec![[1; 32]]),
        ([1; 16], vec![[255; 6]], vec![[1; 32]]),
        ([1; 16], vec![[1; 6], [1; 6]], vec![[1; 32]]),
        ([1; 16], vec![[1; 6]], vec![[1; 32], [1; 32]]),
    ] {
        assert!(ScHubCertificateBinding::new(uuid, vmacs, leaves).is_err());
    }
    assert!(ScHubCertificateBindings::new(vec![]).is_err());
    for conflicting in [
        binding(1, &[2], &[[2; 32]]),
        binding(2, &[1], &[[2; 32]]),
        binding(2, &[2], &[[1; 32]]),
    ] {
        assert!(
            ScHubCertificateBindings::new(vec![binding(1, &[1], &[[1; 32]]), conflicting]).is_err()
        );
    }
    let group = binding(1, &[1, 3], &[[1; 32], [2; 32]]);
    assert_eq!(group.uuid(), [1; 16]);
    assert_eq!(group.allowed_vmacs(), &[[1; 6], [3; 6]]);
    assert_eq!(group.leaf_sha256(), &[[1; 32], [2; 32]]);
    assert_eq!(format!("{group:?}"), "ScHubCertificateBinding { .. }");
    let map = ScHubCertificateBindings::new(vec![group]).unwrap();
    assert_eq!(format!("{map:?}"), "ScHubCertificateBindings { .. }");
    assert!(map.validate_hub_vmac([1; 6]).is_err());
    assert!(map.validate_hub_vmac(HUB_VMAC).is_ok());
}

#[tokio::test]
async fn certificate_bindings_reserve_offline_and_live_identity_before_callback_or_replacement() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let f = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let called = calls.clone();
    let config = mapped(&f).with_admission_policy(move |_| {
        called.fetch_add(1, Ordering::Relaxed);
        bacnet_transport::sc_hub::ScHubAdmissionDecision::Allow
    });
    let hub = bounded(ScHub::start("127.0.0.1:0", config, HUB_VMAC, HUB_UUID))
        .await
        .unwrap();
    let result = AssertUnwindSafe(async {
        let address = hub.local_addr().unwrap();
        // Another mapped leaf cannot claim either reserved dimension, or new claims.
        for (vmac, uuid) in [(1, 2), (2, 1), (3, 1), (4, 4)] {
            let mut peer =
                websocket(address, f.client(Some(&f.server), &rustls::version::TLS13)).await;
            denied(claim(&mut peer, vmac, uuid).await);
        }
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        let mut incumbent =
            websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        accepted(claim(&mut incumbent, 1, 1).await);
        let mut recipient =
            websocket(address, f.client(Some(&f.server), &rustls::version::TLS13)).await;
        accepted(claim(&mut recipient, 2, 2).await);
        let before = hub.status().await;
        for (vmac, uuid) in [(1, 1), (3, 1), (1, 2)] {
            let mut peer =
                websocket(address, f.client(Some(&f.server), &rustls::version::TLS13)).await;
            denied(claim(&mut peer, vmac, uuid).await);
            relay(&mut incumbent, &mut recipient, 9).await;
        }
        let after = hub.status().await;
        assert_eq!(after.client_count, 2);
        assert_eq!(
            after.outcomes.uuid_replacements,
            before.outcomes.uuid_replacements
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(after.admin_denied, 7);
        // Listed VMAC does not authorize a different UUID for the authorized leaf.
        let mut peer = websocket(address, f.client(Some(&f.good), &rustls::version::TLS13)).await;
        denied(claim(&mut peer, 3, 9).await);
        // A configured identity still cannot inject an Originating VMAC.
        // The following legitimate relay is an ordered recipient barrier:
        // accepting the forged message would make relay() observe it first.
        let mut forged = vec![1, 12, 0x55, 1];
        forged.extend_from_slice(&[9; 6]);
        forged.extend_from_slice(&[2; 6]);
        forged.extend_from_slice(&[1, 0, 0x10, 8]);
        bounded(incumbent.send(Message::Binary(forged.into())))
            .await
            .unwrap();
        relay(&mut incumbent, &mut recipient, 10).await;
    })
    .catch_unwind()
    .await;
    finish(hub, result).await;
}

#[tokio::test]
async fn certificate_bindings_rotation_reconnect_and_conjunctive_policy() {
    use bacnet_transport::sc_hub::ScHubAdmissionDecision::{Allow, Deny};
    let f = Fixture::new();
    for policy in 0..3 {
        let config = tls(&f)
            .with_certificate_bindings(
                ScHubCertificateBindings::new(vec![binding(
                    1,
                    &[1, 3],
                    &[digest(&f.good), digest(&f.server)],
                )])
                .unwrap(),
            )
            .with_admission_policy(move |_| match policy {
                0 => Allow,
                1 => Deny,
                _ => panic!("test policy panic"),
            });
        let hub = bounded(ScHub::start("127.0.0.1:0", config, HUB_VMAC, HUB_UUID))
            .await
            .unwrap();
        let result = AssertUnwindSafe(async {
            let address = hub.local_addr().unwrap();
            let mut peers = Vec::new();
            for (identity, vmac) in [(&f.good, 1), (&f.good, 1), (&f.server, 3)] {
                let mut peer =
                    websocket(address, f.client(Some(identity), &rustls::version::TLS13)).await;
                let response = claim(&mut peer, vmac, 1).await;
                if policy == 0 {
                    accepted(response);
                } else {
                    denied(response);
                }
                peers.push(peer);
            }
            let status = hub.status().await;
            assert_eq!(status.client_count, usize::from(policy == 0));
            assert_eq!(
                status.outcomes.uuid_replacements,
                if policy == 0 { 2 } else { 0 }
            );
            assert_eq!(status.admin_denied, if policy == 0 { 0 } else { 3 });
        })
        .catch_unwind()
        .await;
        finish(hub, result).await;
    }
}

#[tokio::test]
async fn certificate_bindings_prebind_conflict_and_clone_runtime_independence() {
    let f = Fixture::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = tls(&f).with_certificate_bindings(
        ScHubCertificateBindings::new(vec![binding(0x10, &[0x10], &[digest(&f.good)])]).unwrap(),
    );
    let error = ScHub::start(
        &listener.local_addr().unwrap().to_string(),
        config,
        HUB_VMAC,
        HUB_UUID,
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("overlaps local Hub VMAC"));
    let config = mapped(&f);
    let first = bounded(ScHub::start(
        "127.0.0.1:0",
        config.clone(),
        HUB_VMAC,
        HUB_UUID,
    ))
    .await
    .unwrap();
    let second = bounded(ScHub::start("127.0.0.1:0", config, HUB_VMAC, HUB_UUID))
        .await
        .unwrap();
    let result = AssertUnwindSafe(async {
        let mut peer = websocket(
            first.local_addr().unwrap(),
            f.client(Some(&f.good), &rustls::version::TLS13),
        )
        .await;
        denied(claim(&mut peer, 2, 2).await);
        assert_eq!(first.status().await.admin_denied, 1);
        assert_eq!(second.status().await.admin_denied, 0);
        let mut other = websocket(
            second.local_addr().unwrap(),
            f.client(Some(&f.good), &rustls::version::TLS13),
        )
        .await;
        accepted(claim(&mut other, 1, 1).await);
    })
    .catch_unwind()
    .await;
    finish(first, Ok(())).await;
    finish(second, result).await;
}

#[tokio::test]
async fn certificate_bindings_absent_intentionally_allows_other_ca_valid_leaf() {
    let f = Fixture::new();
    let hub = bounded(ScHub::start("127.0.0.1:0", tls(&f), HUB_VMAC, HUB_UUID))
        .await
        .unwrap();
    let result = AssertUnwindSafe(async {
        let mut first = websocket(
            hub.local_addr().unwrap(),
            f.client(Some(&f.good), &rustls::version::TLS13),
        )
        .await;
        accepted(claim(&mut first, 1, 1).await);
        let mut second = websocket(
            hub.local_addr().unwrap(),
            f.client(Some(&f.server), &rustls::version::TLS13),
        )
        .await;
        accepted(claim(&mut second, 3, 1).await);
        assert_eq!(hub.status().await.outcomes.uuid_replacements, 1);
    })
    .catch_unwind()
    .await;
    finish(hub, result).await;
}
