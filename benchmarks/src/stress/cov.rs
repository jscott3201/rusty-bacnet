//! COV subscription saturation stress scenario.
//!
//! Scales the number of COV subscriptions and measures notification
//! delivery latency under varying write rates.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_bip_server_with_db, make_stress_client};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

fn make_ao_db(device_instance: u32, count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        instance: device_instance,
        name: "COV Stress Device".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    for i in 1..=count {
        let ao = AnalogOutputObject::new(i, format!("AO-{}", i), 62).unwrap();
        db.add(Box::new(ao)).unwrap();
    }
    db
}

pub async fn run(duration_secs: u64, change_rate_hz: u64, steps: &[u64]) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();
    let max_subs = steps.iter().copied().max().unwrap_or(1) as u32;

    for &count in steps {
        eprintln!("--- {} COV subscriptions ---", count);

        let db = make_ao_db(1234, max_subs);
        let mut server = make_bip_server_with_db(db).await.unwrap();
        let server_mac = server.local_mac().to_vec();

        // Subscriber client
        let subscriber = make_stress_client().await.unwrap();
        let mut cov_rx = subscriber.cov_notifications();

        // Subscribe to COV on `count` objects
        for i in 1..=count as u32 {
            let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, i).unwrap();
            if let Err(e) = subscriber
                .subscribe_cov(&server_mac, 1, oid, false, Some(600))
                .await
            {
                eprintln!("  subscribe error on AO-{}: {}", i, e);
            }
        }

        // Writer client
        let writer = make_stress_client().await.unwrap();

        let notifications_received = Arc::new(AtomicU64::new(0));
        let notifications_rx = notifications_received.clone();
        let done = Arc::new(AtomicU64::new(0));
        let done_flag = done.clone();

        // Spawn notification counter
        let recv_handle = tokio::spawn(async move {
            let mut recorder = LatencyRecorder::new();
            loop {
                match tokio::time::timeout(Duration::from_millis(200), cov_rx.recv()).await {
                    Ok(Ok(_notif)) => {
                        notifications_rx.fetch_add(1, Ordering::Relaxed);
                        recorder.record_success(Instant::now());
                    }
                    Ok(Err(_)) => break,
                    Err(_) => {
                        if done_flag.load(Ordering::Relaxed) != 0 {
                            break;
                        }
                    }
                }
            }
            recorder
        });

        // Write values at change_rate_hz for duration
        let interval = 1_000_000u64
            .checked_div(change_rate_hz)
            .map(Duration::from_micros)
            .unwrap_or(Duration::from_secs(1));
        let deadline = Instant::now() + Duration::from_secs(duration_secs);
        let mut writes = 0u64;
        let mut write_errors = 0u64;
        let mut value = 100.0f32;

        let mut value_buf = bytes::BytesMut::new();

        while Instant::now() < deadline {
            // Write to a random subscribed object
            let instance = (writes % count + 1) as u32;
            let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance).unwrap();
            value += 1.0;

            value_buf.clear();
            bacnet_encoding::primitives::encode_app_real(&mut value_buf, value);
            let vd = value_buf.to_vec();

            match writer
                .write_property(
                    &server_mac,
                    oid,
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    vd,
                    Some(16),
                )
                .await
            {
                Ok(_) => writes += 1,
                Err(_) => write_errors += 1,
            }

            tokio::time::sleep(interval).await;
        }

        // Let notifications drain
        tokio::time::sleep(Duration::from_millis(500)).await;
        done.store(1, Ordering::Relaxed);

        let received = notifications_received.load(Ordering::Relaxed);
        // Expected: roughly `writes` notifications (one per write to a subscribed object)
        let expected = writes;

        drop(subscriber);
        let _recv_recorder = recv_handle.await.unwrap_or_default();

        eprintln!(
            "  writes={} write_errors={} notifications={}/{} (expected)",
            writes, write_errors, received, expected,
        );

        let throughput = received as f64 / duration_secs as f64;
        curve.push(DegradationPoint {
            parameter: count,
            p50_us: 0, // We track notification count rather than latency for COV
            p99_us: 0,
            throughput,
            errors: write_errors + (expected.saturating_sub(received)),
        });

        drop(writer);
        let _ = server.stop().await;
    }

    curve
}
