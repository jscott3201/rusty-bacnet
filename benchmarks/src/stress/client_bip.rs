//! Shared B/IP fixture qualification, setup, measurement and cleanup.
use super::client_adapter::{make_bip_server_with_db, make_large_db, make_stress_client};
use super::client_counters::{self, CounterDelta};
use super::client_measure::{Load, StepReport, drive, stop_clients};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use std::time::Duration;

pub async fn step(index: usize, object_count: u32, count: u64, duration: Duration) -> StepReport {
    let server = make_bip_server_with_db(make_large_db(1234, object_count)).await;
    run_on(server, index, count, Load::Window(duration)).await.0
}

/// One step against a server the caller built (or failed to build); the
/// stopped server comes back for counters read after the step.
pub async fn run_on(
    server: Result<BACnetServer<BipTransport>, Error>,
    index: usize,
    count: u64,
    load: Load,
) -> (StepReport, Option<BACnetServer<BipTransport>>) {
    let mut report = StepReport::new(index, count);
    let mut server = match server {
        Ok(server) => server,
        Err(error) => {
            report.diagnostics.fail("server_setup", None, error);
            report.finish();
            return (report, None);
        }
    };
    let mac = server.local_mac().to_vec();
    let mut clients = Vec::new();
    for index in 0..count {
        match make_stress_client().await {
            Ok(client) => clients.push(client),
            Err(error) => {
                report
                    .diagnostics
                    .fail("client_setup", Some(index as usize), error);
                break;
            }
        }
    }
    report.diagnostics.started_clients = clients.len();
    // Independently expected application-REAL 20.1, before timing and baselines.
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    if let Some(client) = clients.first() {
        match client
            .read_property(&mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
            .await
        {
            Ok(ack)
                if ack.object_identifier == oid
                    && ack.property_identifier == PropertyIdentifier::PRESENT_VALUE
                    && ack.property_array_index.is_none()
                    && ack.property_value == [0x44, 0x41, 0xa0, 0xcc, 0xcd] => {}
            Ok(_) => report.diagnostics.fail(
                "preflight",
                Some(0),
                "AI1 Present_Value differs from the independent 20.1 fixture",
            ),
            Err(error) => report.diagnostics.fail("preflight", Some(0), error),
        }
    }
    let before = client_counters::server(&server);
    if before.is_none() {
        report.diagnostics.fail(
            "server_counters",
            None,
            "running server transport unavailable",
        );
    }
    if report.diagnostics.lifecycle_failures.is_empty() {
        drive(clients, mac, load, client_counters::bip, &mut report).await;
        if let Some(after) = client_counters::server(&server) {
            let counters = CounterDelta::between(before.unwrap(), after);
            if !counters.invalid.is_empty() {
                report.diagnostics.fail(
                    "server_counters",
                    None,
                    "counter decreased or disappeared",
                );
            }
            report.diagnostics.server_counters = Some(counters);
        } else {
            report.diagnostics.fail(
                "server_counters",
                None,
                "running server transport unavailable",
            );
        }
    } else {
        stop_clients(&mut clients, &mut report).await;
    }
    if let Err(error) = server.stop().await {
        report.diagnostics.fail("server_stop", None, error);
    }
    report.finish();
    (report, Some(server))
}
