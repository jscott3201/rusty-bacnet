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
use bacnet_services::read_range::{ReadRangeReply, ReadRangeRequest, ReadRangeValidation};
use bacnet_transport::port::{DataAttribute, TransportProvenance};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, NetworkPriority, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::BytesMut;

use crate::client::pacing::{self, PaceGuard, PaceKey, RequestPacer};
use crate::client::{
    check_routed_unicast, confirmed_response_result, new_coordinated_tsm, ClientConfig,
    TransactionPeer,
};
#[path = "endpoint_operation.rs"]
mod operation;
#[path = "endpoint_operation_request.rs"]
mod operation_request;
pub use operation::{EndpointOperationOutcome, PacedEndpointOperation, PreparedEndpointOperation};
pub use operation_request::{EndpointOperationAck, EndpointOperationRequest};
#[path = "endpoint_read_request.rs"]
mod read_request;
pub use read_request::{EndpointReadAck, EndpointReadRequest};

use crate::tsm::{CompletionOutcome, CoordinatedCompletion, TransactionOwner, Tsm, TsmResponse};

fn shutdown_error() -> Error {
    Error::Encoding("endpoint shutdown".into())
}

/// The transaction peer an outbound endpoint destination is keyed to, the
/// same way the standalone client keys its requests ([`TransactionPeer`]).
fn outbound_tsm_peer(destination: &EndpointApduDestination) -> TransactionPeer {
    TransactionPeer::of(match destination {
        EndpointApduDestination::Direct { destination_mac } => {
            CanonicalPeer::direct(destination_mac)
        }
        EndpointApduDestination::Routed {
            destination_network,
            destination_mac,
            ..
        }
        | EndpointApduDestination::RoutedViaLocalBroadcast {
            destination_network,
            destination_mac,
        } => CanonicalPeer::routed(*destination_network, destination_mac),
        EndpointApduDestination::LocalBroadcast
        | EndpointApduDestination::RemoteBroadcast { .. }
        | EndpointApduDestination::GlobalBroadcast => CanonicalPeer::direct(&[]),
    })
}

/// The pacing lane of a checked, localized destination: network plus MAC,
/// as the standalone client keys its own (#1542).
fn pace_key(destination: &EndpointApduDestination) -> PaceKey {
    match destination {
        EndpointApduDestination::Direct { destination_mac } => PaceKey::new(None, destination_mac),
        EndpointApduDestination::Routed {
            destination_network,
            destination_mac,
            ..
        }
        | EndpointApduDestination::RoutedViaLocalBroadcast {
            destination_network,
            destination_mac,
        } => PaceKey::new(Some(*destination_network), destination_mac),
        // Refused before pacing: a confirmed request names one device.
        EndpointApduDestination::LocalBroadcast
        | EndpointApduDestination::RemoteBroadcast { .. }
        | EndpointApduDestination::GlobalBroadcast => PaceKey::new(None, &[]),
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
    /// The minimum interval between confirmed requests to one destination,
    /// measured as the standalone client measures it (#1542).
    pacer: Arc<RequestPacer>,
}

impl Drop for EndpointRequesterInner {
    fn drop(&mut self) {
        self.open.store(false, Ordering::Release);
        if let Ok(tsm) = self.tsm.get_mut() {
            tsm.cancel_all_transactions();
        }
    }
}

/// Unsegmented RP/RR/RPM/WP requester attached to a shared endpoint.
#[doc(hidden)]
#[derive(Clone)]
pub struct EndpointRequester {
    inner: Arc<EndpointRequesterInner>,
}

impl EndpointRequester {
    /// Attaches requester state to endpoint egress and a device-wide coordinator.
    ///
    /// `config.min_request_interval_ms` paces confirmed requests to each
    /// destination as [`ClientConfig::min_request_interval_ms`] says; past
    /// [`MAX_MIN_REQUEST_INTERVAL_MS`](crate::client::MAX_MIN_REQUEST_INTERVAL_MS)
    /// fails here.
    #[doc(hidden)]
    pub fn new(
        egress: EndpointEgress,
        coordinator: Arc<OutboundTransactionCoordinator>,
        config: ClientConfig,
    ) -> Result<Self, Error> {
        validate_max_apdu_length(config.max_apdu_length)?;
        pacing::validate_interval_ms(config.min_request_interval_ms)?;
        let pacer = Arc::new(RequestPacer::new(Duration::from_millis(
            config.min_request_interval_ms,
        )));
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
                pacer,
            }),
        })
    }

    /// Performs one direct ReadProperty transaction.
    ///
    /// Direct unicast with no data attributes. Routed and
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
    /// `data_attributes` are passed through to
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
        self.prepare_read_property(
            destination,
            data_attributes,
            object_identifier,
            property_identifier,
            property_array_index,
        )
        .await?
        .execute()
        .await
        .result?
        .into_property()
    }

    /// Validate, pace and reserve the exact transaction before transferring
    /// ownership. Dropping the prepared operation releases only its own
    /// requester lease.
    #[doc(hidden)]
    pub async fn prepare_read_property(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        object_identifier: ObjectIdentifier,
        property_identifier: PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<PreparedEndpointOperation, Error> {
        self.prepare_read(
            destination,
            data_attributes,
            EndpointReadRequest::Property(ReadPropertyRequest {
                object_identifier,
                property_identifier,
                property_array_index,
            }),
        )
        .await
    }

    /// Validate/encode, pace, then reserve a lease for the supported read
    /// services.
    #[doc(hidden)]
    pub async fn prepare_read(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        request: EndpointReadRequest,
    ) -> Result<PreparedEndpointOperation, Error> {
        self.prepare_operation(
            destination,
            data_attributes,
            EndpointOperationRequest::Read(request),
        )
        .await
    }

    /// Prepare one WriteProperty on the same requester and lease pool.
    #[doc(hidden)]
    pub async fn prepare_write(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        request: bacnet_services::write_property::WritePropertyRequest,
    ) -> Result<PreparedEndpointOperation, Error> {
        self.prepare_operation(
            destination,
            data_attributes,
            EndpointOperationRequest::Write(request),
        )
        .await
    }

    /// [`Self::pace_operation`], then [`PacedEndpointOperation::prepare`].
    #[doc(hidden)]
    pub async fn prepare_operation(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        request: EndpointOperationRequest,
    ) -> Result<PreparedEndpointOperation, Error> {
        self.pace_operation(destination, data_attributes, request)
            .await?
            .prepare()
    }

    /// Validate the complete request, then wait until its destination's
    /// minimum interval lets a new confirmed request go (#1542).
    ///
    /// Nothing is reserved while it waits, so a caller that gives up then
    /// leaves no trace, and a waiting request holds no invoke ID from the
    /// device's shared pool. [`PacedEndpointOperation::prepare`] reserves
    /// the transaction. Retries of the prepared request reuse its turn.
    #[doc(hidden)]
    pub async fn pace_operation(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        request: EndpointOperationRequest,
    ) -> Result<PacedEndpointOperation, Error> {
        let destination = self.checked_destination(destination)?;
        let service_data = self.encode_operation(&request)?;
        let pace = self.inner.pacer.wait(pace_key(&destination)).await;
        Ok(PacedEndpointOperation {
            inner: Arc::clone(&self.inner),
            destination,
            data_attributes,
            request,
            service_data,
            pace,
        })
    }

    /// The destination a confirmed request goes to once it passes the
    /// one-device checks, localized to this endpoint's network.
    fn checked_destination(
        &self,
        destination: EndpointApduDestination,
    ) -> Result<EndpointApduDestination, Error> {
        if !self.inner.open.load(Ordering::Acquire) {
            return Err(shutdown_error());
        }
        // Broadcast destinations never carry a confirmed request (Clause 6.3).
        // The egress refuses one too, but only once a transaction is taken, so
        // fail fast here, before any lease or invoke ID is allocated.
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
        // A routed destination names one device, as a client's routed
        // confirmed request does (#1278): DNET 0 names no network, and DNET
        // 65535 or an empty DADR would turn the request into a broadcast.
        if let EndpointApduDestination::Routed {
            destination_network,
            destination_mac,
            ..
        }
        | EndpointApduDestination::RoutedViaLocalBroadcast {
            destination_network,
            destination_mac,
        } = &destination
        {
            check_routed_unicast(*destination_network, destination_mac.len())?;
        }
        // Once those checks pass on the destination as named, one routed on
        // the endpoint's own network goes as the local destination it is
        // (#1403). The transaction is keyed to the MAC it goes to. Its answer
        // comes from there with no SNET, or through a router with this
        // network as its SNET, and either completes it (#1465).
        let destination = destination.localized(self.inner.egress.local_network_number().get());
        // A direct destination, as named or as localized, that reaches a group
        // of nodes is a local broadcast (#1479).
        if let EndpointApduDestination::Direct { destination_mac } = &destination {
            if self.inner.egress.is_group_destination(destination_mac) {
                return Err(Error::Encoding(
                    "endpoint requester cannot send confirmed requests to a broadcast or group \
                     address"
                        .into(),
                ));
            }
        }
        Ok(destination)
    }

    /// Preflight without reserving a lease or submitting traffic.
    #[doc(hidden)]
    pub fn validate_operation(&self, request: &EndpointOperationRequest) -> Result<(), Error> {
        self.encode_operation(request).map(|_| ())
    }

    fn encode_operation(&self, request: &EndpointOperationRequest) -> Result<BytesMut, Error> {
        let mut service_data = BytesMut::new();
        request.encode(&mut service_data)?;
        if 4 + service_data.len() > usize::from(self.inner.max_apdu_length) {
            return Err(Error::Segmentation(
                "endpoint requester supports only unsegmented requests".into(),
            ));
        }
        Ok(service_data)
    }

    /// Perform an unsegmented ReadRange to an explicit destination, checking
    /// the acknowledgement as `validation` says.
    #[doc(hidden)]
    pub async fn read_range_with_destination(
        &self,
        destination: EndpointApduDestination,
        data_attributes: Vec<DataAttribute>,
        request: ReadRangeRequest,
        validation: ReadRangeValidation,
    ) -> Result<ReadRangeReply, Error> {
        self.prepare_read(
            destination,
            data_attributes,
            EndpointReadRequest::Range(request, validation),
        )
        .await?
        .execute()
        .await
        .result?
        .into_range()
    }

    /// Handles a response already admitted by the shared coordinator.
    ///
    /// Direct and routed responses are both accepted. The TSM key and
    /// canonical peer are the peer the admission matched
    /// ([`OutboundTransactionCoordinator::admit_from_source`]), not a second
    /// reading of the envelope: the endpoint's network number cannot change
    /// between the two, and a lease matched by its routed alias (#1465)
    /// completes under the key it was registered with. Equal inbound and
    /// outbound numeric invoke IDs stay unambiguous via the classifier and
    /// that admission.
    /// Link-group, attributes, ingress-network and
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
        let TransactionPeer {
            tsm_mac,
            canonical: peer,
        } = TransactionPeer::of(admission.metadata().peer().clone());
        match admission.kind() {
            AdmissionKind::Terminal => {
                let response = match &apdu {
                    Apdu::SimpleAck(_) => TsmResponse::SimpleAck,
                    Apdu::ComplexAck(ack) if !ack.segmented => TsmResponse::ComplexAck {
                        service_data: ack.service_ack.clone(),
                    },
                    Apdu::Error(error) => TsmResponse::from_error_pdu(error),
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
