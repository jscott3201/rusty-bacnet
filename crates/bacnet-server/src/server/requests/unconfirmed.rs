//! Unconfirmed-service dispatch (Who-Is, Who-Has, time sync, and
//! UnconfirmedTextMessage) — see `EXECUTED_UNCONFIRMED`.
//!
//! Split out of `requests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_services::device_mgmt::TimeSynchronizationRequest;

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
];

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Handle an unconfirmed request (e.g., WhoIs).
    pub(in crate::server) async fn handle_unconfirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        config: &ServerConfig,
        clock: Option<&Arc<ServerClock>>,
        comm_state: &Arc<AtomicU8>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        req: UnconfirmedRequestPdu,
        received: &bacnet_network::layer::ReceivedApdu,
    ) {
        let comm = comm_state.load(Ordering::Acquire);
        if comm == 1 {
            tracing::debug!("Dropping unconfirmed service: DCC is DISABLE");
            return;
        }

        if req.service_choice == UnconfirmedServiceChoice::I_AM {
            let i_am = match IAmRequest::decode(&req.service_request) {
                Ok(request) => request,
                Err(_) => {
                    debug!("Ignoring malformed I-Am observation");
                    return;
                }
            };
            let outcome = device_bindings.write().await.observe_i_am_at(
                i_am.object_identifier,
                &received.source_mac,
                received.source_network.as_ref(),
                Instant::now(),
                |mac| network.transport().is_broadcast_mac(mac),
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
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|oid| oid.object_type() == ObjectType::DEVICE);

            if let Some(device_oid) = device_oid {
                let instance = device_oid.instance_number();

                let in_range = match (who_is.low_limit, who_is.high_limit) {
                    (Some(low), Some(high)) => instance >= low && instance <= high,
                    _ => true,
                };

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

                    if let Some(ref source_net) = received.source_network {
                        if let Err(e) = network
                            .send_apdu_routed(
                                &buf,
                                source_net.network,
                                &source_net.mac_address,
                                &received.source_mac,
                                false,
                                NetworkPriority::NORMAL,
                            )
                            .await
                        {
                            warn!(error = %e, "Failed to route IAm back to remote requester");
                        }
                    } else if let Err(e) = network
                        .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                        .await
                    {
                        warn!(error = %e, "Failed to send IAm broadcast");
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WHO_HAS {
            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|oid| oid.object_type() == ObjectType::DEVICE);

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

                            if let Err(e) = network
                                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(error = %e, "Failed to send IHave broadcast");
                            }
                        }
                    }
                    Ok(None) => {}
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
                req.service_request.clone(),
                is_utc,
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
    raw_service_data: Bytes,
    is_utc: bool,
) -> Result<(), Error> {
    let clock = clock.ok_or_else(|| Error::Encoding("Device clock is disabled".into()))?;
    let request = TimeSynchronizationRequest::decode(&raw_service_data)?;
    clock.synchronize(request.date, request.time, is_utc)?;

    if let Some(callback) = &config.on_time_sync {
        callback(TimeSyncData {
            raw_service_data,
            is_utc,
        });
    }
    Ok(())
}
