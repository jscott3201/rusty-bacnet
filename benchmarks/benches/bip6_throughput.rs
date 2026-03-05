use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use bacnet_benchmarks::helpers::{make_bip6_client, make_bip6_server};

fn bench_read_property_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let (mut server, mut client, server_mac) = rt.block_on(async {
        let server = make_bip6_server().await.unwrap();
        let client = make_bip6_client().await.unwrap();
        let mac = server.local_mac().to_vec();
        (server, client, mac)
    });

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let mut group = c.benchmark_group("bip6_read_property_throughput");

    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let client_ref = &client;
                let mac_ref = &server_mac;
                b.to_async(&rt).iter(|| async move {
                    for _ in 0..n {
                        client_ref
                            .read_property(mac_ref, oid, PropertyIdentifier::PRESENT_VALUE, None)
                            .await
                            .unwrap();
                    }
                });
            },
        );
    }

    group.finish();

    rt.block_on(async {
        let _ = client.stop().await;
        let _ = server.stop().await;
    });
}

fn bench_write_property_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let (mut server, mut client, server_mac) = rt.block_on(async {
        let server = make_bip6_server().await.unwrap();
        let client = make_bip6_client().await.unwrap();
        let mac = server.local_mac().to_vec();
        (server, client, mac)
    });

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf, 72.5);
    let value_data = value_buf.to_vec();

    let mut group = c.benchmark_group("bip6_write_property_throughput");

    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let client_ref = &client;
                let mac_ref = &server_mac;
                let vd_ref = &value_data;
                b.to_async(&rt).iter(|| async move {
                    for _ in 0..n {
                        client_ref
                            .write_property(
                                mac_ref,
                                oid,
                                PropertyIdentifier::PRESENT_VALUE,
                                None,
                                vd_ref.clone(),
                                Some(16),
                            )
                            .await
                            .unwrap();
                    }
                });
            },
        );
    }

    group.finish();

    rt.block_on(async {
        let _ = client.stop().await;
        let _ = server.stop().await;
    });
}

criterion_group!(
    benches,
    bench_read_property_throughput,
    bench_write_property_throughput
);
criterion_main!(benches);
