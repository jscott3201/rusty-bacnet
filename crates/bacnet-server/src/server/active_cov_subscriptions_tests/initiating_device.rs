//! COV notifications name the identity's Device as their initiator, as I-Am
//! and I-Have do, even once the application adds a Device of a lower
//! instance after start (#2217).
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;

/// Device 5: below the identity's 813, added once the server runs (startup
/// refuses a database whose lowest Device isn't the identity's).
fn lower_device() -> Box<DeviceObject> {
    Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(5, 0).unwrap(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    )
}

/// The process identifier and initiating Device of every COV notification
/// sent since the last call.
fn notifications(wire: &Wire) -> Vec<(u32, ObjectIdentifier)> {
    wire.sent
        .take()
        .into_iter()
        .map(|frame| {
            let npdu = decode_npdu(frame.npdu).unwrap();
            let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected an unconfirmed COV notification");
            };
            match request.service_choice {
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION => {
                    let cov = COVNotificationRequest::decode(&request.service_request).unwrap();
                    (
                        cov.subscriber_process_identifier,
                        cov.initiating_device_identifier,
                    )
                }
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE => {
                    let cov =
                        COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
                    (
                        cov.subscriber_process_identifier,
                        cov.initiating_device_identifier,
                    )
                }
                service => panic!("unexpected service {service:?}"),
            }
        })
        .collect()
}

async fn wait_for(wire: &Wire, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while wire.sent.len() < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("every COV notification sent");
}

#[tokio::test(start_paused = true)]
async fn cov_and_cov_multiple_name_the_identity_device_beside_a_lower_one() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    wire.server.db.write().await.add(lower_device()).unwrap();
    let lower = ObjectIdentifier::new(ObjectType::DEVICE, 5).unwrap();
    assert_eq!(wire.server.db.read().await.selected_device(), Some(lower));

    simple_ack(
        wire.send(&direct(), subscribe_cov(61, av(1), Some(false), None))
            .await,
    );
    simple_ack(
        wire.send(
            &direct(),
            subscribe_cov_property(av(1), (62, PV, None, None, Some(false), Some(300))),
        )
        .await,
    );
    simple_ack(
        wire.send(
            &direct(),
            subscribe_cov_property_multiple(
                63,
                false,
                Some((300, 1)),
                vec![(av(1), vec![plain(PV)])],
            ),
        )
        .await,
    );
    // The initial notifications, then the ones a Present_Value change fires.
    wait_for(&wire, 3).await;
    let mut seen = notifications(&wire);
    let mut write = BytesMut::new();
    WritePropertyRequest {
        object_identifier: av(1),
        property_identifier: PV,
        property_array_index: None,
        property_value: vec![0x44, 0x42, 0x20, 0x00, 0x00], // REAL 40.0
        priority: Some(8),
    }
    .encode(&mut write)
    .unwrap();
    simple_ack(
        wire.send(&direct(), (ConfirmedServiceChoice::WRITE_PROPERTY, write))
            .await,
    );
    wait_for(&wire, 3).await;
    seen.extend(notifications(&wire));

    let mut processes: Vec<u32> = seen.iter().map(|(process, _)| *process).collect();
    processes.sort_unstable();
    assert_eq!(processes, vec![61, 61, 62, 62, 63, 63]);
    for (process, initiator) in seen {
        assert_eq!(initiator, device(), "process {process}");
    }
    wire.server.stop().await.unwrap();
}

/// With no Device object at all, notifications still name the identity's
/// instance (813 here), never Device 0 or nothing.
#[tokio::test(start_paused = true)]
async fn cov_names_the_identity_instance_without_a_device_object() {
    let mut wire = Wire::start_with_devices(ServerConfig::default(), &[]).await;
    assert_eq!(wire.server.db.read().await.selected_device(), None);
    simple_ack(
        wire.send(&direct(), subscribe_cov(61, av(1), Some(false), None))
            .await,
    );
    simple_ack(
        wire.send(
            &direct(),
            subscribe_cov_property_multiple(
                63,
                false,
                Some((300, 1)),
                vec![(av(1), vec![plain(PV)])],
            ),
        )
        .await,
    );
    wait_for(&wire, 2).await;
    let mut seen = notifications(&wire);
    seen.sort_unstable_by_key(|(process, _)| *process);
    assert_eq!(seen, vec![(61, device()), (63, device())]);
    wire.server.stop().await.unwrap();
}
