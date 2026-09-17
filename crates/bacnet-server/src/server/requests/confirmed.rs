#[cfg(test)]
use super::*;

#[cfg(test)]
#[path = "dcc_tests.rs"]
mod dcc_tests;

#[cfg(test)]
#[path = "reinitialize_device_tests.rs"]
mod reinitialize_device_tests;

#[cfg(test)]
impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Atomically admit a confirmed request before DCC, service decoding,
    /// authorization, mutation, side effects, or response construction.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::server) async fn handle_confirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        seg_ack_senders: &Arc<segmented_send::SegmentedSendRegistry>,
        seg_send_permits: &Arc<Semaphore>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        notification_transactions: &Arc<NotificationTransactions>,
        confirmed_request_tracker: &Arc<ConfirmedRequestTracker>,
        device_bindings: &Arc<RwLock<DeviceBindingTable>>,
        comm_state: &Arc<AtomicU8>,
        dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
        config: &ServerConfig,
        request_tasks: &super::super::request_tasks::RequestTaskSpawner,
        source_mac: &[u8],
        source_network: Option<NpduAddress>,
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    ) {
        // LSO-only replay path mirrors dispatch admission (server level,
        // separate budget). Retransmitted already-executed LSO replays
        // byte-identically; pending in-flight duplicates discard.
        if req.service_choice == ConfirmedServiceChoice::LIFE_SAFETY_OPERATION {
            if comm_state.load(Ordering::Acquire) == 1 {
                return;
            }
            let lso_pending = match confirmed_request_tracker.lso.begin(
                source_mac,
                source_network.as_ref(),
                req.clone(),
            ) {
                LsoAdmission::Replay(bytes) => {
                    confirmed_response::send_replay_bytes(
                        network,
                        &bytes,
                        source_mac,
                        source_network.as_ref(),
                        reply_tx,
                    )
                    .await;
                    return;
                }
                LsoAdmission::DuplicatePending => return,
                LsoAdmission::New(pending) => pending,
            };

            Self::handle_admitted_confirmed_request_with_lso(
                db,
                network,
                cov_table,
                seg_ack_senders,
                seg_send_permits,
                cov_in_flight,
                server_tsm,
                notification_transactions,
                device_bindings,
                comm_state,
                dcc_timer,
                &Arc::new(dcc_outcomes::DccOutcomes::default()),
                &Arc::new(crate::mutation::MutationDecisions::default()),
                config,
                request_tasks,
                source_mac,
                source_network,
                bacnet_transport::port::TransportProvenance::unverified(),
                req,
                reply_tx,
                Some(lso_pending),
            )
            .await;
            return;
        }

        let pending =
            match confirmed_request_tracker.begin(source_mac, source_network.as_ref(), req.clone())
            {
                ConfirmedRequestAdmission::Duplicate => return,
                ConfirmedRequestAdmission::New(pending) => pending,
            };

        Self::handle_admitted_confirmed_request(
            db,
            network,
            cov_table,
            seg_ack_senders,
            seg_send_permits,
            cov_in_flight,
            server_tsm,
            notification_transactions,
            device_bindings,
            comm_state,
            dcc_timer,
            &Arc::new(dcc_outcomes::DccOutcomes::default()),
            &Arc::new(crate::mutation::MutationDecisions::default()),
            config,
            request_tasks,
            source_mac,
            source_network,
            bacnet_transport::port::TransportProvenance::unverified(),
            req,
            reply_tx,
        )
        .await;
        pending.complete();
    }
}
