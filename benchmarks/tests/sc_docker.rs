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
    let hex = |name| {
        let text = required(name);
        assert!(text.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(text.len() % 2, 0);
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
    };
    let hub_vmac = hex("SC_SMOKE_HUB_VMAC").try_into().unwrap();
    let hub_uuid = hex("SC_SMOKE_HUB_UUID").try_into().unwrap();
    let tls = ScNodeTlsConfig::from_der(
        support::pem(std::path::Path::new(&ca)),
        support::pem(std::path::Path::new(&cert)),
        PrivateKeyDer::from_pem_file(key).unwrap(),
    )
    .unwrap();
    let peer = peer::Peer::connect_identity(&url, tls, 42, hub_vmac, hub_uuid).await;
    peer.read().await;
    eprintln!("Verified mutual-TLS 1.3 exact hub VMAC/UUID Connect-Accept and Device:5000 / AI:1 ReadProperty values");
}
