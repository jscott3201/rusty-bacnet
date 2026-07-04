use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
    async fn route_segmented_send_event(
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
        key: SegKey,
        event: SegmentedSendEvent,
    ) -> bool {
        let handle = {
            let senders = seg_ack_senders.lock().await;
            senders.get(&key).cloned()
        };

        let Some(handle) = handle else {
            return false;
        };

        match event {
            SegmentedSendEvent::Abort(abort) => {
                handle.send_control(SegmentedSendControlEvent::Abort(abort));
                true
            }
            SegmentedSendEvent::SegmentAck(ack) => {
                let invoke_id = ack.invoke_id;
                let seq = ack.sequence_number;
                if !handle.accepts_segment_ack(&ack) {
                    debug!(
                        invoke_id,
                        seq,
                        negative = ack.negative_ack,
                        "Ignoring SegmentAck outside current segmented-send window"
                    );
                    return true;
                }

                match handle.segment_ack_tx.try_send(ack) {
                    Ok(()) => true,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        warn!(
                            invoke_id,
                            seq, "Dropping SegmentAck because segmented-send queue is full"
                        );
                        true
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => false,
                }
            }
        }
    }

    /// Dispatch a received APDU.
    ///
    /// ConfirmedRequest and UnconfirmedRequest handlers are spawned as
    /// independent tasks so the dispatch loop can immediately process the
    /// next incoming APDU.  This allows the server to handle multiple
    /// client requests concurrently (e.g. concurrent ReadProperty via
    /// the RwLock on ObjectDatabase).
    ///
    /// Fast-path APDU types (SimpleAck, Error, Reject, Abort, SegmentAck)
    /// remain inline since they are sub-microsecond TSM lookups.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
        seg_send_permits: &Arc<Semaphore>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
        config: &Arc<ServerConfig>,
        source_mac: &[u8],
        apdu: Apdu,
        mut received: bacnet_network::layer::ReceivedApdu,
    ) {
        match apdu {
            Apdu::ConfirmedRequest(req) => {
                let reply_tx = received.reply_tx.take();
                let db = Arc::clone(db);
                let network = Arc::clone(network);
                let cov_table = Arc::clone(cov_table);
                let seg_ack_senders = Arc::clone(seg_ack_senders);
                let seg_send_permits = Arc::clone(seg_send_permits);
                let cov_in_flight = Arc::clone(cov_in_flight);
                let server_tsm = Arc::clone(server_tsm);
                let comm_state = Arc::clone(comm_state);
                let dcc_timer = Arc::clone(dcc_timer);
                let config = Arc::clone(config);
                let source_mac = MacAddr::from_slice(source_mac);
                let source_network = received.source_network.clone();
                tokio::spawn(async move {
                    Self::handle_confirmed_request(
                        &db,
                        &network,
                        &cov_table,
                        &seg_ack_senders,
                        &seg_send_permits,
                        &cov_in_flight,
                        &server_tsm,
                        &comm_state,
                        &dcc_timer,
                        &config,
                        &source_mac,
                        source_network,
                        req,
                        reply_tx,
                    )
                    .await;
                });
            }
            Apdu::UnconfirmedRequest(req) => {
                let db = Arc::clone(db);
                let network = Arc::clone(network);
                let config = Arc::clone(config);
                let comm_state = Arc::clone(comm_state);
                tokio::spawn(async move {
                    Self::handle_unconfirmed_request(
                        &db,
                        &network,
                        &config,
                        &comm_state,
                        req,
                        &received,
                    )
                    .await;
                });
            }
            // Fast paths — remain inline (sub-microsecond TSM lookups)
            Apdu::SimpleAck(sa) => {
                let mut tsm = server_tsm.lock().await;
                let peer = MacAddr::from_slice(source_mac);
                if !tsm.record_result(
                    &peer,
                    received.source_network.as_ref(),
                    sa.invoke_id,
                    CovAckResult::Ack,
                ) {
                    tsm.record_result(&MacAddr::new(), None, sa.invoke_id, CovAckResult::Ack);
                }
                debug!(
                    invoke_id = sa.invoke_id,
                    "SimpleAck received for outgoing confirmed notification"
                );
            }
            Apdu::Error(err) => {
                let mut tsm = server_tsm.lock().await;
                let peer = MacAddr::from_slice(source_mac);
                if !tsm.record_result(
                    &peer,
                    received.source_network.as_ref(),
                    err.invoke_id,
                    CovAckResult::Error,
                ) {
                    tsm.record_result(&MacAddr::new(), None, err.invoke_id, CovAckResult::Error);
                }
                debug!(
                    invoke_id = err.invoke_id,
                    error_class = err.error_class.to_raw(),
                    error_code = err.error_code.to_raw(),
                    "Error received for outgoing confirmed notification"
                );
            }
            Apdu::Reject(rej) => {
                let mut tsm = server_tsm.lock().await;
                let peer = MacAddr::from_slice(source_mac);
                if !tsm.record_result(
                    &peer,
                    received.source_network.as_ref(),
                    rej.invoke_id,
                    CovAckResult::Error,
                ) {
                    tsm.record_result(&MacAddr::new(), None, rej.invoke_id, CovAckResult::Error);
                }
                debug!(
                    invoke_id = rej.invoke_id,
                    "Reject received for outgoing confirmed notification"
                );
            }
            Apdu::Abort(abort) if !abort.sent_by_server => {
                let key = (
                    MacAddr::from_slice(source_mac),
                    received.source_network.clone(),
                    abort.invoke_id,
                );
                let routed_segmented = Self::route_segmented_send_event(
                    seg_ack_senders,
                    key,
                    SegmentedSendEvent::Abort(abort.clone()),
                )
                .await;

                let mut tsm = server_tsm.lock().await;
                let peer = MacAddr::from_slice(source_mac);
                if !tsm.record_result(
                    &peer,
                    received.source_network.as_ref(),
                    abort.invoke_id,
                    CovAckResult::Error,
                ) {
                    tsm.record_result(&MacAddr::new(), None, abort.invoke_id, CovAckResult::Error);
                }
                debug!(
                    invoke_id = abort.invoke_id,
                    routed_segmented,
                    "Abort received for outgoing confirmed notification or segmented response"
                );
            }
            Apdu::SegmentAck(sa) => {
                let invoke_id = sa.invoke_id;
                if sa.sent_by_server {
                    debug!(
                        invoke_id,
                        seq = sa.sequence_number,
                        "Server ignoring SegmentAck with server bit set"
                    );
                    return;
                }

                let key = (
                    MacAddr::from_slice(source_mac),
                    received.source_network.clone(),
                    invoke_id,
                );
                if !Self::route_segmented_send_event(
                    seg_ack_senders,
                    key,
                    SegmentedSendEvent::SegmentAck(sa),
                )
                .await
                {
                    debug!(
                        invoke_id,
                        "Server ignoring SegmentAck for unknown transaction"
                    );
                }
            }
            _ => {
                debug!("Server ignoring unhandled APDU type");
            }
        }
    }
}
