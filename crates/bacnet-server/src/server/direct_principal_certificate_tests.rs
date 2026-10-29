//! The verified leaf certificate in policy contexts over real direct TLS
//! (#1965): a policy allows by Subject Alternative Name, so a renewed
//! certificate keeps its access, and a routed request shows only its
//! forwarding peer's certificate.
use super::*;
use crate::mutation::MutationAuthorizationContext;
use bacnet_transport::sc_tls::{ScPeerCertificate, ScSubjectAltName};

/// What one decision saw: the originating and forwarding peers' DNS SANs,
/// whether both certificates were the same DER, and the context's Debug.
type Seen = Arc<StdMutex<Vec<(Vec<String>, Vec<String>, bool, String)>>>;

fn dns_names(certificate: Option<&ScPeerCertificate>) -> Vec<String> {
    certificate
        .and_then(ScPeerCertificate::metadata)
        .map(|metadata| {
            metadata
                .subject_alt_names()
                .iter()
                .filter_map(|name| match name {
                    ScSubjectAltName::Dns(dns) => Some(dns.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Writes from any certificate naming `operator`; records each decision.
fn san_policy(seen: Seen) -> ServerConfig {
    ServerConfig {
        policy: Some(Arc::new(crate::FnPolicy::new().mutation(
            move |context: &MutationAuthorizationContext| {
                let names = dns_names(context.certificate());
                let same = match (context.certificate(), context.forwarding_peer_certificate()) {
                    (Some(a), Some(b)) => a.der() == b.der(),
                    _ => false,
                };
                seen.lock().unwrap().push((
                    names.clone(),
                    dns_names(context.forwarding_peer_certificate()),
                    same,
                    format!("{context:?}"),
                ));
                names.iter().any(|name| name == "operator")
            },
        ))),
        ..Default::default()
    }
}

fn write(text: &str, invoke: u8) -> Apdu {
    confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY,
        invoke,
        write_payload(text),
    )
}

#[tokio::test]
async fn a_policy_allows_by_subject_alt_name_across_renewal() {
    let ca = TestCa::new();
    let seen = Seen::default();
    let mut f = Fixture::new(
        &ca,
        san_policy(seen.clone()),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;

    let mut operator = f.peer(ca.tls("operator")).await;
    let first = f.capture_from(&mut operator, &write("one", 1), None).await;
    let first_principal = first.provenance.principal().unwrap();
    f.feed(first).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));

    // A renewed certificate (new key, so a new principal) with the same SAN
    // is still allowed: no fingerprint list to update.
    let mut renewed = f.peer(ca.tls("operator")).await;
    let second = f.capture_from(&mut renewed, &write("two", 2), None).await;
    assert_ne!(second.provenance.principal(), Some(first_principal));
    f.feed(second).await;
    assert!(matches!(f.response().await, Apdu::SimpleAck(_)));

    // Another CA-valid peer without that SAN is denied.
    let mut intruder = f.peer(ca.tls("intruder")).await;
    let third = f
        .capture_from(&mut intruder, &write("three", 3), None)
        .await;
    f.feed(third).await;
    assert!(denied(&f.response().await));

    // Routed through the intruder, only the forwarder's certificate shows.
    let routed = f.capture(&mut intruder, &write("four", 4)).await;
    f.feed(routed).await;
    assert!(denied(&f.response().await));

    let seen = seen.lock().unwrap().clone();
    let decisions: Vec<_> = seen
        .iter()
        .map(|(names, forwarder, same, _)| (names.clone(), forwarder.clone(), *same))
        .collect();
    let names = |name: &str| vec![name.to_owned()];
    assert_eq!(
        decisions,
        [
            (names("operator"), names("operator"), true),
            (names("operator"), names("operator"), true),
            (names("intruder"), names("intruder"), true),
            (Vec::new(), names("intruder"), false),
        ]
    );
    // No certificate content reaches Debug.
    for (_, _, _, debug) in &seen {
        assert!(
            !debug.contains("operator") && !debug.contains("intruder"),
            "{debug}"
        );
        assert!(!debug.contains("rcgen"), "{debug}");
    }
    f.stop().await;
}
