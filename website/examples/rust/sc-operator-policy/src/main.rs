//! Remote writes and DeviceCommunicationControl from one certificate name only.
//!
//! The policy allows a covered mutation (WriteProperty, CreateObject,
//! DeleteObject and the rest) or a DeviceCommunicationControl request only
//! when it arrived over a verified direct BACnet/SC connection from the peer
//! that originated it, and that peer's certificate carries a DNS Subject
//! Alternative Name equal to the configured one. Every other request is
//! denied: B/IP, MS/TP and the other links, hub-relayed and routed requests
//! carry no certificate.
//!
//! Usage: `BACNET_DCC_PASSWORD=... sc-operator-policy <operator-dns-name>`
#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;

use bacnet_server::ServerPolicy;
use bacnet_server::device_communication_control::DeviceCommunicationControlAuthorizationContext;
use bacnet_server::mutation::MutationAuthorizationContext;
use bacnet_server::policy::{ScPeerCertificate, ScSubjectAltName};
use bacnet_server::server::{DccPolicy, DccState, ServerConfig};
use bacnet_transport::port::TransportProvenance;
use bacnet_types::MacAddr;

/// Allows writes and DCC only from a certificate that names `operator`.
struct OperatorOnly {
    operator: String,
}

impl OperatorOnly {
    /// Whether `certificate` carries a DNS name equal to the operator's:
    /// exact equality, with no case folding, wildcard or suffix matching.
    fn names_operator(&self, certificate: Option<&ScPeerCertificate>) -> bool {
        certificate
            .and_then(ScPeerCertificate::metadata)
            .is_some_and(|metadata| {
                metadata.subject_alt_names().iter().any(|name| match name {
                    ScSubjectAltName::Dns(dns) => *dns == self.operator,
                    _ => false,
                })
            })
    }
}

impl ServerPolicy for OperatorOnly {
    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        self.names_operator(context.certificate())
    }

    fn authorize_device_communication_control(
        &self,
        context: &DeviceCommunicationControlAuthorizationContext,
    ) -> bool {
        self.names_operator(context.certificate())
    }
}

fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let password = std::env::var("BACNET_DCC_PASSWORD").unwrap_or_default();
    let (Some(operator), None, false) = (args.next(), args.next(), password.is_empty()) else {
        writeln!(
            io::stderr(),
            "usage: BACNET_DCC_PASSWORD=... sc-operator-policy <operator-dns-name>"
        )?;
        return Ok(ExitCode::from(2));
    };
    let policy = Arc::new(OperatorOnly { operator });

    // Pass this configuration to BACnetServer::start with the SC transport
    // that owns your direct listener; a hub-only server never sees a
    // certificate. The static DCC settings run before the policy: the
    // default, DccPolicy::DenyAll, refuses every request without asking it.
    let _config = ServerConfig {
        policy: Some(policy.clone()),
        dcc_policy: DccPolicy::RequirePassword,
        dcc_password: Some(password),
        ..ServerConfig::default()
    };

    // A request with no verified direct SC certificate, as from B/IP,
    // MS/TP, a hub relay or a router, is denied.
    let unverified = DeviceCommunicationControlAuthorizationContext {
        source_mac: MacAddr::from_slice(&[192, 0, 2, 10, 0xBA, 0xC0]),
        source_network: None,
        provenance: TransportProvenance::unverified(),
        invoke_id: 1,
        requested_state: DccState::DisableInitiation,
        duration_minutes: Some(5),
    };
    if policy.authorize_device_communication_control(&unverified) {
        return Err("a request without a certificate was allowed".into());
    }
    writeln!(io::stdout(), "DCC without a certificate: denied")?;
    Ok(ExitCode::SUCCESS)
}
