//! Prepared read ownership and terminal observation shared by the closed RP/RR/RPM paths.
use super::*;

/// A reserved, encoded operation that has not submitted any traffic.
#[doc(hidden)]
pub struct PreparedEndpointRead {
    pub(super) guard: EndpointRequestGuard,
    pub(super) destination: EndpointApduDestination,
    pub(super) data_attributes: Vec<DataAttribute>,
    pub(super) request: EndpointReadRequest,
    pub(super) encoded: Vec<u8>,
    pub(super) response: tokio::sync::oneshot::Receiver<TsmResponse>,
}

/// Caller result plus the narrow transmission evidence needed by source audit.
#[doc(hidden)]
pub struct EndpointReadOutcome {
    /// The original caller result, without waiting for audit delivery.
    pub result: Result<EndpointReadAck, Error>,
    /// At least one transport execution began, or a peer terminal was observed.
    /// This does not claim remote execution when the local transport failed.
    pub attempted: bool,
}

impl PreparedEndpointRead {
    /// The exact reserved wire Invoke ID, stable across all retries.
    #[doc(hidden)]
    pub fn invoke_id(&self) -> u8 {
        self.guard.invoke_id
    }

    /// Execute under caller RAII ownership, or after session ownership transfer.
    #[doc(hidden)]
    pub async fn execute(mut self) -> EndpointReadOutcome {
        let mut attempted = false;
        let result = self.execute_inner(&mut attempted).await.and_then(|bytes| {
            if bytes.len() + 3 > usize::from(self.guard.inner.max_apdu_length) {
                return Err(Error::Segmentation(
                    "endpoint read ACK exceeds configured max APDU".into(),
                ));
            }
            self.request.decode(&bytes)
        });
        EndpointReadOutcome { result, attempted }
    }

    fn terminal(&mut self, response: TsmResponse) -> Result<bytes::Bytes, Error> {
        self.guard.active = false;
        confirmed_response_result(response)
    }

    async fn execute_inner(&mut self, attempted: &mut bool) -> Result<bytes::Bytes, Error> {
        let inner = Arc::clone(&self.guard.inner);
        for attempt in 0..=inner.retries {
            if !inner.open.load(Ordering::Acquire) {
                return Err(shutdown_error());
            }
            if let Ok(response) = self.response.try_recv() {
                *attempted = true;
                return self.terminal(response);
            }
            let send = inner.egress.admit_apdu(
                self.encoded.clone(),
                self.destination.clone(),
                true,
                NetworkPriority::NORMAL,
                self.data_attributes.clone(),
                None,
            );
            let send_result = match send {
                Ok(send) => {
                    let outcome = send.complete().await;
                    *attempted |= outcome.attempted;
                    outcome.result
                }
                Err(error) => Err(error.into()),
            };
            // A peer terminal already delivered during egress is stronger evidence
            // than a contradictory local send failure. Never parse error strings.
            if let Ok(response) = self.response.try_recv() {
                *attempted = true;
                return self.terminal(response);
            }
            send_result?;
            match tokio::time::timeout(inner.timeout, &mut self.response).await {
                Ok(Ok(response)) => {
                    *attempted = true;
                    return self.terminal(response);
                }
                Ok(Err(_)) if !inner.open.load(Ordering::Acquire) => return Err(shutdown_error()),
                Ok(Err(_)) => return Err(Error::Encoding("TSM response channel closed".into())),
                Err(_) if attempt < inner.retries => {}
                Err(_) => return Err(Error::Timeout(inner.timeout)),
            }
        }
        unreachable!("inclusive retry loop always returns")
    }
}
