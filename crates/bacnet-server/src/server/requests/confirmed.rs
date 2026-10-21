use super::*;

#[cfg(test)]
#[path = "dcc_tests.rs"]
mod dcc_tests;

#[cfg(test)]
#[path = "reinitialize_device_tests.rs"]
mod reinitialize_device_tests;

#[cfg(test)]
impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Atomically admit a confirmed request before service decoding,
    /// authorization, mutation, side effects, or response construction. Runs
    /// with fresh DCC outcome and mutation decision logs, whatever `services`
    /// carries for them.
    pub(in crate::server) async fn handle_confirmed_request(
        services: &RequestServices<T>,
        confirmed_request_tracker: &Arc<ConfirmedRequestTracker>,
        request_tasks: &super::super::request_tasks::RequestTaskSpawner,
        source_mac: &[u8],
        source_network: Option<NpduAddress>,
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    ) {
        let RequestServices { network, .. } = services;
        let services = &Arc::new(RequestServices {
            dcc_outcomes: Arc::new(dcc_outcomes::DccOutcomes::default()),
            mutation_decisions: Arc::new(crate::mutation::MutationDecisions::default()),
            ..services.clone()
        });
        // LSO-only replay path mirrors dispatch admission (server level,
        // separate budget). Retransmitted already-executed LSO replays
        // byte-identically; pending in-flight duplicates discard.
        if req.service_choice == ConfirmedServiceChoice::LIFE_SAFETY_OPERATION {
            let lso_pending = match confirmed_request_tracker.lso.begin(
                source_mac,
                source_network.as_ref(),
                bacnet_transport::port::TransportProvenance::unverified(),
                req.clone(),
            ) {
                LsoAdmission::Replay(bytes) => {
                    confirmed_response::send_replay_bytes(
                        network,
                        &bytes,
                        source_mac,
                        source_network.as_ref(),
                        &bacnet_network::response_route::ResponseRoute::unverified(),
                        reply_tx,
                    )
                    .await;
                    return;
                }
                LsoAdmission::DuplicatePending => return,
                LsoAdmission::New(pending) => pending,
            };

            Self::handle_admitted_confirmed_request(
                services,
                request_tasks,
                RequestOrigin {
                    mac: source_mac,
                    network: source_network,
                    route: bacnet_network::response_route::ResponseRoute::new(
                        bacnet_transport::port::TransportProvenance::unverified(),
                        None,
                    ),
                },
                req,
                reply_tx,
                Some(ConfirmedRequestOwnership::LifeSafety(lso_pending)),
            )
            .await;
            return;
        }

        let pending = match confirmed_request_tracker.begin(
            source_mac,
            source_network.as_ref(),
            bacnet_transport::port::TransportProvenance::unverified(),
            req.clone(),
        ) {
            ConfirmedRequestAdmission::Duplicate => return,
            ConfirmedRequestAdmission::New(pending) => pending,
        };

        Self::handle_admitted_confirmed_request(
            services,
            request_tasks,
            RequestOrigin {
                mac: source_mac,
                network: source_network,
                route: bacnet_network::response_route::ResponseRoute::new(
                    bacnet_transport::port::TransportProvenance::unverified(),
                    None,
                ),
            },
            req,
            reply_tx,
            Some(ConfirmedRequestOwnership::Generic(pending)),
        )
        .await;
    }
}
