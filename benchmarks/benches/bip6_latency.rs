use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use bacnet_benchmarks::helpers::{current_rss_bytes, make_bip6_client, make_bip6_server};

fn bench_read_property_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let (mut server, mut client, server_mac) = rt.block_on(async {
        let server = make_bip6_server().await.unwrap();
        let client = make_bip6_client().await.unwrap();
        let mac = server.local_mac().to_vec();
        (server, client, mac)
    });

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let rss_before = current_rss_bytes();

    c.bench_function("bip6_read_property_latency", |b| {
        b.to_async(&rt).iter(|| async {
            client
                .read_property(&server_mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
                .await
                .unwrap();
        });
    });

    let rss_after = current_rss_bytes();
    eprintln!(
        "BIP6 RSS: before={}KB after={}KB delta={}KB",
        rss_before / 1024,
        rss_after / 1024,
        (rss_after as i64 - rss_before as i64) / 1024
    );

    rt.block_on(async {
        let _ = client.stop().await;
        let _ = server.stop().await;
    });
}

fn bench_write_property_latency(c: &mut Criterion) {
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

    c.bench_function("bip6_write_property_latency", |b| {
        b.to_async(&rt).iter(|| {
            let vd = value_data.clone();
            async {
                client
                    .write_property(
                        &server_mac,
                        oid,
                        PropertyIdentifier::PRESENT_VALUE,
                        None,
                        vd,
                        Some(16),
                    )
                    .await
                    .unwrap();
            }
        });
    });

    rt.block_on(async {
        let _ = client.stop().await;
        let _ = server.stop().await;
    });
}

fn bench_rpm_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let (mut server, mut client, server_mac) = rt.block_on(async {
        let server = make_bip6_server().await.unwrap();
        let client = make_bip6_client().await.unwrap();
        let mac = server.local_mac().to_vec();
        (server, client, mac)
    });

    let specs: Vec<ReadAccessSpecification> = (1..=10)
        .map(|i| ReadAccessSpecification {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, i).unwrap(),
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }],
        })
        .collect();

    c.bench_function("bip6_rpm_10_objects_latency", |b| {
        b.to_async(&rt).iter(|| {
            let s = specs.clone();
            async {
                client.read_property_multiple(&server_mac, s).await.unwrap();
            }
        });
    });

    rt.block_on(async {
        let _ = client.stop().await;
        let _ = server.stop().await;
    });
}

criterion_group!(
    benches,
    bench_read_property_latency,
    bench_write_property_latency,
    bench_rpm_latency
);
criterion_main!(benches);
