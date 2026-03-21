//! Object database scale stress scenario.
//!
//! Steps through increasing object counts to measure how RP and RPM
//! latency scales with database size and track memory growth.

use std::time::{Duration, Instant};

use rand::RngExt;

use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{
    current_rss_kb, make_bip_server_with_db, make_large_db, make_stress_client,
};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

pub async fn run(_transport: &str, duration_secs: u64, steps: &[u64]) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();

    for &count in steps {
        eprintln!("--- {} objects ---", count);

        let db = make_large_db(1234, count as u32);
        let mut server = make_bip_server_with_db(db).await.unwrap();
        let server_mac = server.local_mac().to_vec();
        let mut client = make_stress_client().await.unwrap();

        let rss_before = current_rss_kb();
        let mut recorder = LatencyRecorder::new();
        let mut rng = rand::rng();
        let deadline = Instant::now() + Duration::from_secs(duration_secs);

        // Phase 1: Random ReadProperty
        while Instant::now() < deadline {
            let instance = rng.random_range(1..=count as u32);
            let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap();
            let start = Instant::now();
            match client
                .read_property(&server_mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
                .await
            {
                Ok(_) => recorder.record_success(start),
                Err(_) => recorder.record_failure(),
            }
        }

        // Phase 2: RPM batches of 10 random objects
        let rpm_count = 100u32.min(count as u32 / 10).max(1);
        let mut rpm_recorder = LatencyRecorder::new();
        for _ in 0..rpm_count {
            let specs: Vec<ReadAccessSpecification> = (0..10)
                .map(|_| {
                    let instance = rng.random_range(1..=count as u32);
                    ReadAccessSpecification {
                        object_identifier: ObjectIdentifier::new(
                            ObjectType::ANALOG_INPUT,
                            instance,
                        )
                        .unwrap(),
                        list_of_property_references: vec![PropertyReference {
                            property_identifier: PropertyIdentifier::PRESENT_VALUE,
                            property_array_index: None,
                        }],
                    }
                })
                .collect();

            let start = Instant::now();
            match client.read_property_multiple(&server_mac, specs).await {
                Ok(_) => rpm_recorder.record_success(start),
                Err(_) => rpm_recorder.record_failure(),
            }
        }

        let rss_after = current_rss_kb();
        let elapsed = duration_secs as f64;
        let throughput = recorder.successful() as f64 / elapsed;
        let stats = recorder.stats();
        let rpm_stats = rpm_recorder.stats();

        eprintln!(
            "  RP: ok={} err={} throughput={:.0}/s p50={}µs p99={}µs",
            recorder.successful(),
            recorder.failed(),
            throughput,
            stats.p50,
            stats.p99,
        );
        eprintln!(
            "  RPM: ok={} err={} p50={}µs p99={}µs",
            rpm_recorder.successful(),
            rpm_recorder.failed(),
            rpm_stats.p50,
            rpm_stats.p99,
        );
        eprintln!("  RSS: {}KB → {}KB", rss_before, rss_after);

        curve.push(DegradationPoint {
            parameter: count,
            p50_us: stats.p50,
            p99_us: stats.p99,
            throughput,
            errors: recorder.failed() + rpm_recorder.failed(),
        });

        let _ = client.stop().await;
        let _ = server.stop().await;
    }

    curve
}
