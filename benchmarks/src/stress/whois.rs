//! WhoIs / device discovery stress scenario.
//!
//! Starts N BIP servers and measures how fast a client can scan all
//! devices via direct ReadProperty of OBJECT_NAME.
//!
//! Note: True broadcast WhoIs/IAm requires real network interfaces (Docker).
//! This local variant measures device scan throughput at scale.

use std::time::Instant;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_bip_server_with_db, make_large_db, make_stress_client};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

pub async fn run(_duration_secs: u64, steps: &[u64]) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();

    for &count in steps {
        eprintln!("--- {} devices ---", count);

        // Start N servers with unique device instances
        let mut servers = Vec::new();
        let mut server_macs = Vec::new();
        let mut device_instances = Vec::new();
        for i in 0..count {
            let instance = 1000 + i as u32;
            let db = make_large_db(instance, 1);
            match make_bip_server_with_db(db).await {
                Ok(s) => {
                    server_macs.push(s.local_mac().to_vec());
                    device_instances.push(instance);
                    servers.push(s);
                }
                Err(e) => {
                    eprintln!("  server {} error: {}", i, e);
                }
            }
        }
        let actual_servers = servers.len() as u64;

        // Create client
        let mut client = make_stress_client().await.unwrap();

        // Scan all devices: read OBJECT_NAME from each Device object
        let start = Instant::now();
        let mut recorder = LatencyRecorder::new();

        for (mac, instance) in server_macs.iter().zip(device_instances.iter()) {
            let oid = ObjectIdentifier::new(ObjectType::DEVICE, *instance).unwrap();
            let req_start = Instant::now();
            match client
                .read_property(mac, oid, PropertyIdentifier::OBJECT_NAME, None)
                .await
            {
                Ok(_) => recorder.record_success(req_start),
                Err(e) => {
                    eprintln!("  RP error for device {}: {}", instance, e);
                    recorder.record_failure();
                }
            }
        }

        let total_ms = start.elapsed().as_millis() as u64;
        let stats = recorder.stats();

        eprintln!(
            "  scanned={}/{} total={}ms p50={}µs p99={}µs errors={}",
            recorder.successful(),
            actual_servers,
            total_ms,
            stats.p50,
            stats.p99,
            recorder.failed(),
        );

        curve.push(DegradationPoint {
            parameter: count,
            p50_us: stats.p50,
            p99_us: stats.p99,
            throughput: if total_ms > 0 {
                recorder.successful() as f64 / (total_ms as f64 / 1000.0)
            } else {
                recorder.successful() as f64
            },
            errors: recorder.failed(),
        });

        // Cleanup
        let _ = client.stop().await;
        for mut s in servers {
            let _ = s.stop().await;
        }
    }

    curve
}
