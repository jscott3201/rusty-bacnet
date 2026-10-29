//! Replies to inbound confirmed requests, separate from outgoing client TSM controls.
use super::*;
use bacnet_network::layer::IssuedApdu;
use bacnet_network::response_route::ResponseRoute;

pub(super) struct InboundReply {
    route: ResponseRoute,
    reply_tx: Option<oneshot::Sender<Bytes>>,
}
impl InboundReply {
    pub(super) fn new(route: ResponseRoute, reply_tx: Option<oneshot::Sender<Bytes>>) -> Self {
        Self { route, reply_tx }
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    pub(super) async fn send_confirmed_request_reject(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        invoke_id: u8,
        reject_reason: RejectReason,
    ) {
        let reject = Apdu::Reject(RejectPdu {
            invoke_id,
            reject_reason,
        });
        let mut buf = BytesMut::with_capacity(3);
        if let Err(e) = encode_apdu(&mut buf, &reject) {
            warn!(error = %e, reason = reject_reason.to_raw(), "Failed to encode Reject");
            return;
        }
        if let Err(e) =
            Self::send_received_reply_apdu(network, &buf, source_mac, source_network, reply).await
        {
            warn!(error = %e, reason = reject_reason.to_raw(), "Failed to send Reject");
        }
    }

    pub(super) async fn send_server_abort(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        reply: InboundReply,
        invoke_id: u8,
        abort_reason: bacnet_types::enums::AbortReason,
    ) {
        let abort = Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason,
        });
        let mut buf = BytesMut::with_capacity(3);
        if let Err(e) = encode_apdu(&mut buf, &abort) {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to encode server Abort");
            return;
        }
        if let Err(e) =
            Self::send_received_reply_apdu(network, &buf, source_mac, source_network, reply).await
        {
            warn!(error = %e, reason = abort_reason.to_raw(), "Failed to send server Abort");
        }
    }

    pub(super) async fn send_confirmed_cov_notification_response(
        network: &Arc<NetworkLayer<T>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        response: ConfirmedCOVNotificationResponse,
        reply: InboundReply,
    ) {
        let apdu = match response {
            ConfirmedCOVNotificationResponse::Ack => Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice,
            }),
            ConfirmedCOVNotificationResponse::Reject(reject_reason) => Apdu::Reject(RejectPdu {
                invoke_id,
                reject_reason,
            }),
            ConfirmedCOVNotificationResponse::NoResponse => return,
        };

        let mut buf = BytesMut::with_capacity(4);
        if let Err(e) = encode_apdu(&mut buf, &apdu) {
            warn!(error = %e, "Failed to encode response for COV notification");
            return;
        }
        if let Err(e) =
            Self::send_received_reply_apdu(network, &buf, source_mac, source_network, reply).await
        {
            warn!(error = %e, "Failed to send response for COV notification");
        }
    }

    pub(super) async fn send_received_reply_apdu(
        network: &Arc<NetworkLayer<T>>,
        buf: &[u8],
        reply_mac: &[u8],
        reply_network: &Option<NpduAddress>,
        reply: InboundReply,
    ) -> Result<(), Error> {
        let route = reply.route;
        let destination = reply_network
            .as_ref()
            .filter(|address| !address.mac_address.is_empty());
        if !route.requires_direct_response()
            && let Some(reply_tx) = reply.reply_tx
        {
            let npdu = network.encode_response_for_handoff(
                IssuedApdu {
                    apdu: buf,
                    next_hop: reply_mac,
                    destination,
                    expecting_reply: false,
                    priority: NetworkPriority::Normal,
                },
                &route,
            )?;
            if reply_tx.send(npdu.freeze()).is_ok() {
                return Ok(());
            }
        }
        if route.requires_original_route() {
            return network
                .send_response_apdu_on_issuance(
                    IssuedApdu {
                        apdu: buf,
                        next_hop: reply_mac,
                        destination,
                        expecting_reply: false,
                        priority: NetworkPriority::Normal,
                    },
                    &route,
                    || {},
                )
                .await;
        }

        Self::send_reply_apdu(network, buf, reply_mac, reply_network).await
    }
}
