//! The general handler's rarer services: every confirmed service but
//! ReadProperty, which has its own path (`read_property.rs`), and
//! WriteProperty, ReadPropertyMultiple and WritePropertyMultiple, whose arms
//! stay in [`BACnetServer::handle_admitted_confirmed_request`] (#2558).
//!
//! Each arm runs as it did in that handler's body, with the same checks in
//! the same order, and leaves the same effects for it to finish after the
//! response. The two debug lines that moved, for an unsupported service and
//! for a retransmitted event notification, now log under this module's and
//! `event_notification`'s targets, no longer `bacnet_server::server::requests`.
use super::audit_reporter::{ReadAuditIntent, WriteAudit};
use super::*;
use bacnet_device_core::event::AcceptedAcknowledgeAlarm;
use bacnet_services::alarm_event::ForwardedEventNotification;

#[cfg(test)]
#[path = "other_services_tests.rs"]
mod tests;

/// What one of these services reads: the request, where it came from, the
/// server's handles and the limits its response is built under.
pub(super) struct ServiceCall<'a, T: TransportPort + 'static> {
    pub(super) services: &'a Arc<RequestServices<T>>,
    pub(super) request_tasks: &'a super::super::request_tasks::RequestTaskSpawner,
    pub(super) req: &'a bacnet_encoding::apdu::ConfirmedRequest,
    pub(super) source_mac: &'a [u8],
    pub(super) source_network: &'a Option<NpduAddress>,
    /// This network's number as read for the request, the one its command
    /// origin and Audit source were matched against (#1404).
    pub(super) local_network: Option<u16>,
    pub(super) invoke_id: u8,
    pub(super) service_choice: ConfirmedServiceChoice,
    pub(super) effective_max_apdu: u16,
    pub(super) segmented_response_available: bool,
    pub(super) mutation: &'a mutations::Request<'a>,
}

// By hand: derived, they would ask for `T: Copy`.
impl<T: TransportPort + 'static> Clone for ServiceCall<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: TransportPort + 'static> Copy for ServiceCall<'_, T> {}

/// What these services leave for the handler to finish once the response
/// exists, and the audit their writes and reads report to.
pub(super) struct ServiceEffects<'a, 'w, T: TransportPort + 'static> {
    pub(super) effects: &'a mut MutationEffects,
    pub(super) initial_cov_notifications: &'a mut Vec<InitialCovNotification>,
    pub(super) accepted_acknowledgment: &'a mut Option<AcceptedAcknowledgeAlarm>,
    pub(super) received_event: &'a mut Option<ForwardedEventNotification>,
    pub(super) after_reply: &'a mut reinitialize::AfterReply,
    pub(super) read_audits: &'a mut Vec<ReadAuditIntent>,
    pub(super) audit: &'a mut WriteAudit<'w, T>,
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Execute `call`'s service and return its response, or `None` when it
    /// draws none: a retransmitted Audit notification already stored, which
    /// the handler then leaves unanswered.
    pub(super) async fn other_service_response(
        call: ServiceCall<'_, T>,
        out: ServiceEffects<'_, '_, T>,
        provenance: bacnet_transport::port::TransportProvenance,
        ack_buf: BytesMut,
    ) -> Option<Apdu> {
        let ServiceCall {
            services,
            request_tasks,
            req,
            source_mac,
            source_network,
            local_network,
            invoke_id,
            service_choice,
            effective_max_apdu,
            segmented_response_available,
            mutation,
        } = call;
        let ServiceEffects {
            effects,
            initial_cov_notifications,
            accepted_acknowledgment,
            received_event,
            after_reply,
            read_audits,
            audit,
        } = out;
        let RequestServices {
            db,
            cov_table,
            config,
            ..
        } = &**services;
        let response = match service_choice {
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV => {
                mutation
                    .subscribe_cov(db, cov_table, initial_cov_notifications)
                    .await
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY => {
                mutation
                    .subscribe_cov_property(db, cov_table, initial_cov_notifications)
                    .await
            }
            s if s == ConfirmedServiceChoice::CREATE_OBJECT => {
                mutation
                    .create_object::<T>(services, effects, ack_buf, audit)
                    .await
            }
            s if s == ConfirmedServiceChoice::DELETE_OBJECT => {
                mutation.delete_object::<T>(services, effects, audit).await
            }
            s if s == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL => {
                dcc::response(
                    services,
                    req,
                    super::super::dcc_timer::DccRequest {
                        service_data: &req.service_request,
                        source_mac,
                        source: source_network.as_ref(),
                        local_network,
                        provenance,
                        invoke_id,
                    },
                    request_tasks,
                    audit,
                )
                .await
            }
            s if s == ConfirmedServiceChoice::REINITIALIZE_DEVICE => {
                let requester =
                    reinitialize::Requester::new(source_mac, source_network.as_ref(), provenance);
                let (reply, hooks) =
                    reinitialize::server_response(services, req, requester, audit).await;
                *after_reply = hooks;
                reply
            }
            s if s == ConfirmedServiceChoice::GET_EVENT_INFORMATION => {
                event_information::response(
                    db,
                    req,
                    config.get_event_information_budget,
                    effective_max_apdu,
                    segmented_response_available,
                )
                .await
            }
            s if s == ConfirmedServiceChoice::ACKNOWLEDGE_ALARM => {
                acknowledge_alarm::response(db, req, accepted_acknowledgment).await
            }
            s if s == ConfirmedServiceChoice::READ_RANGE => {
                read_range::response(
                    db,
                    services.live_tables(),
                    req,
                    config,
                    services.execution,
                    (!segmented_response_available).then_some(effective_max_apdu),
                    |db, target, property, index, result| {
                        read_audits.extend(audit.completed_read_intent(
                            db,
                            target,
                            Some((property, index)),
                            result,
                        ));
                    },
                )
                .await
            }
            s if s == ConfirmedServiceChoice::ATOMIC_READ_FILE => {
                let db = db.read().await;
                Self::atomic_read_file_response(
                    &db,
                    invoke_id,
                    &req.service_request,
                    config.atomic_read_file_budget,
                    |target, result| {
                        read_audits.extend(audit.completed_read_intent(&db, target, None, result));
                    },
                )
            }
            s if s == ConfirmedServiceChoice::ATOMIC_WRITE_FILE => {
                mutation.atomic_write_file::<T>(db, audit).await
            }
            s if s == ConfirmedServiceChoice::ADD_LIST_ELEMENT
                || s == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT =>
            {
                let remove = s == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT;
                mutation
                    .list_element::<T>(services, effects, audit, remove)
                    .await
            }
            s if s == ConfirmedServiceChoice::GET_ALARM_SUMMARY => {
                let db = db.read().await;
                Self::alarm_summary_response(
                    &db,
                    invoke_id,
                    &req.service_request,
                    config.get_alarm_summary_budget,
                )
            }
            s if s == ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY => {
                let db = db.read().await;
                Self::enrollment_summary_response(
                    &db,
                    invoke_id,
                    &req.service_request,
                    config.get_enrollment_summary_budget,
                )
            }
            s if s == ConfirmedServiceChoice::AUDIT_LOG_QUERY => {
                audit_log_query::response(&call, ack_buf, read_audits, audit).await
            }
            s if s == ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION => {
                audit_notification::confirmed_response(&call, provenance).await?
            }
            // The sender's acknowledgment depends only on the request being
            // well formed; forwarding runs after the response and its outcome
            // never reaches the sender (Clause 12.51). A retransmission of a
            // notification already received is answered again but not
            // offered to the forwarders again (#1259).
            s if s == ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION => {
                event_notification::response(&call, received_event)
            }
            s if s == ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE => {
                match handlers::handle_text_message(&req.service_request) {
                    Ok(_msg) => simple_ack(invoke_id, service_choice),
                    Err(e) => bacnet_device_core::reply::error_apdu(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::LIFE_SAFETY_OPERATION => {
                life_safety_operation::response(&call, provenance, effects).await
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE => {
                mutation
                    .subscribe_cov_property_multiple(db, cov_table, initial_cov_notifications)
                    .await
            }
            _ => {
                debug!(
                    service = service_choice.to_raw(),
                    "Unsupported confirmed service"
                );
                bacnet_device_core::reply::unrecognized_service(invoke_id)
            }
        };
        Some(response)
    }
}
