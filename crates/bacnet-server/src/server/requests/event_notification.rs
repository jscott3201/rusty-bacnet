//! An admitted ConfirmedEventNotification: its acknowledgment, and once
//! that is sent, the received-event log and the forwarders. Moved out of the
//! general handler's body (#2558), which still calls the second part after
//! its response.
use super::super::event_forwarding::{ForwardOrigin, Reception};
use super::super::received_event_log::log_received_event_notification;
use super::other_services::ServiceCall;
use super::*;
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_services::alarm_event::ForwardedEventNotification;

/// Answer `call`'s ConfirmedEventNotification. A notification received for
/// the first time goes into `received_event`, for
/// [`BACnetServer::log_and_forward_received_event`] once the answer is sent.
pub(super) fn response<T: TransportPort + 'static>(
    call: &ServiceCall<'_, T>,
    received_event: &mut Option<ForwardedEventNotification>,
) -> Apdu {
    let ServiceCall {
        services,
        req,
        source_mac,
        source_network,
        invoke_id,
        service_choice,
        ..
    } = *call;
    let confirmed_event_repeats = &services.confirmed_event_repeats;
    match ForwardedEventNotification::decode(&req.service_request) {
        Ok(notification) => {
            if confirmed_event_repeats.first_receipt(
                super::super::request_peer::canonical_requester(
                    source_mac,
                    source_network.as_ref(),
                ),
                invoke_id,
                &req.service_request,
                runtime_clock::now,
            ) {
                *received_event = Some(notification);
            } else {
                debug!(
                    invoke_id,
                    "Retransmitted ConfirmedEventNotification answered, not forwarded again"
                );
            }
            simple_ack(invoke_id, service_choice)
        }
        // A syntax fault draws the Reject naming it, as the
        // client's does (#1446).
        Err(error) => bacnet_device_core::reply::error_apdu(
            invoke_id,
            service_choice,
            &error.into_request_reject(),
        ),
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Log `notification`, a ConfirmedEventNotification from `source_mac`
    /// received for the first time, and offer it to the forwarders: after
    /// its answer is sent.
    pub(super) async fn log_and_forward_received_event(
        services: &RequestServices<T>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        local_network: Option<u16>,
        req: &bacnet_encoding::apdu::ConfirmedRequest,
        notification: ForwardedEventNotification,
    ) {
        let RequestServices {
            db,
            event_suppressions,
            received_event_log,
            ..
        } = services;
        // Logged at its first receipt only, with no lock held (#1346).
        let source = CanonicalPeer::from_source(source_mac, source_network.as_ref(), local_network);
        let (log, suppressions) = (received_event_log, event_suppressions);
        log_received_event_notification(db, log, suppressions, source, &req.service_request).await;
        // The dispatch loop drops a confirmed request that arrives by
        // broadcast (Clause 5.4.5.1), so this one was addressed to this
        // device alone.
        Self::forward_event_notification(
            &services.event_delivery(),
            vec![notification],
            ForwardOrigin::Received(Reception::UNICAST),
        )
        .await;
    }
}
