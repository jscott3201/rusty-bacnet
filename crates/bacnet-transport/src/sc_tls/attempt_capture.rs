//! Observe the actual verifier callback without changing its decision or identity.

use std::{cell::RefCell, fmt, future::Future, sync::Arc};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, DistinguishedName, Error, SignatureScheme};

type Chain = Vec<Vec<u8>>;

tokio::task_local! {
    // Scoped per future poll, not per OS thread or configuration. Concurrent,
    // nested and cancelled attempts cannot borrow another attempt's evidence.
    static OBSERVED: RefCell<Option<Chain>>;
}

pub(super) async fn capture<F: Future>(future: F) -> (F::Output, Option<Chain>) {
    OBSERVED
        .scope(RefCell::new(None), async {
            let result = future.await;
            let chain = OBSERVED.with(|observed| observed.borrow_mut().take());
            (result, chain)
        })
        .await
}

/// One stable wrapper per node configuration, shared with its resumption cache.
/// Creating a verifier per attempt would break Rustls's verifier identity check.
pub(super) struct ObservingVerifier(pub(super) Arc<dyn ServerCertVerifier>);

impl fmt::Debug for ObservingVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservingVerifier").finish_non_exhaustive()
    }
}

impl ServerCertVerifier for ObservingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        // Rustls has already bounded the Certificate message before calling us.
        // Copy bytes only; parse metadata after failure and outside TLS. Release
        // the local borrow before delegation. No scope means no observation,
        // never a change to the normal verifier decision.
        let _ = OBSERVED.try_with(|observed| {
            *observed.borrow_mut() = Some(
                std::iter::once(end_entity)
                    .chain(intermediates)
                    .map(|cert| cert.as_ref().to_vec())
                    .collect(),
            );
        });
        self.0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.0.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        self.0.root_hint_subjects()
    }
}

#[cfg(test)]
#[path = "attempt_capture_tests.rs"]
mod tests;
