//! Real listening capability for ACK and direct-discovery interoperability tests.
use super::*;
use crate::sc_tls::{DirectAcceptConfig, DirectListener, ScNodeTlsConfig};
use rustls::pki_types::PrivatePkcs8KeyDer;

pub(in crate::sc) async fn register_listener(
    transport: ScTransport<LoopbackWebSocket>,
    vmac: Vmac,
    uuid: [u8; 16],
) -> (ScTransport<LoopbackWebSocket>, DirectListener) {
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .signed_by(&key, &issuer)
        .unwrap();
    let tls = ScNodeTlsConfig::from_der(
        vec![ca.der().clone()],
        vec![cert.der().clone()],
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
    .unwrap();
    transport
        .with_direct_listener(DirectAcceptConfig::new(
            "127.0.0.1:0".parse().unwrap(),
            vmac,
            uuid,
            tls,
        ))
        .await
        .unwrap()
}
