use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_encoding::apdu::{
    encode_apdu, validate_max_apdu_length, AbortPdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu,
};
use bacnet_endpoint_core::coordinator::{
    Admission, AdmissionKind, CanonicalPeer, OutboundTransactionCoordinator, TerminalPolicy,
};
use bacnet_endpoint_core::endpoint_ingress::EndpointEgress;
use bacnet_network::layer::ReceivedApdu;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, NetworkPriority, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::BytesMut;

use crate::client::{confirmed_response_result, new_coordinated_tsm, ClientConfig};
use crate::tsm::{CompletionOutcome, CoordinatedCompletion, TransactionOwner, Tsm, TsmResponse};

fn shutdown_error() -> Error {
    Error::Encoding("endpoint shutdown".into())
}

struct EndpointRequesterInner {
    egress: EndpointEgress,
    tsm: Mutex<Tsm>,
    open: AtomicBool,
    timeout: Duration,
    retries: u8,
    max_apdu_length: u16,
}

impl Drop for EndpointRequesterInner {
    fn drop(&mut self) {
        self.open.store(false, Ordering::Release);
        if let Ok(tsm) = self.tsm.get_mut() {
            tsm.cancel_all_transactions();
        }
    }
}

/// Direct, unsegmented ReadProperty requester attached to a shared endpoint.
#[doc(hidden)]
#[derive(Clone)]
pub struct EndpointRequester {
    inner: Arc<EndpointRequesterInner>,
}

impl EndpointRequester {
    /// Attaches requester state to endpoint egress and a device-wide coordinator.
    #[doc(hidden)]
    pub fn new(
        egress: EndpointEgress,
        coordinator: Arc<OutboundTransactionCoordinator>,
        config: ClientConfig,
    ) -> Result<Self, Error> {
        validate_max_apdu_length(config.max_apdu_length)?;
        let timeout = Duration::from_millis(config.apdu_timeout_ms);
        let retries = config.apdu_retries;
        let max_apdu_length = config.max_apdu_length;
        let tsm = new_coordinated_tsm(&config, coordinator);
        Ok(Self {
            inner: Arc::new(EndpointRequesterInner {
                egress,
                tsm: Mutex::new(tsm),
                open: AtomicBool::new(true),
                timeout,
                retries,
                max_apdu_length,
            }),
        })
    }

    /// Performs one direct ReadProperty transaction.
    #[doc(hidden)]
    pub async fn read_property(
        &self,
        destination_mac: &[u8],
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        if !self.inner.open.load(Ordering::Acquire) {
            return Err(shutdown_error());
        }

        let request = ReadPropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
        };
        let mut service_data = BytesMut::new();
        request.encode(&mut service_data);
        if 4 + service_data.len() > usize::from(self.inner.max_apdu_length) {
            return Err(Error::Segmentation(
                "endpoint requester supports only unsegmented ReadProperty".into(),
            ));
        }

        let destination = MacAddr::from_slice(destination_mac);
        let (invoke_id, registration) = {
            let mut tsm = self
                .inner
                .tsm
                .lock()
                .map_err(|_| Error::Encoding("endpoint requester state is poisoned".into()))?;
            if !self.inner.open.load(Ordering::Acquire) {
                return Err(shutdown_error());
            }
            tsm.register_coordinated_transaction_with_policy(
                destination.clone(),
                CanonicalPeer::direct(destination_mac),
                ConfirmedServiceChoice::READ_PROPERTY,
                false,
                TerminalPolicy::ComplexAck,
            )
            .map_err(|error| Error::Encoding(error.to_string()))?
        };

        let owner = registration.owner.clone();
        let mut guard = EndpointRequestGuard {
            inner: Arc::clone(&self.inner),
            destination: destination.clone(),
            invoke_id,
            owner,
            active: true,
        };
        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: self.inner.max_apdu_length,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: service_data.freeze(),
        });
        let mut encoded = BytesMut::new();
        encode_apdu(&mut encoded, &pdu)?;
        let encoded = encoded.to_vec();
        let mut response = registration.response;

        for attempt in 0..=self.inner.retries {
            if !self.inner.open.load(Ordering::Acquire) {
                return Err(shutdown_error());
            }
            self.inner
                .egress
                .send_direct(
                    encoded.clone(),
                    destination.clone(),
                    true,
                    NetworkPriority::NORMAL,
                )
                .await?;

            match tokio::time::timeout(self.inner.timeout, &mut response).await {
                Ok(Ok(response)) => {
                    guard.active = false;
                    let service_data = confirmed_response_result(response)?;
                    return ReadPropertyACK::decode(&service_data);
                }
                Ok(Err(_)) if !self.inner.open.load(Ordering::Acquire) => {
                    return Err(shutdown_error());
                }
                Ok(Err(_)) => {
                    return Err(Error::Encoding("TSM response channel closed".into()));
                }
                Err(_) if attempt < self.inner.retries => {}
                Err(_) => return Err(Error::Timeout(self.inner.timeout)),
            }
        }

        unreachable!("the inclusive retry loop always returns")
    }

    /// Handles a response already admitted by the shared coordinator.
    #[doc(hidden)]
    pub async fn complete_pre_admitted(
        &self,
        admission: Admission,
        apdu: Apdu,
        received: ReceivedApdu,
    ) -> bool {
        if !self.inner.open.load(Ordering::Acquire) || received.source_network.is_some() {
            return false;
        }
        match admission.kind() {
            AdmissionKind::Terminal => {
                let response = match &apdu {
                    Apdu::SimpleAck(_) => TsmResponse::SimpleAck,
                    Apdu::ComplexAck(ack) if !ack.segmented => TsmResponse::ComplexAck {
                        service_data: ack.service_ack.clone(),
                    },
                    Apdu::Error(error) => TsmResponse::Error {
                        class: error.error_class.to_raw() as u32,
                        code: error.error_code.to_raw() as u32,
                    },
                    Apdu::Reject(reject) => TsmResponse::Reject {
                        reason: reject.reject_reason.to_raw(),
                    },
                    Apdu::Abort(abort) => TsmResponse::Abort {
                        reason: abort.abort_reason.to_raw(),
                    },
                    Apdu::SegmentAck(_)
                    | Apdu::ConfirmedRequest(_)
                    | Apdu::UnconfirmedRequest(_)
                    | Apdu::ComplexAck(_) => return false,
                };
                let completion = self.inner.tsm.lock().ok().map(|mut tsm| {
                    tsm.complete_pre_admitted_terminal_response(
                        &received.source_mac,
                        &admission,
                        &apdu,
                        response,
                    )
                });
                matches!(
                    completion,
                    Some(CoordinatedCompletion::Completed(
                        CompletionOutcome::Delivered
                    ))
                )
            }
            AdmissionKind::NonTerminal => {
                let rejected = self.inner.tsm.lock().is_ok_and(|mut tsm| {
                    tsm.reject_pre_admitted_segmented_response(
                        &received.source_mac,
                        &admission,
                        &apdu,
                    )
                });
                if !rejected {
                    return false;
                }

                let abort = Apdu::Abort(AbortPdu {
                    sent_by_server: false,
                    invoke_id: admission.token().invoke_id(),
                    abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
                });
                let mut encoded = BytesMut::new();
                if encode_apdu(&mut encoded, &abort).is_err() {
                    return false;
                }
                self.inner
                    .egress
                    .send_direct(
                        encoded.to_vec(),
                        received.source_mac,
                        false,
                        NetworkPriority::NORMAL,
                    )
                    .await
                    .is_ok()
            }
        }
    }

    /// Cancels exact pending leases and rejects later requester work.
    #[doc(hidden)]
    pub fn close(&self) {
        if !self.inner.open.swap(false, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut tsm) = self.inner.tsm.lock() {
            tsm.cancel_all_transactions();
        }
    }
}

struct EndpointRequestGuard {
    inner: Arc<EndpointRequesterInner>,
    destination: MacAddr,
    invoke_id: u8,
    owner: TransactionOwner,
    active: bool,
}

impl Drop for EndpointRequestGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Ok(mut tsm) = self.inner.tsm.lock() {
            tsm.cancel_transaction_for_owner(&self.destination, self.invoke_id, &self.owner);
        }
    }
}

#[cfg(test)]
#[path = "endpoint_requester_tests.rs"]
mod tests;
