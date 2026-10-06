//! Unconfirmed-service dispatch (Who-Is, Who-Has, time sync, text, event
//! notifications for the forwarders, Audit notification receipt and
//! WriteGroup) — see `EXECUTED_UNCONFIRMED`.
//!
//! Split out of `requests.rs` to keep every file under the 700-LOC cap.

use super::super::event_forwarding::{ForwardOrigin, Reception};
use super::super::received_event_log::log_received_event_notification;
use super::super::*;
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_objects::clock::ClockReader;
use bacnet_services::alarm_event::ForwardedEventNotification;
use bacnet_services::device_mgmt::TimeSynchronizationRequest;
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use std::panic::{catch_unwind, AssertUnwindSafe};

#[cfg(test)]
/// Every unconfirmed service choice with an inbound execution arm in
/// `handle_unconfirmed_request` below. Keep in lockstep with the dispatch
/// chain — see `EXECUTED_CONFIRMED` in `requests/mod.rs` for the cross-check
/// contract.
pub(crate) const EXECUTED_UNCONFIRMED: &[UnconfirmedServiceChoice] = &[
    UnconfirmedServiceChoice::WHO_IS,
    UnconfirmedServiceChoice::WHO_HAS,
    UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
    UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
    UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE,
    UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
    UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
    UnconfirmedServiceChoice::WRITE_GROUP,
];

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Handle an unconfirmed request (e.g., WhoIs).
    pub(in crate::server) async fn handle_unconfirmed_request(
        services: &UnconfirmedServices<T>,
        req: UnconfirmedRequestPdu,
        received: &bacnet_network::layer::ReceivedApdu,
    ) {
        let UnconfirmedServices {
            db,
            network,
            config,
            clock,
            comm_state,
            device_bindings,
            discovery_limiter,
            time_sync_limiter,
            notification_transactions,
            ..
        } = services;
        let clock = clock.as_ref();

        if req.service_choice == UnconfirmedServiceChoice::I_AM {
            let i_am = match IAmRequest::decode(&req.service_request) {
                Ok(request) => request,
                Err(e) => {
                    // A peer whose I-Am carries extra octets never binds, so
                    // say so where it can be seen (#1411).
                    warn!(error = %e, source = ?received.source_mac, "Failed to decode IAm");
                    return;
                }
            };
            let outcome = device_bindings.write().await.observe_i_am_at(
                i_am.object_identifier,
                &received.source_mac,
                received.source_network.as_ref(),
                runtime_clock::now(),
                // An I-Am from a group address binds nothing (#1493).
                |mac| network.transport().is_group_destination(mac),
            );
            match outcome {
                device_bindings::ObservationOutcome::RejectedInvalid => {
                    debug!("Ignoring unusable I-Am observation");
                }
                device_bindings::ObservationOutcome::RejectedCapacity => {
                    warn!("Device binding capacity reached; I-Am observation ignored");
                }
                device_bindings::ObservationOutcome::Inserted
                | device_bindings::ObservationOutcome::Refreshed
                | device_bindings::ObservationOutcome::ConfiguredPreserved => {}
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WHO_IS {
            let who_is = match WhoIsRequest::decode(&req.service_request) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Failed to decode WhoIs");
                    return;
                }
            };

            let db = db.read().await;
            let Ok(device_oid) =
                crate::local_device::validate_apdu_declaration(&db, config.max_apdu_length)
            else {
                warn!("I-Am refused: selected Device declaration differs from local acceptance");
                return;
            };

            if let Some(device_oid) = device_oid {
                let instance = device_oid.instance_number();

                let in_range = who_is.range.is_none_or(|range| range.contains(instance));

                if in_range {
                    let i_am = IAmRequest {
                        object_identifier: device_oid,
                        max_apdu_length: config.max_apdu_length,
                        segmentation_supported: config.segmentation_supported,
                        vendor_id: config.vendor_id,
                    };

                    let mut service_buf = BytesMut::new();
                    i_am.encode(&mut service_buf);

                    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                        service_choice: UnconfirmedServiceChoice::I_AM,
                        service_request: service_buf.freeze(),
                    });

                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                    let now = runtime_clock::now();
                    let is_unicast = !received.is_group && !received.link_layer_group;
                    let (res, directed) = if let Some(ref source_net) = received.source_network {
                        (
                            network
                                .send_apdu_routed(
                                    &buf,
                                    source_net.network,
                                    &source_net.mac_address,
                                    &received.source_mac,
                                    false,
                                    NetworkPriority::NORMAL,
                                )
                                .await,
                            true,
                        )
                    } else if config.discovery_policy.prefer_directed_responses || is_unicast {
                        (
                            network
                                .send_apdu(
                                    &buf,
                                    &received.source_mac,
                                    false,
                                    NetworkPriority::NORMAL,
                                )
                                .await,
                            true,
                        )
                    } else {
                        (
                            network
                                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                .await,
                            false,
                        )
                    };

                    if let Err(e) = res {
                        warn!(error = %e, directed, "Failed to send IAm");
                    } else {
                        discovery_limiter.record_i_am_sent(
                            buf.len(),
                            directed,
                            &received.source_mac,
                            received.source_network.as_ref(),
                            now,
                        );
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WHO_HAS {
            let who_has = match WhoHasRequest::decode(&req.service_request) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Failed to decode WhoHas");
                    return;
                }
            };

            let target = match &who_has.object {
                WhoHasObject::Identifier(oid) => WhoHasTarget::Id(*oid),
                WhoHasObject::Name(name) => WhoHasTarget::Name(name.clone()),
            };

            let now = runtime_clock::now();
            if discovery_limiter.is_negative_who_has(&target, now) {
                return;
            }

            let db = db.read().await;
            let device_oid = db.selected_device();

            if let Some(device_oid) = device_oid {
                match handlers::handle_who_has(&db, &req.service_request, device_oid) {
                    Ok(Some(i_have)) => {
                        let mut service_buf = BytesMut::new();
                        if let Err(e) = i_have.encode(&mut service_buf) {
                            warn!(error = %e, "Failed to encode IHave");
                        } else {
                            let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                                service_choice: UnconfirmedServiceChoice::I_HAVE,
                                service_request: service_buf.freeze(),
                            });

                            let mut buf = BytesMut::new();
                            encode_apdu(&mut buf, &pdu).expect("valid APDU encoding");

                            // Of the discovery answers, Clause 16.1.2 lets
                            // only an I-Am for a Who-Is out while initiation
                            // is disabled. Read the state after the last
                            // await before the send, and before the limiter
                            // records anything, so the same Who-Has is
                            // answered once initiation is enabled again.
                            if comm_state.initiation_restricted() {
                                debug!("I-Have held back: DCC restricts initiation");
                                return;
                            }

                            if !discovery_limiter.try_consume_who_has_response(
                                &target,
                                received,
                                buf.len(),
                                now,
                            ) {
                                return;
                            }

                            let is_unicast = !received.is_group && !received.link_layer_group;
                            let (res, directed) =
                                if let Some(ref source_net) = received.source_network {
                                    (
                                        network
                                            .send_apdu_routed(
                                                &buf,
                                                source_net.network,
                                                &source_net.mac_address,
                                                &received.source_mac,
                                                false,
                                                NetworkPriority::NORMAL,
                                            )
                                            .await,
                                        true,
                                    )
                                } else if config.discovery_policy.prefer_directed_responses
                                    || is_unicast
                                {
                                    (
                                        network
                                            .send_apdu(
                                                &buf,
                                                &received.source_mac,
                                                false,
                                                NetworkPriority::NORMAL,
                                            )
                                            .await,
                                        true,
                                    )
                                } else {
                                    (
                                        network
                                            .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                            .await,
                                        false,
                                    )
                                };

                            if let Err(e) = res {
                                warn!(error = %e, directed, "Failed to send IHave");
                            } else {
                                discovery_limiter.record_i_have_sent(
                                    buf.len(),
                                    directed,
                                    &target,
                                    &received.source_mac,
                                    received.source_network.as_ref(),
                                    now,
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        discovery_limiter.record_negative_who_has(target, now);
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to decode WhoHas");
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
            || req.service_choice == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
        {
            debug!("Received time synchronization request");
            let is_utc = req.service_choice == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION;
            if let Err(error) = apply_time_sync_request(
                clock.map(Arc::as_ref),
                config,
                time_sync_limiter,
                req.service_request.clone(),
                is_utc,
                received,
                network.local_network_number().get(),
            ) {
                debug!(%error, is_utc, "Ignoring time synchronization request");
            }
        } else if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_TEXT_MESSAGE {
            match handlers::handle_text_message(&req.service_request) {
                Ok(msg) => {
                    debug!(
                        source = ?msg.source_device,
                        priority = ?msg.message_priority,
                        "UnconfirmedTextMessage: {}",
                        msg.message
                    );
                }
                Err(e) => {
                    debug!(error = %e, "UnconfirmedTextMessage decode failed");
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION {
            match ForwardedEventNotification::decode(&req.service_request) {
                Ok(notification) => {
                    let source = CanonicalPeer::from_source(
                        &received.source_mac,
                        received.source_network.as_ref(),
                        network.local_network_number().get(),
                    );
                    log_received_event_notification(
                        db,
                        &services.received_event_log,
                        &services.event_suppressions,
                        source,
                        &req.service_request,
                    )
                    .await;
                    Self::forward_event_notification(
                        &services.event_delivery(),
                        vec![notification],
                        ForwardOrigin::Received(Reception::of(received)),
                    )
                    .await;
                }
                Err(error) => debug!(%error, "Ignoring malformed UnconfirmedEventNotification"),
            }
        } else if req.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION {
            match super::audit_notification::receive_unconfirmed_audit_notification(
                db,
                config,
                &received.source_mac,
                received.source_network.as_ref(),
                received.provenance,
                &req.service_request,
            )
            .await
            {
                Ok(Some(forward)) => forward.start(
                    network,
                    notification_transactions,
                    device_bindings,
                    config.max_apdu_length,
                ),
                Ok(None) => {}
                Err(error) => debug!(%error, "Ignoring UnconfirmedAuditNotification request"),
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WRITE_GROUP {
            Self::execute_write_group(services, &req.service_request, received).await;
        } else {
            debug!(
                service = req.service_choice.to_raw(),
                "Ignoring unsupported unconfirmed service"
            );
        }
    }
}

pub(super) fn apply_time_sync_request(
    clock: Option<&ServerClock>,
    config: &ServerConfig,
    limiter: &TimeSyncLimiter,
    raw_service_data: Bytes,
    is_utc: bool,
    received: &bacnet_network::layer::ReceivedApdu,
    local_network: Option<u16>,
) -> Result<(), Error> {
    let request = TimeSynchronizationRequest::decode(&raw_service_data)?;
    let supplied = clock::date_time_to_hundredths(request.date, request.time)?;
    let policy = &config.time_sync_policy;
    if !policy.enabled {
        return Err(time_sync_policy::denied("disabled"));
    }
    if policy
        .source_restriction
        .as_ref()
        .is_some_and(|restriction| {
            !restriction.allows(
                &received.source_mac,
                received.source_network.as_ref(),
                local_network,
            )
        })
    {
        return Err(time_sync_policy::denied("source not allowed"));
    }
    let clock = clock.ok_or_else(|| Error::Encoding("Device clock is disabled".into()))?;
    let mut delta_hundredths = None;
    let result = limiter.apply_at(received, local_network, runtime_clock::now(), || {
        delta_hundredths = clock
            .read_clock()
            .and_then(|frame| time_sync_policy::step_hundredths(supplied, is_utc, frame));
        time_sync_policy::check_step(delta_hundredths, policy.max_step)?;
        clock.synchronize(request.date, request.time, is_utc)
    });
    debug!(
        accepted = result.is_ok(),
        ?delta_hundredths,
        is_utc,
        "Time synchronization decision"
    );
    result?;

    if let Some(callback) = &config.on_time_sync {
        let data = TimeSyncData {
            raw_service_data,
            is_utc,
            source_mac: received.source_mac.clone(),
            source_network: received.source_network.clone(),
            provenance: received.provenance,
        };
        if catch_unwind(AssertUnwindSafe(|| callback(data))).is_err() {
            debug!(
                is_utc,
                "Time synchronization observer panicked after clock update"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "unconfirmed_time_sync_tests.rs"]
mod time_sync_tests;
