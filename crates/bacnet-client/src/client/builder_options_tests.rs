use super::*;
use bacnet_transport::loopback::LoopbackTransport;
use std::net::Ipv4Addr;

fn assert_apdu_tuning(config: &ClientConfig) {
    assert_eq!(config.apdu_retries, 7);
    assert_eq!(config.max_segments, Some(8));
    assert!(!config.segmented_response_accepted);
    assert_eq!(config.proposed_window_size, 4);
}

#[tokio::test]
async fn generic_builder_sets_apdu_tuning_options() {
    let (transport, _peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .apdu_retries(7)
        .max_segments(Some(8))
        .segmented_response_accepted(false)
        .proposed_window_size(4)
        .build()
        .await
        .unwrap();

    assert_apdu_tuning(&client.config);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn bip_builder_sets_apdu_tuning_options() {
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_retries(7)
        .max_segments(Some(8))
        .segmented_response_accepted(false)
        .proposed_window_size(4)
        .build()
        .await
        .unwrap();

    assert_apdu_tuning(&client.config);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn builder_rejects_invalid_proposed_window_size() {
    let (transport, _peer_transport) = LoopbackTransport::pair(vec![0x10], vec![0x11]);
    let result = BACnetClient::generic_builder()
        .transport(transport)
        .proposed_window_size(0)
        .build()
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn builder_rejects_max_segments_below_protocol_minimum() {
    for invalid in [0, 1] {
        let (transport, _peer_transport) = LoopbackTransport::pair(vec![0x10], vec![0x11]);
        let result = BACnetClient::generic_builder()
            .transport(transport)
            .max_segments(Some(invalid))
            .build()
            .await;

        match result {
            Err(Error::Encoding(message)) => {
                assert!(message.contains("max-segments-accepted"));
            }
            Err(other) => panic!("expected encoding error, got {other}"),
            Ok(mut client) => {
                client.stop().await.unwrap();
                panic!("max_segments={invalid} must be rejected");
            }
        }
    }
}
