//! Real TLS principal/incarnation snapshots, independent of Connect claims.
use super::*;
use crate::port::TransportProvenance;
use crate::port_ownership::connect_loopback;

async fn snapshot(
    url: &str,
    tls: ScNodeTlsConfig,
    rx: &mut tokio::sync::mpsc::Receiver<crate::port::ReceivedNpdu>,
) -> (super::super::super::TlsWebSocket, TransportProvenance) {
    let (ws, mut conn) = dial_and_handshake(url, tls).await;
    let mut wire = BytesMut::new();
    encode_sc_message(
        &mut wire,
        &conn.build_direct_encapsulated_npdu(NPDU, &[]).unwrap(),
    );
    ws.send(&wire).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    ws.send(&wire).await.unwrap();
    let second = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.provenance, second.provenance);
    assert_eq!(first.source_mac.as_ref(), DIAL_VMAC);
    assert!(first.provenance.is_direct_peer());
    (ws, first.provenance)
}

#[tokio::test]
async fn direct_principal_different_leaf_replacement_has_distinct_snapshot() {
    let ca = TestCa::generate();
    let (mut listener, mut rx) = start_listener(&ca, |c| c.with_max_established_peers(1)).await;
    let url = direct_url(&listener.local_addr());
    let (_a, a) = snapshot(&url, ca.node_config(vec!["a".into()]), &mut rx).await;
    let (_b, b) = snapshot(&url, ca.node_config(vec!["b".into()]), &mut rx).await;
    assert_ne!(a, b, "same UUID/VMAC must not erase verified leaf identity");
    let a = a.direct_sc_identity().unwrap();
    let b = b.direct_sc_identity().unwrap();
    assert_ne!(a.leaf_sha256(), b.leaf_sha256());
    assert_ne!(a.incarnation(), b.incarnation());
    assert_eq!(format!("{a:?}"), "DirectScIdentity { .. }");
    listener.stop().await;
}

#[tokio::test]
async fn direct_principal_same_leaf_reconnect_has_distinct_incarnation() {
    let ca = TestCa::generate();
    let tls = ca.node_config(vec!["a".into()]);
    let (mut listener, mut rx) = start_listener(&ca, |c| c.with_max_established_peers(1)).await;
    let url = direct_url(&listener.local_addr());
    let (_a, a) = snapshot(&url, tls.clone(), &mut rx).await;
    let (_b, b) = snapshot(&url, tls, &mut rx).await;
    assert_ne!(a, b, "a reconnect must never reuse an admitted incarnation");
    let a = a.direct_sc_identity().unwrap();
    let b = b.direct_sc_identity().unwrap();
    assert_eq!(a.leaf_sha256(), b.leaf_sha256());
    assert_ne!(a.incarnation(), b.incarnation());
    listener.stop().await;
}

#[tokio::test]
async fn direct_principal_listener_restart_cannot_reuse_queued_identity() {
    let ca = TestCa::generate();
    let tls = ca.node_config(vec!["a".into()]);
    let (mut listener, mut rx) = start_listener(&ca, |c| c).await;
    let (_a, original) = snapshot(&direct_url(&listener.local_addr()), tls.clone(), &mut rx).await;
    listener.stop().await;
    let (mut restarted, mut rx) = start_listener(&ca, |c| c).await;
    let (_b, replacement) = snapshot(&direct_url(&restarted.local_addr()), tls, &mut rx).await;
    let a = original.direct_sc_identity().unwrap();
    let b = replacement.direct_sc_identity().unwrap();
    assert_eq!(a.leaf_sha256(), b.leaf_sha256());
    assert_ne!(a.incarnation(), b.incarnation());
    // The old copied snapshot stays meaningful after the admitting owner stops.
    assert_eq!(original.direct_sc_identity(), Some(a));
    restarted.stop().await;
}

#[test]
fn direct_principal_absent_chain_fails_closed_and_hashes_exact_leaf() {
    let hash = super::super::verified_leaf_sha256;
    assert!(hash(None).is_none());
    assert!(hash(Some(&[])).is_none());
    let chain = [
        CertificateDer::from(b"abc".to_vec()),
        CertificateDer::from(b"issuer".to_vec()),
    ];
    // Independent SHA-256 known answer for the exact leaf bytes "abc".
    assert_eq!(
        hash(Some(&chain)).unwrap(),
        [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ]
    );
}

#[tokio::test]
async fn direct_principal_verified_leaf_survives_real_tls_resumption() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let ca = TestCa::generate();
    let listener = tokio::net::TcpListener::bind(loopback_addr())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let server = ca.node_config(vec!["localhost".into()]);
    let client = ca.node_config(vec!["client".into()]);
    let task = tokio::spawn(async move {
        let mut observed = Vec::new();
        for _ in 0..2 {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut tls = server.acceptor().accept(tcp).await.unwrap();
            observed.push((
                tls.get_ref().1.handshake_kind().unwrap(),
                super::super::verified_leaf_sha256(tls.get_ref().1.peer_certificates()).unwrap(),
            ));
            tls.write_all(&[1]).await.unwrap();
            assert_eq!(tls.read_u8().await.unwrap(), 2);
        }
        observed
    });
    for _ in 0..2 {
        let tcp = connect_loopback(addr).await;
        let mut tls = client
            .clone()
            .into_connector()
            .connect("localhost".try_into().unwrap(), tcp)
            .await
            .unwrap();
        // Process the server's post-handshake ticket before the next connect.
        assert_eq!(tls.read_u8().await.unwrap(), 1);
        tls.write_all(&[2]).await.unwrap();
    }
    let observed = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observed[0].0, rustls::HandshakeKind::Full);
    assert_eq!(observed[1].0, rustls::HandshakeKind::Resumed);
    assert_eq!(observed[0].1, observed[1].1);
}
