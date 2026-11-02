//! The B/IP clients step with the server's admission policy as a parameter,
//! and the most confirmed handlers it ran at once (#1979). Kept apart from
//! `client_bip`, which the #1979 historical bootstrap also compiles against
//! v0.13.0, whose server has neither hook.
use super::client_bip::run_on;
use super::client_measure::{Load, StepReport};
use super::harness::{make_large_db, stress_device_config};
use bacnet_server::server::{BACnetServer, RequestAdmissionPolicy};
use bacnet_transport::bip::BipTransport;
use bacnet_types::error::Error;
use std::net::Ipv4Addr;

const DEVICE_INSTANCE: u32 = 1234;

/// The server policy for a run: the default, with `max_confirmed_in_flight`
/// replaced when given (the recovery reserve of 4 stays inside it).
pub fn policy(max_confirmed_in_flight: Option<usize>) -> RequestAdmissionPolicy {
    let default = RequestAdmissionPolicy::default();
    RequestAdmissionPolicy {
        max_confirmed_in_flight: max_confirmed_in_flight.unwrap_or(default.max_confirmed_in_flight),
        ..default
    }
}

async fn server(
    object_count: u32,
    policy: RequestAdmissionPolicy,
) -> Result<BACnetServer<BipTransport>, Error> {
    BACnetServer::bip_builder()
        .device(stress_device_config(DEVICE_INSTANCE).identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .request_admission_policy(policy)
        .database(make_large_db(DEVICE_INSTANCE, object_count))
        .build()
        .await
}

/// One B/IP clients step on a fresh server with [`policy`]; the report
/// carries the server's peak of live ordinary confirmed handlers.
pub async fn step(
    index: usize,
    object_count: u32,
    count: u64,
    load: Load,
    max_confirmed_in_flight: Option<usize>,
) -> StepReport {
    let built = server(object_count, policy(max_confirmed_in_flight)).await;
    let (mut report, stopped) = run_on(built, index, count, load).await;
    report.diagnostics.confirmed_peak =
        stopped.map(|server| server.request_admission_confirmed_peak());
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_confirmed_limit_moves() {
        let default = RequestAdmissionPolicy::default();
        assert_eq!(policy(None), default);
        let raised = policy(Some(104));
        assert_eq!(raised.max_confirmed_in_flight, 104);
        assert_eq!(
            raised.confirmed_recovery_reserve,
            default.confirmed_recovery_reserve
        );
        assert_eq!(
            raised.max_confirmed_in_flight_per_peer,
            default.max_confirmed_in_flight_per_peer
        );
    }
}
