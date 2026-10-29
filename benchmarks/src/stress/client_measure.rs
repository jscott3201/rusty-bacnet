//! One request clock and complete task accounting for every client transport.
use super::client_adapter::classify;
use super::client_counters::{CounterDelta, Snapshot};
use super::client_diagnostics::{Failures, StepDiagnostics};
use super::output::{DegradationPoint, LatencyRecorder};
use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use std::time::{Duration, Instant};

pub struct StepReport {
    pub point: Option<DegradationPoint>,
    pub diagnostics: StepDiagnostics,
}

impl StepReport {
    pub fn new(index: usize, count: u64) -> Self {
        Self {
            point: None,
            diagnostics: StepDiagnostics::new(index, count),
        }
    }
    pub fn finish(&mut self) {
        self.diagnostics
            .finish(self.point.as_ref().map_or(0, |p| p.errors));
    }
}

/// Stop owned participants even when only part of setup succeeded.
pub async fn stop_clients<T: TransportPort + 'static>(
    clients: &mut [BACnetClient<T>],
    report: &mut StepReport,
) {
    for (index, client) in clients.iter_mut().enumerate() {
        if let Err(error) = client.stop().await {
            report.diagnostics.fail("client_stop", Some(index), error);
        }
    }
}

pub async fn drive<T: TransportPort + 'static>(
    clients: Vec<BACnetClient<T>>,
    server_mac: Vec<u8>,
    duration: Duration,
    snapshot: fn(&T) -> Option<Snapshot>,
    report: &mut StepReport,
) {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    // Snapshot every client before the common clock, so setup is excluded.
    let prepared: Vec<_> = clients
        .into_iter()
        .map(|client| {
            let before = snapshot(client.transport());
            (client, before)
        })
        .collect();
    let started = Instant::now();
    let deadline = started + duration;
    let mut handles = Vec::new();
    for (mut client, before) in prepared {
        let mac = server_mac.clone();
        handles.push(tokio::spawn(async move {
            let mut recorder = LatencyRecorder::new();
            let mut failures = Failures::default();
            while Instant::now() < deadline {
                let start = Instant::now();
                match client
                    .read_property(&mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
                    .await
                {
                    Ok(_) => recorder.record_success(start),
                    Err(error) => {
                        let elapsed = start.elapsed();
                        recorder.record_failure();
                        failures.record(classify(&error), elapsed);
                    }
                }
            }
            // Include the last in-flight request, exclude snapshot and stop time.
            let ended = Instant::now();
            let counters = before
                .zip(snapshot(client.transport()))
                .map(|(a, b)| CounterDelta::between(a, b));
            let stop = client.stop().await;
            ClientResult {
                recorder,
                ended,
                failures,
                counters,
                stop,
            }
        }));
    }
    report.diagnostics.started_clients = handles.len();
    collect(handles, started, report).await;
}

struct ClientResult {
    recorder: LatencyRecorder,
    ended: Instant,
    failures: Failures,
    counters: Option<CounterDelta>,
    stop: Result<(), bacnet_types::error::Error>,
}

async fn collect(
    handles: Vec<tokio::task::JoinHandle<ClientResult>>,
    started: Instant,
    report: &mut StepReport,
) {
    let mut results = Vec::new();
    for (index, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(ClientResult {
                recorder,
                ended,
                failures,
                counters,
                stop,
            }) => {
                report.diagnostics.completed_clients.push(index);
                report.diagnostics.failures.merge(&failures);
                if let Some(counters) = counters {
                    if !counters.invalid.is_empty() {
                        report.diagnostics.fail(
                            "client_counters",
                            Some(index),
                            "counter decreased or disappeared",
                        );
                    }
                    report.diagnostics.client_counters.push(counters);
                }
                if let Err(error) = stop {
                    report.diagnostics.fail("client_stop", Some(index), error);
                }
                results.push((recorder, ended));
            }
            // A panicked worker owned its client: explicit stop cannot be proven.
            Err(error) => report.diagnostics.fail("join", Some(index), error),
        }
    }
    report.point = Some(DegradationPoint::from_clients(
        report.diagnostics.requested_clients,
        started,
        results,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_stay_out_of_success_histogram_and_tail_uses_loop_end() {
        let start = Instant::now();
        let mut recorder = LatencyRecorder::new();
        recorder.record_latency(Duration::from_micros(30));
        recorder.record_failure();
        let mut failures = Failures::default();
        failures.record(
            super::super::client_diagnostics::FailureKind::Timeout,
            Duration::from_secs(20),
        );
        let point =
            DegradationPoint::from_clients(1, start, [(recorder, start + Duration::from_secs(21))]);
        assert_eq!(point.p99_us, 30);
        assert_eq!(point.errors, 1);
        assert_eq!(point.elapsed_secs, 21.0);
        assert_eq!(failures.max_ns, 20_000_000_000);
    }
    fn completed(start: Instant, stop: Result<(), bacnet_types::error::Error>) -> ClientResult {
        let mut recorder = LatencyRecorder::new();
        recorder.record_latency(Duration::from_micros(20));
        ClientResult {
            recorder,
            ended: start + Duration::from_secs(12),
            failures: Failures::default(),
            counters: None,
            stop,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn actual_panicked_and_cancelled_workers_remain_invalid() {
        for cancel in [false, true] {
            let start = Instant::now();
            let good = tokio::spawn(async move { completed(start, Ok(())) });
            let bad = tokio::spawn(async move {
                if cancel {
                    std::future::pending::<()>().await;
                }
                panic!("injected worker panic");
            });
            if cancel {
                bad.abort();
            }
            let mut report = StepReport::new(0, 2);
            report.diagnostics.started_clients = 2;
            collect(vec![good, bad], start, &mut report).await;
            report.finish();
            assert!(!report.diagnostics.valid);
            assert_eq!(report.diagnostics.completed_clients, [0]);
            assert!(
                report
                    .diagnostics
                    .lifecycle_failures
                    .iter()
                    .any(|failure| failure.stage == "join" && failure.client == Some(1))
            );
            assert_eq!(report.point.unwrap().successful, 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stop_failure_is_visible_and_cleanup_time_is_not_measured() {
        let start = Instant::now();
        let handle = tokio::spawn(async move {
            let result = completed(
                start,
                Err(bacnet_types::error::Error::Transport(
                    std::io::ErrorKind::BrokenPipe.into(),
                )),
            );
            tokio::time::sleep(Duration::from_secs(100)).await;
            result
        });
        let mut report = StepReport::new(0, 1);
        report.diagnostics.started_clients = 1;
        collect(vec![handle], start, &mut report).await;
        report.finish();
        assert!(!report.diagnostics.valid);
        assert_eq!(
            report.diagnostics.lifecycle_failures[0].stage,
            "client_stop"
        );
        assert_eq!(report.point.unwrap().elapsed_secs, 12.0);
    }
}
