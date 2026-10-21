//! Package-local deterministic certificate and deliberately invalid DER fixtures.

pub fn pem(kind: &str, der: &[u8]) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut body = String::new();
    for bytes in der.chunks(3) {
        let n = (u32::from(bytes[0]) << 16)
            | (u32::from(*bytes.get(1).unwrap_or(&0)) << 8)
            | u32::from(*bytes.get(2).unwrap_or(&0));
        body.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        body.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        body.push(if bytes.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        body.push(if bytes.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    format!("-----BEGIN {kind}-----\n{body}\n-----END {kind}-----\n").into_bytes()
}

pub fn tlv(tag: u8, bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    if bytes.len() < 128 {
        out.push(bytes.len() as u8);
    } else if bytes.len() < 256 {
        out.extend([0x81, bytes.len() as u8]);
    } else {
        out.extend([0x82, (bytes.len() >> 8) as u8, bytes.len() as u8]);
    }
    out.extend(bytes);
    out
}

pub fn params() -> rcgen::CertificateParams {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "offline leaf");
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2025, 1, 1);
    params
}

pub fn certificate(params: rcgen::CertificateParams) -> (Vec<u8>, Vec<u8>) {
    let key = rcgen::KeyPair::generate().unwrap();
    (
        params.self_signed(&key).unwrap().pem().into_bytes(),
        key.serialize_pem().into_bytes(),
    )
}
