//! Mixed realistic workload stress scenario.
//!
//! Simulates a building automation workload with weighted service mix:
//! 60% RP, 15% WP, 10% RPM, 5% COV sub/unsub, 5% WhoIs, 5% other.

use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_bip_server_with_db, make_large_db, make_stress_client};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

fn make_mixed_db(device_instance: u32) -> ObjectDatabase {
    // Start with 500 AI objects from the standard helper
    let mut db = make_large_db(device_instance, 500);
    // Add 50 more writable AO objects (starting at instance 2 since make_large_db adds AO-1)
    for i in 2..=50 {
        let ao = AnalogOutputObject::new(i, format!("AO-{}", i), EngineeringUnits::DEGREES_CELSIUS)
            .unwrap();
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

    // Every client connects before the clock starts, and each records into
    // its own histogram, so neither setup nor a shared lock is measured.
    let mut pool = Vec::new();
    for _ in 0..clients {
        match make_stress_client().await {
            Ok(c) => pool.push(c),
            Err(e) => eprintln!("  client error: {}", e),
        }
    }

    let started = Instant::now();
    let deadline = started + Duration::from_secs(duration_secs);

    let mut handles = Vec::new();
    for mut client in pool {
        let mac = server_mac.clone();
        handles.push(tokio::spawn(async move {
            let mut recorder = LatencyRecorder::new();
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
                    client.who_is(None).await
                } else {
                    // 5% ReadProperty on Device object
                    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
                    client
                        .read_property(&mac, oid, PropertyIdentifier::OBJECT_NAME, None)
                        .await
                        .map(|_| ())
                };

                match result {
                    Ok(_) => recorder.record_success(start),
                    Err(_) => recorder.record_failure(),
                }
            }

            let ended = Instant::now();
            let _ = client.stop().await;
            (recorder, ended)
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        if let Ok(result) = h.await {
            results.push(result);
        }
    }
    let point = DegradationPoint::from_clients(clients, started, results);
    point.log();
    let curve = vec![point];

    let _ = server.stop().await;

    curve
}
