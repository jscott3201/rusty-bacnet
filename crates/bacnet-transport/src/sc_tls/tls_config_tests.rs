use super::*;
use crate::sc_tls::ScTlsInput;
use rcgen::{CertificateParams, Issuer, KeyPair};
use rustls::pki_types::{PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer};

struct Material {
    ca: CertificateDer<'static>,
    leaf: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

impl Material {
    fn new(years: Option<(i32, i32)>) -> Self {
        let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let cert = ca.self_signed(&ca_key).unwrap();
        let mut params = CertificateParams::new(vec!["node".into()]).unwrap();
        if let Some((start, end)) = years {
            params.not_before = rcgen::date_time_ymd(start, 1, 1);
            params.not_after = rcgen::date_time_ymd(end, 1, 1);
        }
        let key = KeyPair::generate().unwrap();
        let leaf = params
            .signed_by(&key, &Issuer::from_params(&ca, &ca_key))
            .unwrap();
        Self {
            ca: cert.der().clone(),
            leaf: leaf.der().clone(),
            key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        }
    }

    fn policy(&self) -> ScNodeTlsConfig {
        ScNodeTlsConfig::from_der(
            vec![self.ca.clone()],
            vec![self.leaf.clone()],
            self.key.clone_key(),
        )
        .unwrap()
    }
}

/// The configuration error `result` holds, which shows no key material.
fn refused(result: Result<ScNodeTlsConfig, ScTlsConfigError>) -> ScTlsConfigError {
    let error = result.unwrap_err();
    assert!(!error.to_string().contains("PRIVATE KEY"));
    error
}

#[test]
fn node_tls_factory_requires_nonempty_ca_and_identity() {
    let m = Material::new(None);
    let error = refused(ScNodeTlsConfig::from_der(
        vec![],
        vec![m.leaf.clone()],
        m.key.clone_key(),
    ));
    assert!(
        matches!(
            error,
            ScTlsConfigError::NoCertificate {
                input: ScTlsInput::CaBundle,
                ..
            }
        ),
        "{error:?}"
    );
    let error = refused(ScNodeTlsConfig::from_der(vec![m.ca], vec![], m.key));
    assert!(
        matches!(
            error,
            ScTlsConfigError::NoCertificate {
                input: ScTlsInput::CertificateChain,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn node_tls_factory_rejects_malformed_der_in_every_position() {
    let m = Material::new(None);
    for position in 0..3 {
        let mut ca = vec![m.ca.clone(); 3];
        ca[position] = CertificateDer::from(vec![1, 2, 3]);
        let error = refused(ScNodeTlsConfig::from_der(
            ca,
            vec![m.leaf.clone()],
            m.key.clone_key(),
        ));
        assert!(
            matches!(error, ScTlsConfigError::UnusableCertificate { input: ScTlsInput::CaBundle, index, .. } if index == position),
            "{error:?}"
        );
        let mut chain = vec![m.leaf.clone(), m.ca.clone(), m.ca.clone()];
        chain[position] = CertificateDer::from(vec![1, 2, 3]);
        let error = refused(ScNodeTlsConfig::from_der(
            vec![m.ca.clone()],
            chain,
            m.key.clone_key(),
        ));
        assert!(
            matches!(error, ScTlsConfigError::UnusableCertificate { input: ScTlsInput::CertificateChain, index, .. } if index == position),
            "{error:?}"
        );
    }
}

#[test]
fn node_tls_factory_rejects_unusable_and_mismatched_keys() {
    let m = Material::new(None);
    for key in [
        PrivatePkcs8KeyDer::from(vec![1, 2, 3]).into(),
        PrivatePkcs1KeyDer::from(vec![1, 2, 3]).into(),
        PrivateSec1KeyDer::from(vec![1, 2, 3]).into(),
    ] {
        let error = refused(ScNodeTlsConfig::from_der(
            vec![m.ca.clone()],
            vec![m.leaf.clone()],
            key,
        ));
        assert!(
            matches!(error, ScTlsConfigError::UnusableKey { .. }),
            "{error:?}"
        );
    }
    let error = refused(ScNodeTlsConfig::from_der(
        vec![m.ca],
        vec![m.leaf],
        Material::new(None).key,
    ));
    assert!(matches!(error, ScTlsConfigError::KeyMismatch), "{error:?}");
}

#[test]
fn node_tls_factory_owns_material_and_clones_share_the_entire_policy() {
    fn traits<T: Clone + Send + Sync>() {}
    traits::<ScNodeTlsConfig>();
    let m = Material::new(None);
    let config = m.policy();
    drop(m);
    let clone = config.clone();
    assert!(Arc::ptr_eq(&config.inner, &clone.inner));
    drop(config);
    assert_eq!(format!("{clone:?}"), "ScNodeTlsConfig { .. }");
    assert!(!clone.inner.enable_early_data);
}

#[test]
fn node_tls_factory_does_not_preflight_local_dates_or_issuer_authorization() {
    for dates in [None, Some((2000, 2001)), Some((4090, 4091))] {
        let m = Material::new(dates);
        let _same_issuer = m.policy();
        // Trust and local identity need not have the same issuer to construct.
        ScNodeTlsConfig::from_der(vec![Material::new(None).ca], vec![m.leaf], m.key).unwrap();
    }
}

#[test]
fn a_decoded_key_prints_no_key_bytes() {
    let pair = KeyPair::generate().unwrap();
    let der = pair.serialize_der();
    let key = crate::sc_tls::material::single_key(pair.serialize_pem().as_bytes()).unwrap();
    // The decoded key sits in a clearing wrapper whose Debug elides it.
    let shown = format!("{key:?}");
    assert!(shown.contains("elided"), "{shown}");
    let decimal = format!("{:?}", &der[der.len() - 16..]);
    assert!(!shown.contains(decimal.trim_matches(['[', ']'])), "{shown}");
}

/// A reader that hands out at most `chunk` bytes per call, as a pipe does.
struct Trickle<'a> {
    data: &'a [u8],
    chunk: usize,
}

impl std::io::Read for Trickle<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.chunk.min(self.data.len()).min(out.len());
        out[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

#[test]
fn a_bounded_read_with_no_length_hint_loads_and_still_refuses_an_oversized_input() {
    use crate::sc_tls::material::read_limited;
    let data: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
    let trickle = |chunk| Trickle { data: &data, chunk };
    for (hint, chunk) in [(0, 7), (0, 4096), (10, 1), (5000, 5000)] {
        let got = read_limited(trickle(chunk), hint, 8192).unwrap();
        assert_eq!(&got[..], &data[..], "hint {hint}, chunk {chunk}");
    }
    // At the limit it is kept whole; past it, the result is one byte over.
    assert_eq!(read_limited(trickle(100), 0, 5000).unwrap().len(), 5000);
    assert_eq!(read_limited(trickle(100), 0, 4999).unwrap().len(), 5000);
    assert_eq!(read_limited(trickle(100), 0, 100).unwrap().len(), 101);
    let empty = Trickle {
        data: &[],
        chunk: 1,
    };
    assert!(read_limited(empty, 0, 100).unwrap().is_empty());
}
