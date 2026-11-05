//! What a cheap Rust mutation policy costs per WriteProperty, asked on
//! Tokio's blocking pool or inline (#2049): the `counts` binary's `wp`
//! round trip (72.5 to AO 1 at priority 16, B/IP loopback, client and
//! server in one process), through a server with
//!
//! - `none`: no policy, as `counts wp`;
//! - `pool`: a cheap policy that keeps `ServerPolicy::decides_inline`'s
//!   default, so each request asks it on the blocking pool;
//! - `inline`: the same policy declared inline, asked on the request task.
//!
//! `policy_counts <none|pool|inline> <ct|mt> <n>` runs 200 warm-up
//! requests, checks the written value, runs `n` counted requests, checks it
//! again, and prints the harness's `RESULT` line for the counted loop
//! (workload `wp-none`, `wp-pool` or `wp-inline`) and then its wall time.
//! Run it under callgrind at two lengths, as `scripts/perf/perf_counts.py`
//! runs `counts`, for the instructions per request.
//!
//! It is an example, not a `counts` workload: it needs `decides_inline`,
//! and `perf_counts.py` builds every binary of the harness in both trees it
//! compares, so as a binary it would stop any baseline older than #2049
//! from building.

use std::net::Ipv4Addr;
use std::time::Instant;

use bacnet_client::client::BACnetClient;
use bacnet_counts::{AllocSnapshot, CountingAlloc, WARMUP, parse_n, report, setup, usage_exit};
use bacnet_objects::DeviceIdentity;
use bacnet_server::ServerPolicy;
use bacnet_server::mutation::MutationAuthorizationContext;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const USAGE: &str = "policy_counts <none|pool|inline> <ct|mt> <n>";

/// Allows writes from the local network only: a field test, which never
/// blocks, locks, calls back into the server or does I/O.
struct LocalWritesOnly {
    inline: bool,
}

impl ServerPolicy for LocalWritesOnly {
    fn decides_inline(&self) -> bool {
        self.inline
    }

    fn authorize_mutation(&self, context: &MutationAuthorizationContext) -> bool {
        context.source_network.is_none()
    }
}

/// `setup::bip_server`, with `policy` installed when there is one.
async fn bip_server(policy: Option<LocalWritesOnly>) -> Result<BACnetServer<BipTransport>, Error> {
    let builder = BACnetServer::bip_builder()
        .device(DeviceIdentity::new(setup::DEVICE_INSTANCE, 555)?)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(setup::benchmark_db());
    match policy {
        Some(policy) => builder.policy(policy).build().await,
        None => builder.build().await,
    }
}

fn real(value: f32) -> Vec<u8> {
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, value);
    buf.to_vec()
}

async fn run(client: &BACnetClient<BipTransport>, mac: &[u8], value: &[u8], n: u64) {
    let ao1 = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).expect("valid AO identifier");
    for _ in 0..n {
        client
            .write_property(
                mac,
                ao1,
                PropertyIdentifier::PRESENT_VALUE,
                None,
                value.to_vec(),
                Some(16),
            )
            .await
            .expect("WriteProperty");
    }
}

/// The value oracle, outside the counted loop: AO 1 reads back the value.
async fn verify(client: &BACnetClient<BipTransport>, mac: &[u8], value: &[u8]) {
    let ao1 = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).expect("valid AO identifier");
    let ack = client
        .read_property(mac, ao1, PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .expect("oracle ReadProperty");
    assert_eq!(ack.object_identifier, ao1);
    assert_eq!(ack.property_value, value);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (policy, name) = match args.next().as_deref() {
        Some("none") => (None, "wp-none"),
        Some("pool") => (Some(LocalWritesOnly { inline: false }), "wp-pool"),
        Some("inline") => (Some(LocalWritesOnly { inline: true }), "wp-inline"),
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
        let mut server = bip_server(policy).await.expect("B/IP server");
        let mut client = setup::bip_client().await.expect("B/IP client");
        let mac = server.local_mac().to_vec();
        let value = real(72.5);

        run(&client, &mac, &value, WARMUP).await;
        verify(&client, &mac, &value).await;
        let start = AllocSnapshot::now();
        let started = Instant::now();
        run(&client, &mac, &value, n).await;
        let elapsed = started.elapsed();
        let counted = AllocSnapshot::now().since(start);
        report(name, &runtime_name, n, counted);
        eprintln!(
            "WALL workload={name} runtime={runtime_name} n={n} ns={} ns_per_iter={:.1}",
            elapsed.as_nanos(),
            elapsed.as_nanos() as f64 / n as f64,
        );
        verify(&client, &mac, &value).await;

        client.stop().await.expect("client stop");
        server.stop().await.expect("server stop");
    });
}
