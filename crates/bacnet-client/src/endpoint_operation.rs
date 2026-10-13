//! Prepared operation ownership and terminal observation shared by the closed RP/RR/RPM/WP paths.
use super::*;

/// A checked, encoded operation whose destination's minimum interval has
/// let it go (#1542). It holds its turn but no transaction: dropping it
/// before [`prepare`](Self::prepare) starts the next request's interval,
/// as a request given up does.
#[doc(hidden)]
pub struct PacedEndpointOperation {
    pub(super) inner: Arc<EndpointRequesterInner>,
    pub(super) destination: EndpointApduDestination,
    pub(super) data_attributes: Vec<DataAttribute>,
    pub(super) request: EndpointOperationRequest,
    pub(super) service_data: BytesMut,
    pub(super) pace: PaceGuard,
}

impl PacedEndpointOperation {
    /// Where the request goes: the destination as checked, with one routed
    /// on this endpoint's own network localized to its MAC (#1403).
    #[doc(hidden)]
    pub fn destination(&self) -> &EndpointApduDestination {
        &self.destination
    }

    /// Reserve the transaction: an invoke ID from the shared pool and the
    /// requester's TSM entry. The operation keeps its turn until it ends.
    #[doc(hidden)]
    pub fn prepare(self) -> Result<PreparedEndpointOperation, Error> {
        let Self {
            inner,
            destination,
            data_attributes,
            request,
            service_data,
            pace,
        } = self;
        let service = request.service();
        let TransactionPeer {
            tsm_mac,
            canonical: peer,
        } = outbound_tsm_peer(&destination);
        let (invoke_id, registration) = {
            let mut tsm = inner
                .tsm
                .lock()
                .map_err(|_| Error::Encoding("endpoint requester state is poisoned".into()))?;
            if !inner.open.load(Ordering::Acquire) {
                return Err(shutdown_error());
            }
            tsm.register_coordinated_transaction_with_policy(
                tsm_mac.clone(),
                peer,
                service,
                false,
                request.terminal_policy(),
            )
            .map_err(|error| Error::Encoding(error.to_string()))?
        };
        let max_apdu_length = inner.max_apdu_length;
        let guard = EndpointRequestGuard {
            inner,
            destination: tsm_mac,
            invoke_id,
            owner: registration.owner.clone(),
            active: true,
        };
        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: service_data.freeze(),
        });
        let mut encoded = BytesMut::new();
        encode_apdu(&mut encoded, &pdu)?;
        Ok(PreparedEndpointOperation {
            guard,
            destination,
            data_attributes,
            request,
            encoded: encoded.to_vec(),
            response: registration.response,
            _pace: pace,
        })
    }
}

/// A reserved, encoded operation that has not submitted any traffic.
#[doc(hidden)]
pub struct PreparedEndpointOperation {
    pub(super) guard: EndpointRequestGuard,
    pub(super) destination: EndpointApduDestination,
    pub(super) data_attributes: Vec<DataAttribute>,
    pub(super) request: EndpointOperationRequest,
    pub(super) encoded: Vec<u8>,
    pub(super) response: tokio::sync::oneshot::Receiver<TsmResponse>,
    /// The destination's turn, held through every retry until the
    /// operation ends, by a terminal, an error or being dropped (#1542).
    pub(super) _pace: PaceGuard,
}

/// Caller result plus the narrow transmission evidence needed by source audit.
#[doc(hidden)]
pub struct EndpointOperationOutcome {
    /// The original caller result, without waiting for audit delivery.
    pub result: Result<EndpointOperationAck, Error>,
    /// At least one transport execution began, or a peer terminal was observed.
    /// This does not claim remote execution when the local transport failed.
    pub attempted: bool,
}

impl PreparedEndpointOperation {
    /// The exact reserved wire Invoke ID, stable across all retries.
    #[doc(hidden)]
    pub fn invoke_id(&self) -> u8 {
        self.guard.invoke_id
    }

    /// Execute under caller RAII ownership, or after session ownership transfer.
    #[doc(hidden)]
    pub async fn execute(mut self) -> EndpointOperationOutcome {
        let mut attempted = false;
        let result = self.execute_inner(&mut attempted).await.and_then(|bytes| {
            if bytes.len() + 3 > usize::from(self.guard.inner.max_apdu_length) {
                return Err(Error::Segmentation(
                    "endpoint operation ACK exceeds configured max APDU".into(),
                ));
            }
            self.request.decode(&bytes)
        });
        EndpointOperationOutcome { result, attempted }
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
            let send = inner.egress.admit_owned_apdu(
                self.encoded.clone(),
                self.destination.clone(),
                true,
                NetworkPriority::NORMAL,
                self.data_attributes.clone(),
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
