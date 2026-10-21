//! Segmented RPM stress scenario.
//!
//! Sends RPM requests for increasing numbers of objects to force
//! segmented responses and measure reassembly performance.

use std::net::Ipv4Addr;
use std::time::Instant;

use bacnet_server::server::BACnetServer;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

use crate::stress::harness::{make_large_db_with, make_stress_client, stress_device_config};
use crate::stress::output::{DegradationPoint, LatencyRecorder};

pub async fn run(_transport: &str, steps: &[u64]) -> Vec<DegradationPoint> {
    let mut curve = Vec::new();
    let max_objects = steps.iter().copied().max().unwrap_or(10) as u32;

    for &count in steps {
        eprintln!("--- RPM {} objects ---", count);

        // The server and its Device both segment, so a reply longer than
        // the client's 1,476-octet APDU goes out in segments.
        let mut config = stress_device_config(1234);
        config.identity = config.identity.with_segmentation(Segmentation::BOTH);
        let mut server = BACnetServer::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .broadcast_address(Ipv4Addr::LOCALHOST)
            .device(config.identity.clone())
            .database(make_large_db_with(config, max_objects))
            .build()
            .await
            .unwrap();
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

        let started = Instant::now();
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

        // Requests per second over the requests' measured time.
        let point = DegradationPoint::from_recorder(count, &recorder, started.elapsed());
        point.log();
        eprintln!("  max={} µs", recorder.stats().max);
        curve.push(point);

        let _ = client.stop().await;
        let _ = server.stop().await;
    }

    curve
}
