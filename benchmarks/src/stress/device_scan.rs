//! Device scan stress scenario.
//!
//! Starts N B/IP servers on loopback, each its own device, and times one
//! client reading every Device's Object_Name in turn, as a supervisory tool
//! does once it knows the devices. It sends no Who-Is: on loopback each
//! server has its own UDP port, so a broadcast would reach only one of them.
//! The Docker orchestrator's discovery scenario broadcasts a Who-Is to many
//! devices on one network instead.

use std::time::Instant;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_bip_server_with_db, make_large_db, make_stress_client};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

pub async fn run(steps: &[u64]) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();

    for &count in steps {
        eprintln!("--- {} devices ---", count);

        // Start N servers with unique device instances
        let mut servers = Vec::new();
        let mut targets = Vec::new();
        for i in 0..count {
            let instance = 1000 + i as u32;
            let db = make_large_db(instance, 1);
            match make_bip_server_with_db(db).await {
                Ok(s) => {
                    targets.push((s.local_mac().to_vec(), instance));
                    servers.push(s);
                }
                Err(e) => {
                    eprintln!("  server {} error: {}", i, e);
                }
            }
        }

        let mut client = make_stress_client().await.unwrap();

        // Scan all devices: read OBJECT_NAME from each Device object
        let mut recorder = LatencyRecorder::new();
        let start = Instant::now();
        for (mac, instance) in &targets {
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
        // Devices per second over the scan's own time, which is often under
        // a millisecond: the rate keeps its full resolution.
        let point = DegradationPoint::from_recorder(count, &recorder, start.elapsed());
        eprintln!(
            "  scanned {}/{} devices in {:.3} ms",
            point.successful,
            targets.len(),
            point.elapsed_secs * 1000.0,
        );
        point.log();
        curve.push(point);

        // Cleanup
        let _ = client.stop().await;
        for mut s in servers {
            let _ = s.stop().await;
        }
    }

    curve
}
