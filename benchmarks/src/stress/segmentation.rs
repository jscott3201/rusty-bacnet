//! Segmented RPM stress scenario.
//!
//! Sends RPM requests for increasing numbers of objects to force
//! segmented responses and measure reassembly performance.

use std::time::Instant;

use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_bip_server_with_db, make_large_db, make_stress_client};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

pub async fn run(_transport: &str, steps: &[u64]) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();
    let max_objects = steps.iter().copied().max().unwrap_or(10) as u32;

    for &count in steps {
        eprintln!("--- RPM {} objects ---", count);

        let db = make_large_db(1234, max_objects);
        let mut server = make_bip_server_with_db(db).await.unwrap();
        let server_mac = server.local_mac().to_vec();
        let mut client = make_stress_client().await.unwrap();

        let specs: Vec<ReadAccessSpecification> = (1..=count as u32)
            .map(|i| ReadAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, i).unwrap(),
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                }],
            })
            .collect();

        let iterations = 50u32;
        let mut recorder = LatencyRecorder::new();

        for _ in 0..iterations {
            let start = Instant::now();
            match client
                .read_property_multiple(&server_mac, specs.clone())
                .await
            {
                Ok(_) => recorder.record_success(start),
                Err(e) => {
                    eprintln!("  RPM error: {}", e);
                    recorder.record_failure();
                }
            }
        }

        let stats = recorder.stats();
        let throughput = recorder.successful() as f64
            / (stats.p50 as f64 * recorder.successful() as f64 / 1_000_000.0).max(0.001);

        eprintln!(
            "  ok={} err={} p50={}µs p99={}µs max={}µs",
            recorder.successful(),
            recorder.failed(),
            stats.p50,
            stats.p99,
            stats.max,
        );

        curve.push(DegradationPoint {
            parameter: count,
            p50_us: stats.p50,
            p99_us: stats.p99,
            throughput,
            errors: recorder.failed(),
        });

        let _ = client.stop().await;
        let _ = server.stop().await;
    }

    curve
}
