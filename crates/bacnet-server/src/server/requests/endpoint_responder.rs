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

#[allow(dead_code)]
pub(super) struct EndpointResponder {
    db: Arc<RwLock<ObjectDatabase>>,
    egress: EndpointEgress,
    open: AtomicBool,
}

#[allow(dead_code)]
impl EndpointResponder {
    pub(super) fn new(db: Arc<RwLock<ObjectDatabase>>, egress: EndpointEgress) -> Self {
        Self {
            db,
            egress,
            open: AtomicBool::new(true),
        }
    }

    pub(super) async fn handle(&self, mut received: ReceivedApdu) -> Result<bool, Error> {
        if !self.open.load(Ordering::Acquire) {
            return Err(shutdown_error());
        }
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
                    Vec::new(),
                )
                .await?;
            return Ok(true);
        }
        self.egress
            .send_direct(
                encoded.to_vec(),
                received.source_mac,
                false,
                NetworkPriority::NORMAL,
            )
            .await?;
        Ok(true)
    }

    pub(super) fn close(&self) {
        self.open.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "endpoint_responder_tests.rs"]
mod tests;
