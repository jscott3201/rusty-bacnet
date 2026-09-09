use super::*;

pub(super) async fn response<T: TransportPort + 'static>(
    timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
    comm_state: &Arc<AtomicU8>,
    outcomes: &dcc_outcomes::DccOutcomes,
    config: &ServerConfig,
    req: &ConfirmedRequestPdu,
    source_mac: &[u8],
    source: Option<&NpduAddress>,
) -> Apdu {
    let result = super::super::dcc_timer::replace(
        timer,
        comm_state,
        &req.service_request,
        &config.dcc_password,
        config.dcc_policy,
    )
    .await;
    // No await between validation failure/live commit and completion telemetry.
    // The counter and event happen before constructing either response.
    match result {
        Ok(metadata) => {
            outcomes.record(
                dcc_outcomes::DccOutcome::Accepted,
                metadata,
                req.invoke_id,
                source_mac,
                source,
            );
            Apdu::SimpleAck(SimpleAck {
                invoke_id: req.invoke_id,
                service_choice: req.service_choice,
            })
        }
        Err(failure) => {
            outcomes.record(
                failure.outcome,
                failure.metadata,
                req.invoke_id,
                source_mac,
                source,
            );
            BACnetServer::<T>::error_apdu_from_error(
                req.invoke_id,
                req.service_choice,
                &failure.error,
            )
        }
    }
}
