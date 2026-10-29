use super::*;

#[cfg(test)]
#[path = "dcc_disable_rate_tests.rs"]
mod rate_tests;

/// Answer one DeviceCommunicationControl request, which `request` carries
/// with where it came from, and `audit` reports the change it carries out
/// (#1387).
pub(super) async fn response<T: TransportPort + 'static>(
    services: &RequestServices<T>,
    req: &ConfirmedRequestPdu,
    request: super::super::dcc_timer::DccRequest<'_>,
    request_tasks: &super::super::request_tasks::RequestTaskSpawner,
    audit: &mut super::super::audit_reporter::WriteAudit<'_, T>,
) -> Apdu {
    let RequestServices {
        dcc_outcomes: outcomes,
        cov_table,
        ..
    } = services;
    let (source_mac, source) = (request.source_mac, request.source);
    let cov_resume = cov_table.read().await.timed().clone();
    let result =
        super::super::dcc_timer::replace(services, request, request_tasks, &cov_resume, audit)
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
