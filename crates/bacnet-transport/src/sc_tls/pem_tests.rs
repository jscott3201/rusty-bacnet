use super::*;

#[allow(dead_code)]
#[path = "../../tests/support/sc_pem_materials.rs"]
mod fixture;
use fixture::pem;

/// The inspection's decoding: input order, unknown sections skipped, the
/// first key taken without reading on, and a bad later block found only
/// when the iterator reaches it.
#[test]
fn decoder_retains_order_unknown_and_first_key_contracts() {
    let mut chain = pem("UNKNOWN", b"ignored");
    chain.extend(pem("CERTIFICATE", b"first"));
    chain.extend(pem("CERTIFICATE", b"second"));
    let certs = certificate_iter(&chain)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        certs.iter().map(|c| c.as_ref()).collect::<Vec<_>>(),
        [b"first".as_slice(), b"second"]
    );
    assert!(certificate_iter(b"").next().is_none());
    for (label, expected) in [
        ("RSA PRIVATE KEY", 1),
        ("EC PRIVATE KEY", 2),
        ("PRIVATE KEY", 3),
    ] {
        let mut bytes = pem(label, b"unusable DER still decodes");
        bytes.extend(b"-----BEGIN SECRET malformed\n");
        let key = private_key(&bytes).unwrap();
        assert!(matches!(
            (expected, key),
            (1, PrivateKeyDer::Pkcs1(_))
                | (2, PrivateKeyDer::Sec1(_))
                | (3, PrivateKeyDer::Pkcs8(_))
        ));
    }
    assert!(private_key(&pem("ENCRYPTED PRIVATE KEY", b"encrypted")).is_err());
    let mut invalid_tail = pem("CERTIFICATE", b"first");
    invalid_tail.extend(b"-----BEGIN CERTIFICATE-----\n***\n-----END CERTIFICATE-----\n");
    let mut iter = certificate_iter(&invalid_tail);
    assert_eq!(iter.next().unwrap().unwrap().as_ref(), b"first");
    assert!(iter.next().unwrap().is_err());
}
