use super::*;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use std::num::NonZeroUsize;

async fn batch(
    client: &BACnetClient<LoopbackTransport>,
    service: u8,
    count: usize,
    limit: Option<NonZeroUsize>,
) {
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap();
    let pid = PropertyIdentifier::DESCRIPTION;
    match service {
        0 => {
            let result = client
                .read_property_from_devices(
                    (0..count)
                        .map(|_| DeviceReadRequest {
                            device_instance: 10,
                            object_identifier: oid,
                            property_identifier: pid,
                            property_array_index: None,
                        })
                        .collect(),
                    limit,
                )
                .await;
            assert_eq!(result.len(), count);
        }
        1 => {
            let spec = bacnet_services::rpm::ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: vec![bacnet_services::common::PropertyReference {
                    property_identifier: pid,
                    property_array_index: None,
                }],
            };
            let requests = (0..count)
                .map(|_| DeviceRpmRequest {
                    device_instance: 10,
                    specs: vec![spec.clone()],
                })
                .collect();
            let result = client
                .read_property_multiple_from_devices(requests, limit)
                .await;
            assert_eq!(result.len(), count);
        }
        _ => {
            let result = client
                .write_property_to_devices(
                    (0..count)
                        .map(|_| DeviceWriteRequest {
                            device_instance: 10,
                            object_identifier: oid,
                            property_identifier: pid,
                            property_array_index: None,
                            property_value: vec![0x70],
                            priority: None,
                        })
                        .collect(),
                    limit,
                )
                .await;
            assert_eq!(result.len(), count);
        }
    }
}

#[tokio::test]
async fn batch_empty_default_and_positive_complete() {
    let (transport, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    for service in 0..3 {
        for limit in [None, NonZeroUsize::new(1), NonZeroUsize::new(4)] {
            timeout(Duration::from_secs(1), batch(&client, service, 0, limit))
                .await
                .unwrap();
            // Missing lookup errors also complete through each bounded stream.
            timeout(Duration::from_secs(1), batch(&client, service, 3, limit))
                .await
                .unwrap();
        }
    }
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn batch_limit_and_drop_release_all_active_leases() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut received = peer.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    client.add_device(10, &[2]).await.unwrap();
    for service in 0..3 {
        for (limit, active) in [
            (None, 32),
            (NonZeroUsize::new(1), 1),
            (NonZeroUsize::new(3), 3),
        ] {
            let mut pending = Box::pin(batch(&client, service, 35, limit));
            for _ in 0..active {
                tokio::select! {
                    _ = &mut pending => panic!("batch completed without replies"),
                    item = timeout(Duration::from_secs(1), received.recv()) => { item.unwrap().unwrap(); }
                }
            }
            assert_eq!(client.tsm.lock().await.coordinated_active_count(), active);
            tokio::select! {
                _ = &mut pending => panic!("batch completed without replies"),
                item = received.recv() => panic!("limit exceeded: {item:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            drop(pending);
            timeout(Duration::from_secs(1), async {
                while client.tsm.lock().await.coordinated_active_count() != 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(client.tsm.lock().await.pending_count(), 0);
            assert!(received.try_recv().is_err());
        }
    }
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
