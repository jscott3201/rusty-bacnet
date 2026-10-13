//! The router's own network-control consumer (#1175).
//!
//! Most network messages a router receives are for its routing table, and it
//! handles them inline. A Reject-Message-To-Network addressed to the router
//! itself also answers something the router's own side sent, so besides
//! updating the table it goes to the opt-in stream that
//! [`RouterOptions::network_control_receiver`](super::RouterOptions::network_control_receiver)
//! asks for. Its records are the [`ReceivedNetworkControl`] a non-router
//! [`NetworkLayer`](crate::layer::NetworkLayer) hands its own consumer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::port::ReceivedNpdu;

use super::BACnetRouter;
use crate::layer::{next_ingress_sequence, AdmissionSender, ReceivedNetworkControl};

/// Where network controls addressed to this router go.
#[derive(Default)]
pub(super) struct LocalControl {
    /// The opted-in consumer, if any.
    tx: Option<AdmissionSender<ReceivedNetworkControl>>,
    /// Source of [`ReceivedNetworkControl::ingress_sequence`].
    ingress_sequence: Arc<AtomicU64>,
}

impl LocalControl {
    pub(super) fn new(
        tx: Option<AdmissionSender<ReceivedNetworkControl>>,
        ingress_sequence: Arc<AtomicU64>,
    ) -> Self {
        Self {
            tx,
            ingress_sequence,
        }
    }

    /// Offer the control `received` carries to the consumer. Like the
    /// router's local APDU queue, a full or closed consumer drops the
    /// arriving control and never holds up routing.
    pub(super) fn deliver(&self, received: &ReceivedNpdu) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        // The engine parsed it already, and bacnet-encoding's decoder
        // accepts exactly what the engine's view does.
        let Ok(npdu) = decode_npdu(received.npdu.clone()) else {
            return;
        };
        let _ = tx.try_send(ReceivedNetworkControl {
            npdu,
            source_mac: received.source_mac.clone(),
            link_layer_group: received.link_layer_group,
            data_attributes: received.data_attributes.clone(),
            provenance: received.provenance,
            ingress_sequence: next_ingress_sequence(&self.ingress_sequence),
        });
    }
}

impl BACnetRouter {
    /// Sequence assigned to the most recent control offered to the receiver
    /// from [`RouterOptions::network_control_receiver`](super::RouterOptions::network_control_receiver)
    /// or [`RouterOptions::network_control_receiver_with_admission`](super::RouterOptions::network_control_receiver_with_admission),
    /// including one the receiver dropped. Saturates at `u64::MAX`.
    pub fn network_control_ingress_sequence(&self) -> u64 {
        self.network_control_ingress_sequence.load(Ordering::SeqCst)
    }
}
