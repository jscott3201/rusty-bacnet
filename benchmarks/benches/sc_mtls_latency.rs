use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_tls::TlsWebSocket;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use bacnet_benchmarks::helpers::current_rss_bytes;
use bacnet_benchmarks::sc_helpers::*;

fn bench_sc_mtls_read_property_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let certs = generate_test_certs();
    let hub_vmac = [0xA0; 6];
    let server_vmac = [0xA1; 6];
    let client_vmac = [0xA2; 6];

    let (mut hub, mut server, mut client, server_mac) = rt.block_on(async {
        let (hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

        let server_transport = make_sc_transport_mtls(&url, &certs, server_vmac).await;
        let db = bacnet_benchmarks::helpers::make_benchmark_db(6789);
        let server =
            bacnet_server::server::BACnetServer::<ScTransport<TlsWebSocket>>::generic_builder()
                .transport(server_transport)
                .database(db)
                .build()
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let client_transport = make_sc_transport_mtls(&url, &certs, client_vmac).await;
        let client =
            bacnet_client::client::BACnetClient::<ScTransport<TlsWebSocket>>::generic_builder()
                .transport(client_transport)
                .apdu_timeout_ms(5000)
                .build()
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mac = server.local_mac().to_vec();
        (hub, server, client, mac)
    });

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let rss_before = current_rss_bytes();

    c.bench_function("sc_mtls_read_property_latency", |b| {
        b.to_async(&rt).iter(|| async {
            client
                .read_property(&server_mac, oid, PropertyIdentifier::PRESENT_VALUE, None)
                .await
                .unwrap();
        });
    });

    let rss_after = current_rss_bytes();
    eprintln!(
        "SC mTLS RSS: before={}KB after={}KB delta={}KB",
        rss_before / 1024,
        rss_after / 1024,
        (rss_after as i64 - rss_before as i64) / 1024
    );

    rt.block_on(async {
        let _ = client.stop().await;
        let _ = server.stop().await;
        hub.stop().await;
    });
}

fn bench_sc_mtls_write_property_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let certs = generate_test_certs();
    let hub_vmac = [0xB0; 6];
    let server_vmac = [0xB1; 6];
    let client_vmac = [0xB2; 6];

    let (mut hub, mut server, mut client, server_mac) = rt.block_on(async {
        let (hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

        let server_transport = make_sc_transport_mtls(&url, &certs, server_vmac).await;
        let db = bacnet_benchmarks::helpers::make_benchmark_db(6789);
        let server =
            bacnet_server::server::BACnetServer::<ScTransport<TlsWebSocket>>::generic_builder()
                .transport(server_transport)
                .database(db)
                .build()
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let client_transport = make_sc_transport_mtls(&url, &certs, client_vmac).await;
        let client =
            bacnet_client::client::BACnetClient::<ScTransport<TlsWebSocket>>::generic_builder()
                .transport(client_transport)
                .apdu_timeout_ms(5000)
                .build()
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mac = server.local_mac().to_vec();
        (hub, server, client, mac)
    });

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf, 72.5);
    let value_data = value_buf.to_vec();

    c.bench_function("sc_mtls_write_property_latency", |b| {
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
        hub.stop().await;
    });
}

fn bench_sc_mtls_rpm_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let certs = generate_test_certs();
    let hub_vmac = [0xC0; 6];
    let server_vmac = [0xC1; 6];
    let client_vmac = [0xC2; 6];

    let (mut hub, mut server, mut client, server_mac) = rt.block_on(async {
        let (hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

        let server_transport = make_sc_transport_mtls(&url, &certs, server_vmac).await;
        let db = bacnet_benchmarks::helpers::make_benchmark_db(6789);
        let server =
            bacnet_server::server::BACnetServer::<ScTransport<TlsWebSocket>>::generic_builder()
                .transport(server_transport)
                .database(db)
                .build()
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let client_transport = make_sc_transport_mtls(&url, &certs, client_vmac).await;
        let client =
            bacnet_client::client::BACnetClient::<ScTransport<TlsWebSocket>>::generic_builder()
                .transport(client_transport)
                .apdu_timeout_ms(5000)
                .build()
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mac = server.local_mac().to_vec();
        (hub, server, client, mac)
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

    c.bench_function("sc_mtls_rpm_10_objects_latency", |b| {
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
        hub.stop().await;
    });
}

criterion_group!(
    benches,
    bench_sc_mtls_read_property_latency,
    bench_sc_mtls_write_property_latency,
    bench_sc_mtls_rpm_latency
);
criterion_main!(benches);
