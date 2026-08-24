//! Unconfirmed-service dispatch (Who-Is, Who-Has, time sync, and
//! UnconfirmedTextMessage) — see `EXECUTED_UNCONFIRMED`.
//!
//! Split out of `requests.rs` to keep every file under the 700-LOC cap.

use super::super::*;

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
        comm_state: &Arc<AtomicU8>,
        req: UnconfirmedRequestPdu,
        received: &bacnet_network::layer::ReceivedApdu,
    ) {
        let comm = comm_state.load(Ordering::Acquire);
        if comm == 1 {
            tracing::debug!("Dropping unconfirmed service: DCC is DISABLE");
            return;
        }

        if req.service_choice == UnconfirmedServiceChoice::WHO_IS {
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
            if let Some(ref callback) = config.on_time_sync {
                let data = TimeSyncData {
                    raw_service_data: req.service_request.clone(),
                    is_utc: req.service_choice
                        == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
                };
                callback(data);
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
