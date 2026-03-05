//! Max concurrent clients stress scenario.
//!
//! Scales the number of concurrent BIP clients doing ReadProperty against
//! a single server to find the throughput ceiling and error threshold.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_bip_server_with_db, make_large_db, make_stress_client};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

pub async fn run(
    _transport: &str,
    object_count: u32,
    duration_secs: u64,
    steps: &[u64],
) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();

    for &count in steps {
        eprintln!("--- {} clients ---", count);

        let db = make_large_db(1234, object_count);
        let mut server = make_bip_server_with_db(db).await.unwrap();
        let server_mac = server.local_mac().to_vec();

        let recorder = Arc::new(Mutex::new(LatencyRecorder::new()));
        let deadline = Instant::now() + Duration::from_secs(duration_secs);
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

        let mut handles = Vec::new();
        for _ in 0..count {
            let mac = server_mac.clone();
            let rec = recorder.clone();
            handles.push(tokio::spawn(async move {
                let mut client = match make_stress_client().await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("  client spawn error: {}", e);
                        return;
                    }
                };

                while Instant::now() < deadline {
                    let start = Instant::now();
                    match client
                        .read_property(&mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
                        .await
                    {
                        Ok(_) => rec.lock().await.record_success(start),
                        Err(_) => rec.lock().await.record_failure(),
                    }
                }

                let _ = client.stop().await;
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let rec = recorder.lock().await;
        let elapsed = duration_secs as f64;
        let throughput = rec.successful() as f64 / elapsed;
        let stats = rec.stats();

        eprintln!(
            "  ok={} err={} throughput={:.0}/s p50={}µs p99={}µs",
            rec.successful(),
            rec.failed(),
            throughput,
            stats.p50,
            stats.p99,
        );

        curve.push(DegradationPoint {
            parameter: count,
            p50_us: stats.p50,
            p99_us: stats.p99,
            throughput,
            errors: rec.failed(),
        });

        drop(rec);
        let _ = server.stop().await;
    }

    curve
}
