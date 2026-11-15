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
