use super::*;

struct NeverStart;
impl TransportPort for NeverStart {
    async fn start(
        &mut self,
    ) -> Result<tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        panic!("invalid enrollment budget reached startup")
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

#[tokio::test]
async fn enrollment_summary_defaults_and_all_builders_validate_before_start() {
    let default = GetEnrollmentSummaryBudget::default();
    assert_eq!(
        (default.max_objects, default.max_service_ack_bytes),
        (4096, 16384)
    );
    assert_eq!(
        ServerConfig::default().get_enrollment_summary_budget,
        default
    );
    assert!(format!("{:?}", ServerConfig::default()).contains("get_enrollment_summary_budget"));
    GetEnrollmentSummaryBudget {
        max_objects: usize::MAX,
        max_service_ack_bytes: usize::MAX,
    }
    .validate()
    .unwrap();
    for budget in [
        GetEnrollmentSummaryBudget {
            max_objects: 0,
            ..default
        },
        GetEnrollmentSummaryBudget {
            max_service_ack_bytes: 0,
            ..default
        },
    ] {
        let direct = BACnetServer::start(
            ServerConfig {
                get_enrollment_summary_budget: budget,
                ..Default::default()
            },
            ObjectDatabase::new(),
            NeverStart,
        )
        .await
        .err();
        let generic = BACnetServer::generic_builder()
            .transport(NeverStart)
            .get_enrollment_summary_budget(budget)
            .build()
            .await
            .err();
        let bip = BACnetServer::bip_builder()
            .port(0)
            .get_enrollment_summary_budget(budget)
            .build()
            .await
            .err();
        for error in [direct, generic, bip] {
            assert!(
                matches!(error, Some(Error::Encoding(m)) if m.contains("enrollment_summary_max_"))
            );
        }
        #[cfg(feature = "sc-tls")]
        {
            let result = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(crate::server::sc_builder::test_tls_config())
                .device_uuid(crate::server::sc_builder::TEST_DEVICE_UUID)
                .get_enrollment_summary_budget(budget)
                .build()
                .await
                .err();
            assert!(
                matches!(result, Some(Error::Encoding(m)) if m.contains("enrollment_summary_max_"))
            );
        }
    }
}

#[tokio::test]
async fn enrollment_summary_segmented_complete_ack_or_whole_abort() {
    use bacnet_client::client::BACnetClient;
    use bacnet_objects::{analog::AnalogValueObject, notification_class::NotificationClass};
    for (objects, bytes, expected) in [
        (21, 260, None),
        (21, 259, Some(AbortReason::BUFFER_OVERFLOW)),
        (20, 260, Some(AbortReason::OUT_OF_RESOURCES)),
    ] {
        let mut database = ObjectDatabase::new();
        for instance in 1..=20 {
            database
                .add(Box::new(
                    AnalogValueObject::new(instance, format!("summary-{instance}"), 62).unwrap(),
                ))
                .unwrap();
        }
        database
            .add(Box::new(NotificationClass::new(0, "NC").unwrap()))
            .unwrap();
        let mut server = BACnetServer::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .database(database)
            .segmentation_supported(Segmentation::BOTH)
            .get_enrollment_summary_budget(GetEnrollmentSummaryBudget {
                max_objects: objects,
                max_service_ack_bytes: bytes,
            })
            .build()
            .await
            .unwrap();
        let mut client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .max_apdu_length(50)
            .build()
            .await
            .unwrap();
        let result = client
            .confirmed_request(
                server.local_mac(),
                ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
                &[9, 0],
            )
            .await;
        client.stop().await.unwrap();
        server.stop().await.unwrap();
        match expected {
            Some(expected) => assert!(
                matches!(result, Err(Error::Abort { reason }) if reason == expected.to_raw()),
                "{result:?}"
            ),
            None => {
                let ack = result.unwrap();
                assert_eq!(ack.len(), 260);
                assert_eq!(
                    bacnet_services::enrollment_summary::GetEnrollmentSummaryAck::decode(&ack)
                        .unwrap()
                        .entries
                        .len(),
                    20
                );
            }
        }
    }
}

#[tokio::test]
async fn enrollment_summary_default_object_budget_wire() {
    default_object_budget_wire(true).await;
}

#[tokio::test]
async fn enrollment_summary_default_noncandidate_budget_wire() {
    default_object_budget_wire(false).await;
}

async fn default_object_budget_wire(candidate: bool) {
    use bacnet_client::client::BACnetClient;
    use bacnet_objects::analog::AnalogValueObject;

    let mut database = ObjectDatabase::new();
    for instance in 1..=4097 {
        if candidate {
            database
                .add(Box::new(
                    AnalogValueObject::new(instance, format!("summary-{instance}"), 62).unwrap(),
                ))
                .unwrap();
        } else {
            database
                .add(Box::new(
                    bacnet_objects::notification_class::NotificationClass::new(
                        instance,
                        format!("class-{instance}"),
                    )
                    .unwrap(),
                ))
                .unwrap();
        }
    }
    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(database)
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let result = client
        .confirmed_request(
            server.local_mac(),
            ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
            &[0x09, 0],
        )
        .await;
    client.stop().await.unwrap();
    server.stop().await.unwrap();
    assert!(
        matches!(result, Err(Error::Abort { reason })
        if reason == AbortReason::OUT_OF_RESOURCES.to_raw()),
        "{result:?}"
    );
}
