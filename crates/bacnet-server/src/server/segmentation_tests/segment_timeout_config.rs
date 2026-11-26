//! One segment timeout declaration drives all full-server entry points.
use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};

fn database(mode: Segmentation, timeout: u64) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default()
                .with_segmentation(mode)
                .with_apdu_segment_timeout_ms(timeout),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db
}

/// Who refuses a contract: the server's own check of four segment
/// timeouts, or bacnet-objects' checks of the identity and of the Device it
/// is declared against. Both are configuration refusals (#1833), so only
/// the text tells them apart.
const SERVER: &str = "server";
const OBJECTS: &str = "bacnet-objects";
const SEGMENTATION_MISMATCH: &str =
    "selected Device PropertyIdentifier::SEGMENTATION_SUPPORTED must match identity";

#[tokio::test]
async fn segment_timeout_invalid_configuration_and_declaration_fail_before_io() {
    for (mode, timeout, device_mode, device_timeout, whose, refusal) in [
        (
            Segmentation::SEGMENTED_BOTH,
            0,
            Segmentation::SEGMENTED_BOTH,
            5000,
            OBJECTS,
            "APDU_Segment_Timeout must be positive when segmentation is supported",
        ),
        (
            Segmentation::SEGMENTED_BOTH,
            u64::MAX,
            Segmentation::SEGMENTED_BOTH,
            5000,
            SERVER,
            "receive segment timeout overflows milliseconds",
        ),
        (
            Segmentation::from_raw(64),
            5000,
            Segmentation::NO_SEGMENTATION,
            5000,
            OBJECTS,
            "invalid Device segmentation support",
        ),
        (
            Segmentation::SEGMENTED_BOTH,
            5000,
            Segmentation::NO_SEGMENTATION,
            5000,
            OBJECTS,
            SEGMENTATION_MISMATCH,
        ),
        (
            Segmentation::NO_SEGMENTATION,
            5000,
            Segmentation::SEGMENTED_BOTH,
            5000,
            OBJECTS,
            SEGMENTATION_MISMATCH,
        ),
        (
            Segmentation::SEGMENTED_RECEIVE,
            5000,
            Segmentation::SEGMENTED_TRANSMIT,
            5000,
            OBJECTS,
            SEGMENTATION_MISMATCH,
        ),
        (
            Segmentation::SEGMENTED_BOTH,
            5000,
            Segmentation::SEGMENTED_BOTH,
            6000,
            OBJECTS,
            "selected Device APDU_Segment_Timeout must match identity",
        ),
    ] {
        let config = ServerConfig {
            device: bacnet_objects::DeviceIdentity::default()
                .with_segmentation(mode)
                .with_apdu_segment_timeout_ms(timeout),
            ..Default::default()
        };
        let error = BACnetServer::start(
            config,
            database(device_mode, device_timeout),
            TestTransport::never_start(),
        )
        .await
        .map_err(Error::from)
        .err()
        .expect("invalid contract must fail");
        // A configuration refusal either way; the text says whose.
        assert!(
            matches!(&error, Error::InvalidConfig(why) if why.to_string() == refusal),
            "{whose}: {error:?}"
        );
    }
    // Checked multiplication and platform deadline representation are distinct.
    let timeout_ms = u64::MAX / 4;
    if Instant::now()
        .checked_add(Duration::from_millis(timeout_ms * 4))
        .is_none()
    {
        let config = ServerConfig {
            device: bacnet_objects::DeviceIdentity::default()
                .with_apdu_segment_timeout_ms(timeout_ms),
            ..Default::default()
        };
        assert!(super::super::segmented_receive::validate_segment_timeout(&config).is_err());
    }
}

#[tokio::test]
async fn segment_timeout_builder_validation_precedes_transport_start_or_sc_dial() {
    for timeout in [0, u64::MAX] {
        let generic = BACnetServer::generic_builder()
            .transport(TestTransport::never_start())
            .device(
                bacnet_objects::DeviceIdentity::default()
                    .with_segmentation(Segmentation::SEGMENTED_BOTH)
                    .with_apdu_segment_timeout_ms(timeout),
            )
            .build()
            .await
            .map_err(Error::from)
            .err();
        let bip = BACnetServer::bip_builder()
            .port(0)
            .device(
                bacnet_objects::DeviceIdentity::default()
                    .with_segmentation(Segmentation::SEGMENTED_BOTH)
                    .with_apdu_segment_timeout_ms(timeout),
            )
            .build()
            .await
            .map_err(Error::from)
            .err();
        for error in [generic, bip] {
            assert!(refused_for_segments(timeout, error));
        }
        #[cfg(feature = "sc-tls")]
        {
            let error = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(crate::server::sc_builder::test_tls_config())
                .device_uuid(crate::server::sc_builder::TEST_DEVICE_UUID)
                .device(
                    bacnet_objects::DeviceIdentity::default()
                        .with_segmentation(Segmentation::SEGMENTED_BOTH)
                        .with_apdu_segment_timeout_ms(timeout),
                )
                .build()
                .await
                .map_err(Error::from)
                .err();
            assert!(refused_for_segments(timeout, error));
        }
    }
}

#[tokio::test]
async fn segment_timeout_all_known_modes_start_with_coherent_device() {
    for mode in [
        Segmentation::NO_SEGMENTATION,
        Segmentation::SEGMENTED_TRANSMIT,
        Segmentation::SEGMENTED_RECEIVE,
        Segmentation::SEGMENTED_BOTH,
    ] {
        let (transport, _) = recording_transport();
        let mut server = BACnetServer::generic_builder()
            .transport(transport)
            .database(database(mode, 700))
            .device(
                bacnet_objects::DeviceIdentity::default()
                    .with_segmentation(mode)
                    .with_apdu_segment_timeout_ms(700),
            )
            .build()
            .await
            .unwrap();
        assert_eq!(server.config.device.apdu_segment_timeout_ms(), 700);
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
            identity: bacnet_objects::DeviceIdentity::default()
                .with_name("large-response-".repeat(20))
                .with_segmentation(Segmentation::SEGMENTED_TRANSMIT)
                .with_apdu_segment_timeout_ms(250),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut server = BACnetServer::generic_builder()
        .transport(transport)
        .database(db)
        .device(
            bacnet_objects::DeviceIdentity::default()
                .with_segmentation(Segmentation::SEGMENTED_TRANSMIT)
                .with_apdu_segment_timeout_ms(250),
        )
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

/// Whether a build with segment timeout `timeout` failed for it, as a
/// configuration refusal (#1833) whose text says whose: zero is
/// bacnet-objects' refusal of the identity; four of the largest overflow
/// the server's own clock check.
fn refused_for_segments(timeout: u64, error: Option<Error>) -> bool {
    let Some(Error::InvalidConfig(why)) = error else {
        return false;
    };
    let text = why.to_string();
    match timeout {
        0 => text == "APDU_Segment_Timeout must be positive when segmentation is supported",
        u64::MAX => text.contains("segment timeout overflow"),
        _ => false,
    }
}
