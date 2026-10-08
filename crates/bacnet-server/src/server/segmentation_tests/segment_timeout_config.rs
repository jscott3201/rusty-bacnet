//! One segment timeout declaration drives all full-server entry points.
use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};

fn database(mode: Segmentation, timeout: u64) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            segmentation_supported: mode,
            apdu_segment_timeout: timeout,
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db
}

#[tokio::test]
async fn segment_timeout_invalid_configuration_and_declaration_fail_before_io() {
    for (mode, timeout, device_mode, device_timeout) in [
        (Segmentation::BOTH, 0, Segmentation::BOTH, 5000),
        (Segmentation::BOTH, u64::MAX, Segmentation::BOTH, 5000),
        (Segmentation::from_raw(64), 5000, Segmentation::NONE, 5000),
        (Segmentation::BOTH, 5000, Segmentation::NONE, 5000),
        (Segmentation::NONE, 5000, Segmentation::BOTH, 5000),
        (Segmentation::RECEIVE, 5000, Segmentation::TRANSMIT, 5000),
        (Segmentation::BOTH, 5000, Segmentation::BOTH, 6000),
    ] {
        let config = ServerConfig {
            segmentation_supported: mode,
            apdu_segment_timeout_ms: timeout,
            ..Default::default()
        };
        let error = BACnetServer::start(
            config,
            database(device_mode, device_timeout),
            TestTransport::never_start(),
        )
        .await
        .err()
        .expect("invalid contract must fail");
        assert!(matches!(error, Error::Encoding(_)), "{error:?}");
    }
    // Checked multiplication and platform deadline representation are distinct.
    let timeout_ms = u64::MAX / 4;
    if Instant::now()
        .checked_add(Duration::from_millis(timeout_ms * 4))
        .is_none()
    {
        let config = ServerConfig {
            apdu_segment_timeout_ms: timeout_ms,
            ..Default::default()
        };
        assert!(super::super::segmented_receive::validate_segment_timeout(
            &config,
            &ObjectDatabase::new()
        )
        .is_err());
    }
}

#[tokio::test]
async fn segment_timeout_builder_validation_precedes_transport_start_or_sc_dial() {
    for timeout in [0, u64::MAX] {
        let generic = BACnetServer::generic_builder()
            .transport(TestTransport::never_start())
            .segmentation_supported(Segmentation::BOTH)
            .apdu_segment_timeout_ms(timeout)
            .build()
            .await
            .err();
        let bip = BACnetServer::bip_builder()
            .port(0)
            .segmentation_supported(Segmentation::BOTH)
            .apdu_segment_timeout_ms(timeout)
            .build()
            .await
            .err();
        for error in [generic, bip] {
            assert!(matches!(error, Some(Error::Encoding(m)) if m.contains("segment")));
        }
        #[cfg(feature = "sc-tls")]
        {
            let error = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(crate::server::sc_builder::test_tls_config())
                .device_uuid(crate::server::sc_builder::TEST_DEVICE_UUID)
                .segmentation_supported(Segmentation::BOTH)
                .apdu_segment_timeout_ms(timeout)
                .build()
                .await
                .err();
            assert!(matches!(error, Some(Error::Encoding(m)) if m.contains("segment")));
        }
    }
}

#[tokio::test]
async fn segment_timeout_all_known_modes_start_with_coherent_device() {
    for mode in [
        Segmentation::NONE,
        Segmentation::TRANSMIT,
        Segmentation::RECEIVE,
        Segmentation::BOTH,
    ] {
        let (transport, _) = recording_transport();
        let mut server = BACnetServer::generic_builder()
            .transport(transport)
            .database(database(mode, 700))
            .segmentation_supported(mode)
            .apdu_segment_timeout_ms(700)
            .build()
            .await
            .unwrap();
        assert_eq!(server.config.apdu_segment_timeout_ms, 700);
        server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn configured_segment_timeout_drives_real_server_response_ack_wait() {
    use bacnet_services::read_property::ReadPropertyRequest;
    let (transport, incoming, sent) = request_reassembly::routed_injection_transport();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            name: "large-response-".repeat(20),
            segmentation_supported: Segmentation::TRANSMIT,
            apdu_segment_timeout: 250,
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut server = BACnetServer::generic_builder()
        .transport(transport)
        .database(db)
        .segmentation_supported(Segmentation::TRANSMIT)
        .apdu_segment_timeout_ms(250)
        .build()
        .await
        .unwrap();
    let read = ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    };
    let mut service = BytesMut::new();
    read.encode(&mut service);
    let router = test_mac(30);
    let remote = routed_address(400, 9);
    let request = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 50,
        invoke_id: 42,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: service.freeze(),
    });
    request_reassembly::inject_routed_apdu(&incoming, &router, &remote, &request).await;
    wait_for_sent_len(&sent, 1).await;
    assert!(
        matches!(sent.frame(0).apdu(), Apdu::ComplexAck(ack) if ack.segmented && ack.sequence_number == Some(0))
    );
    tokio::time::advance(Duration::from_millis(249)).await;
    tokio::task::yield_now().await;
    assert_eq!(sent_count(&sent), 1, "configured Tseg has not elapsed");
    tokio::time::advance(Duration::from_millis(2)).await;
    wait_for_sent_len(&sent, 2).await;
    assert_eq!(
        sent.frame(0),
        sent.frame(1),
        "first segment retransmitted at configured Tseg"
    );
    request_reassembly::inject_routed_apdu(
        &incoming,
        &router,
        &remote,
        &Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id: 42,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    server.stop().await.unwrap();
}
