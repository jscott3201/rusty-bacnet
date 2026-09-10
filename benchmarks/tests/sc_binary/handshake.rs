use super::peer::Peer;
use super::preflight::replace;
use super::support::*;
use bacnet_benchmarks::sc_helpers::*;
use bacnet_transport::sc_tls::ScNodeTlsConfig;
use rcgen::{CertificateParams, Issuer, KeyPair};
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use std::sync::Arc;

fn identity(
    material: &CertMaterial,
    cert: &str,
    key: &str,
    version: &'static rustls::SupportedProtocolVersion,
) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(material.ca_cert_pem.as_bytes()).unwrap())
        .unwrap();
    Arc::new(
        rustls::ClientConfig::builder_with_protocol_versions(&[version])
            .with_root_certificates(roots)
            .with_client_auth_cert(
                vec![CertificateDer::from_pem_slice(cert.as_bytes()).unwrap()],
                PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
            )
            .unwrap(),
    )
}

async fn good_pair(files: &Files, certs: &CertMaterial) {
    let mut hub = Process::start(&mut files.secure_hub(), files);
    let url = hub.hub_url().await;
    let mut device = Process::start(&mut files.device(&url), files);
    device.ready("SC device connected").await;
    let peer = Peer::connect(&url, try_make_node_tls_config(certs).unwrap(), 42).await;
    peer.read().await;
    assert!(device.child.try_wait().unwrap().is_none());
}

#[tokio::test]
async fn hub_identity_is_explicit_and_stable_across_binary_restart() {
    let files = Files::new();
    let certs = generate_test_certs();
    files.certs(&certs);
    let uuid = [0, 1, 2, 3, 4, 5, 0, 7, 0, 9, 10, 11, 12, 13, 14, 15];
    let vmac = [0xff, 0, 0, 0, 0, 0x51];
    let mut address = "127.0.0.1:0".to_owned();
    for _ in 0..3 {
        let base = replace(
            &files.secure_hub(),
            "--device-uuid",
            "000102030405000700090A0B0C0D0E0F",
        );
        let mut cmd = replace(&base, "--listen", &address);
        cmd.args(["--vmac", "FF0000000051"]);
        let mut hub = Process::start(&mut cmd, &files);
        let url = hub.hub_url().await;
        address = url.trim_start_matches("wss://").to_owned();
        let mut device = Process::start(&mut files.device(&url), &files);
        device.ready("SC device connected").await;
        let peer = Peer::connect_identity(
            &url,
            try_make_node_tls_config(&certs).unwrap(),
            42,
            vmac,
            uuid,
        )
        .await;
        peer.read().await;
        drop(peer);
        drop(device);
        // Kill-and-wait is explicit child reaping, not a timeout-only oracle.
        hub.child.kill().unwrap();
        assert!(!hub.wait().await.success());
        drop(hub);
        drop(tokio::net::TcpListener::bind(&address).await.unwrap());
    }
}

#[tokio::test]
async fn actual_binaries_mutual_tls_reads_and_denials_recover() {
    // Distinct signing CA, hub, device and peer keys, with explicit EKUs.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Standalone SC test site");
    let ca_key = KeyPair::generate().unwrap();
    let ca = params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::from_params(&params, &ca_key);
    let leaf = |name: &str, usage, years: Option<(i32, i32)>| {
        let mut params = CertificateParams::new(vec![name.into()]).unwrap();
        params.extended_key_usages = vec![usage];
        if let Some((start, end)) = years {
            params.not_before = rcgen::date_time_ymd(start, 1, 1);
            params.not_after = rcgen::date_time_ymd(end, 1, 1);
        }
        let key = KeyPair::generate().unwrap();
        (
            params.signed_by(&key, &issuer).unwrap().pem(),
            key.serialize_pem(),
        )
    };
    use rcgen::ExtendedKeyUsagePurpose::{ClientAuth, ServerAuth};
    let (server_cert_pem, server_key_pem) = leaf("127.0.0.1", ServerAuth, None);
    let (client_cert_pem, client_key_pem) = leaf("device", ClientAuth, None);
    let certs = CertMaterial {
        ca_cert_pem: ca.pem(),
        server_cert_pem,
        server_key_pem,
        client_cert_pem,
        client_key_pem,
    };
    let files = Files::new();
    files.certs(&certs);
    let mut hub = Process::start(&mut files.secure_hub(), &files);
    let url = hub.hub_url().await;
    let mut device = Process::start(&mut files.device(&url), &files);
    device.ready("SC device connected").await;
    let (cert, key) = leaf("independent peer", ClientAuth, None);
    let peer_tls = identity(&certs, &cert, &key, &rustls::version::TLS13);
    // Directly observe TLS 1.3 on the actual executable hub.
    let stream = bounded(tokio::net::TcpStream::connect(
        url.trim_start_matches("wss://"),
    ))
    .await
    .unwrap();
    let tls = bounded(tokio_rustls::TlsConnector::from(peer_tls.clone()).connect(
        rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap(),
        stream,
    ))
    .await
    .unwrap();
    assert_eq!(
        tls.get_ref().1.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
    drop(tls);
    let node = ScNodeTlsConfig::from_der(
        vec![CertificateDer::from_pem_slice(certs.ca_cert_pem.as_bytes()).unwrap()],
        vec![CertificateDer::from_pem_slice(cert.as_bytes()).unwrap()],
        PrivateKeyDer::from_pem_slice(key.as_bytes()).unwrap(),
    )
    .unwrap();
    let peer = Peer::connect(&url, node, 42).await;
    peer.read().await;
    let wrong = generate_test_certs();
    let (expired, expired_key) = leaf("expired", ClientAuth, Some((2000, 2001)));
    let (future, future_key) = leaf("future", ClientAuth, Some((4090, 4091)));
    let negatives = [
        (make_client_tls_config(&certs), "CertificateRequired"),
        (
            identity(
                &certs,
                &wrong.client_cert_pem,
                &wrong.client_key_pem,
                &rustls::version::TLS13,
            ),
            "UnknownCA",
        ),
        (
            identity(&certs, &expired, &expired_key, &rustls::version::TLS13),
            "CertificateExpired",
        ),
        (
            identity(&certs, &future, &future_key, &rustls::version::TLS13),
            "CertificateExpired",
        ),
        (
            identity(&certs, &cert, &key, &rustls::version::TLS12),
            "ProtocolVersion",
        ),
    ];
    for (tls, expected) in negatives {
        let error = match bounded(tokio_tungstenite::connect_async_tls_with_config(
            &url,
            None,
            false,
            Some(tokio_tungstenite::Connector::Rustls(tls)),
        ))
        .await
        {
            Ok(_) => panic!("invalid peer admitted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains(expected), "expected {expected}, got {error}");
        peer.read().await;
    }
    // The standalone device also pins only its explicit CA, not SSL_CERT_FILE.
    let wrong_ca = files.write("wrong-ca.pem", &wrong.ca_cert_pem);
    let mut cmd = replace(&files.device(&url), "--sc-ca", wrong_ca.to_str().unwrap());
    cmd.env("SSL_CERT_FILE", files.0.join("ca.pem"));
    let mut bad = Process::start(&mut cmd, &files);
    assert!(!bad.wait().await.success());
    let error = bad.output().1;
    assert!(error.contains("UnknownIssuer"), "{error}");
    peer.read().await;
}

#[tokio::test]
async fn device_rejects_hub_dates_and_san_then_good_pair_reads() {
    for (certs, expected) in [
        (
            generate_test_certs_with_expired_server(),
            "certificate expired",
        ),
        (
            generate_test_certs_with_not_yet_valid_server(),
            "not valid yet",
        ),
        (
            generate_test_certs_with_wrong_server_name(),
            "not valid for name",
        ),
    ] {
        let files = Files::new();
        files.certs(&certs);
        let mut hub = Process::start(&mut files.secure_hub(), &files);
        let url = hub.hub_url().await;
        let mut device = Process::start(&mut files.device(&url), &files);
        assert!(!device.wait().await.success());
        let error = device.output().1;
        assert!(
            error.contains("TLS handshake") && error.contains(expected),
            "{error}"
        );
        drop(device);
        drop(hub);
        let good = generate_test_certs();
        let good_files = Files::new();
        good_files.certs(&good);
        good_pair(&good_files, &good).await;
    }
}

#[tokio::test]
async fn device_tls12_rejected_and_connect_request_carries_explicit_identity() {
    use bacnet_transport::sc_frame::{
        decode_sc_message, encode_sc_message, ScFunction, ScMessage, BACNET_SC_HUB_SUBPROTOCOL,
    };
    use bytes::{Bytes, BytesMut};
    use futures_util::{SinkExt, StreamExt};
    let files = Files::new();
    let certs = generate_test_certs();
    files.certs(&certs);
    for version in [&rustls::version::TLS12, &rustls::version::TLS13] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("wss://{}", listener.local_addr().unwrap());
        let mut config = (*make_server_tls_config_mtls(&certs)).clone();
        // Build independently rather than mutate the typed hub's fixed policy.
        if version.version == rustls::ProtocolVersion::TLSv1_2 {
            let mut roots = rustls::RootCertStore::empty();
            roots.add(pem(&files.0.join("ca.pem")).remove(0)).unwrap();
            config = rustls::ServerConfig::builder_with_protocol_versions(&[version])
                .with_client_cert_verifier(
                    rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
                        .build()
                        .unwrap(),
                )
                .with_single_cert(
                    pem(&files.0.join("hub.pem")),
                    PrivateKeyDer::from_pem_file(files.0.join("hub.key")).unwrap(),
                )
                .unwrap();
        }
        let mut device = Process::start(&mut files.device(&url), &files);
        let (tcp, _) = bounded(listener.accept()).await.unwrap();
        let result = bounded(tokio_rustls::TlsAcceptor::from(Arc::new(config)).accept(tcp)).await;
        if version.version == rustls::ProtocolVersion::TLSv1_2 {
            assert!(format!("{:?}", result.err().unwrap()).contains("PeerIncompatible"));
            assert!(!device.wait().await.success());
            assert!(device.output().1.contains("ProtocolVersion"));
            continue;
        }
        let tls = result.unwrap();
        assert_eq!(
            tls.get_ref().1.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );
        assert_eq!(
            tls.get_ref().1.peer_certificates().unwrap(),
            pem(&files.0.join("device.pem"))
        );
        let mut ws = bounded(tokio_tungstenite::accept_hdr_async(tls,
            |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert("Sec-WebSocket-Protocol", BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap());
                Ok(response)
            })).await.unwrap();
        let frame = bounded(ws.next()).await.unwrap().unwrap().into_data();
        let request = decode_sc_message(&frame).unwrap();
        assert_eq!(request.function, ScFunction::ConnectRequest);
        assert_eq!(&request.payload[..6], &[2, 0, 0, 0, 0x13, 0x88]);
        assert_eq!(
            &request.payload[6..22],
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x13, 0x88]
        );
        let mut payload = vec![9; 6];
        payload.extend_from_slice(&[9; 16]);
        payload.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
        let mut frame = BytesMut::new();
        encode_sc_message(
            &mut frame,
            &ScMessage {
                function: ScFunction::ConnectAccept,
                message_id: request.message_id,
                originating_vmac: None,
                destination_vmac: None,
                dest_options: vec![],
                data_options: vec![],
                payload: Bytes::from(payload),
            },
        );
        bounded(ws.send(tokio_tungstenite::tungstenite::Message::Binary(
            frame.freeze(),
        )))
        .await
        .unwrap();
        device.ready("SC device connected").await;
    }
    good_pair(&files, &certs).await;
}
