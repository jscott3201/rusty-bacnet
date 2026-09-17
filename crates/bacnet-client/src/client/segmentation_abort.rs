//! Client-originated Abort helpers for segmented reassembly (RB-07 split).
//!
//! Split from `segmentation.rs` to keep the 700-LOC file cap; no behavior
//! change. Provenance threading lives in the reassembly path, these helpers
//! only send the wire Abort and complete the local transaction.

use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Transmit an Abort this client originates.
    ///
    /// Every Abort a requesting BACnet-user sends carries `'server' = FALSE` —
    /// Clauses 5.4.4.1, 5.4.4.3 and 5.4.4.4 each spell it out — because the
    /// flag names the sender's role, not the error.
    pub(super) async fn send_client_abort(
        network: &Arc<NetworkLayer<T>>,
        reply_mac: &[u8],
        reply_network: &Option<NpduAddress>,
        invoke_id: u8,
        abort_reason: bacnet_types::enums::AbortReason,
    ) {
        let abort = Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason,
        });
        let mut buf = BytesMut::with_capacity(4);
        if let Err(e) = encode_apdu(&mut buf, &abort) {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to encode Abort");
            return;
        }
        if let Err(e) = Self::send_reply_apdu(network, &buf, reply_mac, reply_network).await {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to send Abort");
        }
    }

    /// Abort a reassembly in progress, telling both the peer and the caller.
    ///
    /// Clause 5.4.4.4 gives this same shape to every way SEGMENTED_CONF can
    /// end badly — `NewSegmentReceived_NoSpace` when local storage cannot
    /// retain a segment and `UnexpectedPDU_Received` for an inappropriate
    /// PDU. Both send a client-side BACnet-Abort-PDU (`server` = FALSE),
    /// deliver ABORT.indication locally, and return to IDLE; only `abort-reason`
    /// differs. The local ABORT.indication is the waiting caller, so the
    /// transaction is completed rather than left to time out.
    ///
    /// The caller is responsible for having removed the `seg_state` entry —
    /// that removal implements the return to IDLE.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn abort_reassembly(
        tsm: &Arc<Mutex<Tsm>>,
        network: &Arc<NetworkLayer<T>>,
        tsm_mac: &MacAddr,
        owner: &TransactionOwner,
        reply_mac: &MacAddr,
        reply_network: &Option<NpduAddress>,
        invoke_id: u8,
        reason: bacnet_types::enums::AbortReason,
    ) {
        Self::send_client_abort(network, reply_mac, reply_network, invoke_id, reason).await;
        tsm.lock().await.complete_transaction_for_owner(
            tsm_mac,
            invoke_id,
            owner,
            None,
            TsmResponse::Abort {
                reason: reason.to_raw(),
            },
        );
    }
}
