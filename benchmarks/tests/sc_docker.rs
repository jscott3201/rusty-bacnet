//! Opt-in read-only smoke peer for an explicitly provisioned, isolated SC pair.
//! Build natively on the Docker builder; mount this test executable and the
//! peer's three credential files read-only into a transient client container.
#[path = "sc_binary/peer.rs"]
mod peer;
#[path = "sc_binary/support.rs"]
mod support;

#[tokio::test]
#[ignore = "requires explicit isolated Docker SC pair and caller-provided peer credentials"]
async fn docker_pair_read_property() {
    use bacnet_transport::sc_tls::ScNodeTlsConfig;
    use rustls::pki_types::{pem::PemObject, PrivateKeyDer};
    let required = |name| std::env::var(name).expect("explicit smoke environment required");
    let url = required("SC_SMOKE_URL");
    let ca = required("SC_SMOKE_CA");
    let cert = required("SC_SMOKE_CERT");
    let key = required("SC_SMOKE_KEY");
    let tls = ScNodeTlsConfig::from_der(
        support::pem(std::path::Path::new(&ca)),
        support::pem(std::path::Path::new(&cert)),
        PrivateKeyDer::from_pem_file(key).unwrap(),
    )
    .unwrap();
    let peer = peer::Peer::connect(&url, tls, 42).await;
    peer.read().await;
    eprintln!("Verified mutual-TLS 1.3 SC Connect and Device:5000 / AI:1 ReadProperty values");
}
