use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;

use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_tls::TlsWebSocket;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use bacnet_benchmarks::sc_helpers::*;

fn bench_sc_mtls_read_property_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let certs = generate_test_certs();
    let hub_vmac: [u8; 6] = [0xD0; 6];
    let server_vmac: [u8; 6] = [0xD1; 6];
    let client_vmac: [u8; 6] = [0xD2; 6];

    let (mut hub, mut server, mut client, server_mac) = rt.block_on(async {
        let (hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

        let server_transport = make_sc_transport_mtls(&url, &certs, server_vmac).await;
        let db = bacnet_benchmarks::helpers::make_benchmark_db(7890);
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

    let mut group = c.benchmark_group("sc_mtls_read_property_throughput");

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
        hub.stop().await;
    });
}

fn bench_sc_mtls_write_property_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let certs = generate_test_certs();
    let hub_vmac: [u8; 6] = [0xE0; 6];
    let server_vmac: [u8; 6] = [0xE1; 6];
    let client_vmac: [u8; 6] = [0xE2; 6];

    let (mut hub, mut server, mut client, server_mac) = rt.block_on(async {
        let (hub, url) = start_sc_hub_mtls(&certs, hub_vmac).await;

        let server_transport = make_sc_transport_mtls(&url, &certs, server_vmac).await;
        let db = bacnet_benchmarks::helpers::make_benchmark_db(7890);
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

    let mut group = c.benchmark_group("sc_mtls_write_property_throughput");

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
        hub.stop().await;
    });
}

criterion_group!(
    benches,
    bench_sc_mtls_read_property_throughput,
    bench_sc_mtls_write_property_throughput
);
criterion_main!(benches);
