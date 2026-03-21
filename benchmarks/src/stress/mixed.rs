//! Mixed realistic workload stress scenario.
//!
//! Simulates a building automation workload with weighted service mix:
//! 60% RP, 15% WP, 10% RPM, 5% COV sub/unsub, 5% WhoIs, 5% other.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use tokio::sync::Mutex;

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_bip_server_with_db, make_large_db, make_stress_client};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

fn make_mixed_db(device_instance: u32) -> ObjectDatabase {
    // Start with 500 AI objects from the standard helper
    let mut db = make_large_db(device_instance, 500);
    // Add 50 more writable AO objects (starting at instance 2 since make_large_db adds AO-1)
    for i in 2..=50 {
        let ao = AnalogOutputObject::new(i, format!("AO-{}", i), 62).unwrap();
        db.add(Box::new(ao)).unwrap();
    }
    db
}

pub async fn run(_transport: &str, clients: u64, duration_secs: u64) -> Vec<DegradationPoint> {
    eprintln!(
        "--- mixed workload: {} clients, {}s ---",
        clients, duration_secs
    );

    let db = make_mixed_db(1234);
    let mut server = make_bip_server_with_db(db).await.unwrap();
    let server_mac = server.local_mac().to_vec();

    let recorder = Arc::new(Mutex::new(LatencyRecorder::new()));
    let deadline = Instant::now() + Duration::from_secs(duration_secs);

    let mut handles = Vec::new();
    for _ in 0..clients {
        let mac = server_mac.clone();
        let rec = recorder.clone();
        handles.push(tokio::spawn(async move {
            let mut client = match make_stress_client().await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("  client error: {}", e);
                    return;
                }
            };

            let mut rng = StdRng::from_rng(&mut rand::rng());
            let mut cov_subscribed: Vec<u32> = Vec::new();

            while Instant::now() < deadline {
                let roll: u32 = rng.random_range(0..100);
                let start = Instant::now();

                let result = if roll < 60 {
                    // 60% ReadProperty
                    let instance = rng.random_range(1..=500u32);
                    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap();
                    client
                        .read_property(&mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
                        .await
                        .map(|_| ())
                } else if roll < 75 {
                    // 15% WriteProperty
                    let instance = rng.random_range(1..=50u32);
                    let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance).unwrap();
                    let value: f32 = rng.random_range(0.0..100.0);
                    let mut buf = bytes::BytesMut::new();
                    bacnet_encoding::primitives::encode_app_real(&mut buf, value);
                    client
                        .write_property(
                            &mac,
                            oid,
                            PropertyIdentifier::PRESENT_VALUE,
                            None,
                            buf.to_vec(),
                            Some(16),
                        )
                        .await
                } else if roll < 85 {
                    // 10% ReadPropertyMultiple (5 objects)
                    let specs: Vec<ReadAccessSpecification> = (0..5)
                        .map(|_| {
                            let instance = rng.random_range(1..=500u32);
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
                    client.read_property_multiple(&mac, specs).await.map(|_| ())
                } else if roll < 90 {
                    // 5% COV subscribe/unsubscribe cycle
                    if cov_subscribed.len() < 10 {
                        let instance = rng.random_range(1..=50u32);
                        let oid =
                            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance).unwrap();
                        let r = client.subscribe_cov(&mac, 100, oid, false, Some(300)).await;
                        if r.is_ok() {
                            cov_subscribed.push(instance);
                        }
                        r
                    } else {
                        let instance = cov_subscribed.pop().unwrap();
                        let oid =
                            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance).unwrap();
                        client.unsubscribe_cov(&mac, 100, oid).await
                    }
                } else if roll < 95 {
                    // 5% WhoIs
                    client.who_is(None, None).await
                } else {
                    // 5% ReadProperty on Device object
                    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
                    client
                        .read_property(&mac, oid, PropertyIdentifier::OBJECT_NAME, None)
                        .await
                        .map(|_| ())
                };

                match result {
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

    let curve = vec![DegradationPoint {
        parameter: clients,
        p50_us: stats.p50,
        p99_us: stats.p99,
        throughput,
        errors: rec.failed(),
    }];

    drop(rec);
    let _ = server.stop().await;

    curve
}
