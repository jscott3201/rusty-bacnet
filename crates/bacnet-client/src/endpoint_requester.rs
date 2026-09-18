use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_encoding::apdu::{
    encode_apdu, validate_max_apdu_length, AbortPdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_endpoint_core::coordinator::{
    Admission, AdmissionKind, CanonicalPeer, OutboundTransactionCoordinator, TerminalPolicy,
};
use bacnet_endpoint_core::endpoint_ingress::{EndpointApduDestination, EndpointEgress};
use bacnet_network::layer::ReceivedApdu;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::port::{DataAttribute, TransportProvenance};
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

/// Derives the TSM key MAC for an outbound endpoint destination.
///
/// Direct destinations use the link MAC; routed destinations use the
/// `FF 52` synthetic key (`network || len || address`) so a routed peer
/// never collides with a direct peer that happens to share trailing bytes.
/// Mirrors `transaction_peer` without importing client internals.
fn outbound_tsm_peer(destination: &EndpointApduDestination) -> (MacAddr, CanonicalPeer) {
    match destination {
        EndpointApduDestination::Direct { destination_mac } => (
            destination_mac.clone(),
            CanonicalPeer::from_source(destination_mac.as_slice(), None),
        ),
        EndpointApduDestination::Routed {
            destination_network,
            destination_mac,
            ..
        } => (
            routed_tsm_mac(*destination_network, destination_mac.as_slice()),
            CanonicalPeer::routed(*destination_network, destination_mac.as_slice()),
        ),
        EndpointApduDestination::RoutedViaLocalBroadcast {
            destination_network,
            destination_mac,
        } => (
            routed_tsm_mac(*destination_network, destination_mac.as_slice()),
            CanonicalPeer::routed(*destination_network, destination_mac.as_slice()),
        ),
        EndpointApduDestination::LocalBroadcast
        | EndpointApduDestination::RemoteBroadcast { .. }
        | EndpointApduDestination::GlobalBroadcast => {
            (MacAddr::new(), CanonicalPeer::direct(&[] as &[u8]))
        }
    }
}

fn routed_tsm_mac(network: u16, mac: &[u8]) -> MacAddr {
    let mut key = MacAddr::new();
    key.extend_from_slice(&[0xFF, b'R']);
    key.extend_from_slice(&network.to_be_bytes());
    key.push(mac.len() as u8);
    key.extend_from_slice(mac);
    key
}

/// Derives the inbound TSM key + canonical peer for an admitted response.
///
/// RB-07 compat mode: provenance is preserved structurally by the caller
/// (threaded through `ReceivedApdu`) and never gates admission here.
fn inbound_tsm_peer(received: &ReceivedApdu) -> (MacAddr, CanonicalPeer) {
    match received.source_network.as_ref() {
        Some(address) if !address.mac_address.is_empty() => (
            routed_tsm_mac(address.network, address.mac_address.as_slice()),
            CanonicalPeer::from_source(received.source_mac.as_slice(), Some(address)),
        ),
        _ => (
            received.source_mac.clone(),
            CanonicalPeer::from_source(received.source_mac.as_slice(), None),
        ),
    }
}

fn reply_destination_for(received: &ReceivedApdu) -> EndpointApduDestination {
    match received.source_network.clone() {
        Some(address) if !address.mac_address.is_empty() => EndpointApduDestination::Routed {
            destination_network: address.network,
            destination_mac: address.mac_address,
            router_mac: received.source_mac.clone(),
        },
        _ => EndpointApduDestination::Direct {
            destination_mac: received.source_mac.clone(),
        },
    }
}

#[allow(dead_code)]
fn preserve_inbound_context(
    received: &ReceivedApdu,
) -> (
    bool,
    bool,
    Vec<DataAttribute>,
    TransportProvenance,
    Option<NpduAddress>,
    Option<u16>,
) {
    (
        received.link_layer_group,
        received.is_group,
        received.data_attributes.clone(),
        received.provenance,
        received.source_network.clone(),
        received.ingress_network,
    )
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
    ///
    /// Compat entry: direct unicast with no data attributes. Routed and
    /// attribute-preserving sends use
    /// [`Self::read_property_with_destination`].
    #[doc(hidden)]
    pub async fn read_property(
        &self,
        destination_mac: &[u8],
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        self.read_property_with_destination(
            EndpointApduDestination::Direct {
                destination_mac: MacAddr::from_slice(destination_mac),
            },
            Vec::new(),
            object_identifier,
            property_identifier,
            property_array_index,
        )
        .await
    }

    /// Performs one ReadProperty transaction to an explicit endpoint destination.
    ///
    /// RB-07 compat mode: `data_attributes` are passed through to
    /// [`EndpointEgress`] unchanged; no new policy decisions are made.
    #[doc(hidden)]
    pub async fn read_property_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        if !self.inner.open.load(Ordering::Acquire) {
            return Err(shutdown_error());
        }
        // Broadcast destinations never carry a confirmed request (§6.3 guard
        // lives in the egress path; fail fast here without allocating a lease).
        if matches!(
            destination,
            EndpointApduDestination::LocalBroadcast
                | EndpointApduDestination::RemoteBroadcast { .. }
                | EndpointApduDestination::GlobalBroadcast
        ) {
            return Err(Error::Encoding(
                "endpoint requester cannot send confirmed requests to a broadcast destination"
                    .into(),
            ));
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

        let (tsm_mac, peer) = outbound_tsm_peer(&destination);
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
                tsm_mac.clone(),
                peer,
                ConfirmedServiceChoice::READ_PROPERTY,
                false,
                TerminalPolicy::ComplexAck,
            )
            .map_err(|error| Error::Encoding(error.to_string()))?
        };

        let owner = registration.owner.clone();
        let mut guard = EndpointRequestGuard {
            inner: Arc::clone(&self.inner),
            destination: tsm_mac.clone(),
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
                .send_apdu(
                    encoded.clone(),
                    destination.clone(),
                    true,
                    NetworkPriority::NORMAL,
                    data_attributes.clone(),
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

    /// Performs one routed ReadProperty transaction via a known router.
    ///
    /// RB-07 compat mode: `data_attributes` pass through unchanged.
    #[doc(hidden)]
    pub async fn read_property_routed(
        &self,
        router_mac: &[u8],
        destination_network: u16,
        destination_mac: &[u8],
        data_attributes: Vec<DataAttribute>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<ReadPropertyACK, Error> {
        self.read_property_with_destination(
            EndpointApduDestination::Routed {
                destination_network,
                destination_mac: MacAddr::from_slice(destination_mac),
                router_mac: MacAddr::from_slice(router_mac),
            },
            data_attributes,
            object_identifier,
            property_identifier,
            property_array_index,
        )
        .await
    }

    /// Handles a response already admitted by the shared coordinator.
    ///
    /// Direct and routed responses are both accepted: the TSM key and
    /// canonical peer are derived from the received envelope
    /// (`source_mac` + `source_network`), so equal inbound/outbound numeric
    /// invoke IDs stay unambiguous via the classifier + coordinator admission.
    /// RB-07 compat mode: link-group, attributes, ingress-network and
    /// provenance are preserved structurally (threaded, never used for a new
    /// decision).
    #[doc(hidden)]
    pub async fn complete_pre_admitted(
        &self,
        admission: Admission,
        apdu: Apdu,
        received: ReceivedApdu,
    ) -> bool {
        if !self.inner.open.load(Ordering::Acquire) {
            return false;
        }
        // Structural preservation: bind every provenance/context field so a
        // future drop is a compile-visible change, not a silent regression.
        // No new decisions are made from these values here.
        let (_link_group, _is_group, _attributes, _provenance, _source_network, _ingress) =
            preserve_inbound_context(&received);
        let (tsm_mac, peer) = inbound_tsm_peer(&received);
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
                    tsm.complete_pre_admitted_terminal_response_for_peer(
                        &tsm_mac, &peer, &admission, &apdu, response,
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
                    tsm.reject_pre_admitted_segmented_response_for_peer(
                        &tsm_mac, &peer, &admission, &apdu,
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
                // Preserve the inbound envelope's routing + data attributes on
                // the abort (pass-through, no new decisions). Provenance and
                // link-group are already bound above for structural preservation.
                let destination = reply_destination_for(&received);
                let attributes = received.data_attributes.clone();
                self.inner
                    .egress
                    .send_apdu(
                        encoded.to_vec(),
                        destination,
                        false,
                        NetworkPriority::NORMAL,
                        attributes,
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
