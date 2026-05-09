use super::*;

impl<T: TransportPort + 'static> BACnetServer<T> {
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
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
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
                let cov_in_flight = Arc::clone(cov_in_flight);
                let server_tsm = Arc::clone(server_tsm);
                let comm_state = Arc::clone(comm_state);
                let dcc_timer = Arc::clone(dcc_timer);
                let config = Arc::clone(config);
                let source_mac = MacAddr::from_slice(source_mac);
                tokio::spawn(async move {
                    Self::handle_confirmed_request(
                        &db,
                        &network,
                        &cov_table,
                        &seg_ack_senders,
                        &cov_in_flight,
                        &server_tsm,
                        &comm_state,
                        &dcc_timer,
                        &config,
                        &source_mac,
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
                if !tsm.record_result(&peer, sa.invoke_id, CovAckResult::Ack) {
                    tsm.record_result(&MacAddr::new(), sa.invoke_id, CovAckResult::Ack);
                }
                debug!(
                    invoke_id = sa.invoke_id,
                    "SimpleAck received for outgoing confirmed notification"
                );
            }
            Apdu::Error(err) => {
                let mut tsm = server_tsm.lock().await;
                let peer = MacAddr::from_slice(source_mac);
                if !tsm.record_result(&peer, err.invoke_id, CovAckResult::Error) {
                    tsm.record_result(&MacAddr::new(), err.invoke_id, CovAckResult::Error);
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
                if !tsm.record_result(&peer, rej.invoke_id, CovAckResult::Error) {
                    tsm.record_result(&MacAddr::new(), rej.invoke_id, CovAckResult::Error);
                }
                debug!(
                    invoke_id = rej.invoke_id,
                    "Reject received for outgoing confirmed notification"
                );
            }
            Apdu::Abort(abort) if !abort.sent_by_server => {
                let mut tsm = server_tsm.lock().await;
                let peer = MacAddr::from_slice(source_mac);
                if !tsm.record_result(&peer, abort.invoke_id, CovAckResult::Error) {
                    tsm.record_result(&MacAddr::new(), abort.invoke_id, CovAckResult::Error);
                }
                debug!(
                    invoke_id = abort.invoke_id,
                    "Abort received for outgoing confirmed notification"
                );
            }
            Apdu::SegmentAck(sa) => {
                let key = (MacAddr::from_slice(source_mac), sa.invoke_id);
                let senders = seg_ack_senders.lock().await;
                if let Some(tx) = senders.get(&key) {
                    let _ = tx.try_send(sa);
                } else {
                    debug!(
                        invoke_id = sa.invoke_id,
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
