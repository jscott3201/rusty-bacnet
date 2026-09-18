use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bacnet_encoding::apdu::{decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_endpoint_core::endpoint_ingress::{EndpointApduDestination, EndpointEgress};
use bacnet_network::layer::ReceivedApdu;

use super::confirmed_response;
use super::*;

#[allow(dead_code)]
fn shutdown_error() -> Error {
    Error::Encoding("endpoint shutdown".into())
}

/// Composition-visible inbound responder (narrow service scope).
///
/// Handles `ReadProperty` + `Reject`/`Abort` + segmentation-`Abort` only;
/// full service parity is a later packet. Inbound transactions reuse the
/// wire invoke ID directly and NEVER allocate from the shared outbound
/// client ID pool, so equal inbound/outbound numeric IDs stay unambiguous
/// via the ingress classifier + coordinator admission.
#[doc(hidden)]
pub struct EndpointResponder {
    db: Arc<RwLock<ObjectDatabase>>,
    egress: EndpointEgress,
    open: AtomicBool,
}

impl EndpointResponder {
    #[doc(hidden)]
    pub fn new(db: Arc<RwLock<ObjectDatabase>>, egress: EndpointEgress) -> Self {
        Self {
            db,
            egress,
            open: AtomicBool::new(true),
        }
    }

    /// Handles one inbound request, preserving provenance structurally.
    ///
    /// RB-07 compat mode: `link_layer_group` (raw), `is_group` (effective),
    /// `data_attributes`, `ingress_network` and `provenance` are threaded
    /// through without new policy decisions; `data_attributes` are forwarded
    /// on the reply send instead of being dropped.
    #[doc(hidden)]
    pub async fn handle(&self, mut received: ReceivedApdu) -> Result<bool, Error> {
        if !self.open.load(Ordering::Acquire) {
            return Err(shutdown_error());
        }
        // Structural preservation: bind raw + effective group, attributes,
        // ingress identity and provenance so a future drop is compile-visible.
        let _link_layer_group = received.link_layer_group;
        let _ingress_network = received.ingress_network;
        let _provenance = received.provenance;
        let preserved_attributes = received.data_attributes.clone();
        if received.is_group {
            return Ok(false);
        }
        let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu.clone())? else {
            return Ok(false);
        };

        let invoke_id = request.invoke_id;
        let mut response = if request.segmented {
            Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            })
        } else if request.service_choice == ConfirmedServiceChoice::READ_PROPERTY {
            confirmed_response::read_property_response(&self.db, &request).await
        } else {
            Apdu::Reject(RejectPdu {
                invoke_id,
                reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
            })
        };

        let mut encoded = BytesMut::new();
        encode_apdu(&mut encoded, &response)?;
        if matches!(response, Apdu::ComplexAck(_))
            && encoded.len() > usize::from(request.max_apdu_length)
        {
            response = Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            });
            encoded.clear();
            encode_apdu(&mut encoded, &response)?;
        }

        if let Some(reply_tx) = received.reply_tx.take() {
            let apdu = encoded.freeze();
            let npdu = Npdu {
                is_network_message: false,
                expecting_reply: false,
                priority: NetworkPriority::NORMAL,
                destination: received.source_network,
                source: None,
                payload: apdu,
                ..Npdu::default()
            };
            let mut wrapped = BytesMut::new();
            encode_npdu(&mut wrapped, &npdu)?;
            let _ = reply_tx.send(wrapped.freeze());
            return Ok(true);
        }

        if let Some(source_network) = received.source_network {
            self.egress
                .send_apdu(
                    encoded.to_vec(),
                    EndpointApduDestination::Routed {
                        destination_network: source_network.network,
                        destination_mac: source_network.mac_address,
                        router_mac: received.source_mac,
                    },
                    false,
                    NetworkPriority::NORMAL,
                    preserved_attributes,
                )
                .await?;
            return Ok(true);
        }
        self.egress
            .send_apdu(
                encoded.to_vec(),
                EndpointApduDestination::Direct {
                    destination_mac: received.source_mac,
                },
                false,
                NetworkPriority::NORMAL,
                preserved_attributes,
            )
            .await?;
        Ok(true)
    }

    /// Internal close for the endpoint session owner only.
    ///
    /// Lifecycle control lives on the session owner; role handles expose no
    /// public lifecycle methods.
    #[doc(hidden)]
    pub fn close(&self) {
        self.open.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "endpoint_responder_tests.rs"]
mod tests;
