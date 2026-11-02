//! Round-trip workloads over B/IP loopback, client and server in one
//! process: `rp` (ReadProperty of AI 1's present value), `wp`
//! (WriteProperty of 72.5 to AO 1 at priority 16) and `rpm`
//! (ReadPropertyMultiple of the present values of AI 1 to 10).
//!
//! `counts <rp|wp|rpm> <ct|mt> <n>` runs 200 warm-up requests, checks the
//! values the workload reads or writes, runs `n` counted requests, checks
//! them again and prints the counted loop's allocator totals. `ct` is
//! Tokio's current-thread runtime, `mt` its multi-thread runtime with two
//! workers.

use bacnet_client::client::BACnetClient;
use bacnet_counts::{AllocSnapshot, CountingAlloc, WARMUP, parse_n, report, setup, usage_exit};
use bacnet_services::rpm::ReadPropertyMultipleACK;
use bacnet_transport::bip::BipTransport;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const USAGE: &str = "counts <rp|wp|rpm> <ct|mt> <n>";

#[derive(Clone, Copy, PartialEq)]
enum Workload {
    Rp,
    Wp,
    Rpm,
}

/// What each request sends, built once outside the loop.
struct Requests {
    mac: Vec<u8>,
    ai1: ObjectIdentifier,
    ao1: ObjectIdentifier,
    /// 72.5 as an application-tagged REAL.
    write_value: Vec<u8>,
    rpm_specs: Vec<ReadAccessSpecification>,
}

fn real(value: f32) -> Vec<u8> {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, value);
    buf.to_vec()
}

fn analog_input(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).expect("valid AI identifier")
}

/// Runs `n` requests of `workload`, each arm with its own loop so a
/// request's future is exactly the client call's.
async fn run(client: &BACnetClient<BipTransport>, workload: Workload, r: &Requests, n: u64) {
    let pv = PropertyIdentifier::PRESENT_VALUE;
    match workload {
        Workload::Rp => {
            for _ in 0..n {
                client
                    .read_property(&r.mac, r.ai1, pv, None)
                    .await
                    .expect("ReadProperty");
            }
        }
        Workload::Wp => {
            for _ in 0..n {
                client
                    .write_property(&r.mac, r.ao1, pv, None, r.write_value.clone(), Some(16))
                    .await
                    .expect("WriteProperty");
            }
        }
        Workload::Rpm => {
            for _ in 0..n {
                client
                    .read_property_multiple(&r.mac, r.rpm_specs.clone())
                    .await
                    .expect("ReadPropertyMultiple");
            }
        }
    }
}

/// The value oracle: reads back what the workload reads or wrote and checks
/// identity and value. Runs outside the counted loop.
async fn verify(client: &BACnetClient<BipTransport>, workload: Workload, r: &Requests) {
    let pv = PropertyIdentifier::PRESENT_VALUE;
    match workload {
        Workload::Rp | Workload::Wp => {
            let (object, expected) = if workload == Workload::Rp {
                (r.ai1, real(21.0))
            } else {
                (r.ao1, r.write_value.clone())
            };
            let ack = client
                .read_property(&r.mac, object, pv, None)
                .await
                .expect("oracle ReadProperty");
            assert_eq!(ack.object_identifier, object);
            assert_eq!(ack.property_identifier, pv);
            assert_eq!(ack.property_array_index, None);
            assert_eq!(ack.property_value, expected);
        }
        Workload::Rpm => {
            let ack: ReadPropertyMultipleACK = client
                .read_property_multiple(&r.mac, r.rpm_specs.clone())
                .await
                .expect("oracle ReadPropertyMultiple");
            assert_eq!(ack.list_of_read_access_results.len(), 10);
            for (i, result) in (1..=10).zip(&ack.list_of_read_access_results) {
                assert_eq!(result.object_identifier, analog_input(i));
                let [element] = result.list_of_results.as_slice() else {
                    panic!(
                        "AI {i}: expected one result, got {:?}",
                        result.list_of_results
                    );
                };
                assert_eq!(element.property_identifier, pv);
                assert_eq!(element.property_array_index, None);
                assert_eq!(element.error, None);
                assert_eq!(element.property_value, Some(real(20.0 + i as f32)));
            }
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (workload, name) = match args.next().as_deref() {
        Some("rp") => (Workload::Rp, "rp"),
        Some("wp") => (Workload::Wp, "wp"),
        Some("rpm") => (Workload::Rpm, "rpm"),
        _ => usage_exit(USAGE),
    };
    let runtime_name = args.next().unwrap_or_default();
    let runtime = match runtime_name.as_str() {
        "ct" => tokio::runtime::Builder::new_current_thread(),
        "mt" => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.worker_threads(2);
            builder
        }
        _ => usage_exit(USAGE),
    }
    .enable_all()
    .build()
    .expect("Tokio runtime");
    let n = parse_n(args.next(), USAGE);

    runtime.block_on(async move {
        let mut server = setup::bip_server().await.expect("B/IP server");
        let mut client = setup::bip_client().await.expect("B/IP client");
        let requests = Requests {
            mac: server.local_mac().to_vec(),
            ai1: analog_input(1),
            ao1: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).expect("valid AO identifier"),
            write_value: real(72.5),
            rpm_specs: (1..=10)
                .map(|i| ReadAccessSpecification {
                    object_identifier: analog_input(i),
                    list_of_property_references: vec![PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    }],
                })
                .collect(),
        };

        run(&client, workload, &requests, WARMUP).await;
        verify(&client, workload, &requests).await;
        let start = AllocSnapshot::now();
        run(&client, workload, &requests, n).await;
        let counted = AllocSnapshot::now().since(start);
        report(name, &runtime_name, n, counted);
        verify(&client, workload, &requests).await;

        client.stop().await.expect("client stop");
        server.stop().await.expect("server stop");
    });
}
