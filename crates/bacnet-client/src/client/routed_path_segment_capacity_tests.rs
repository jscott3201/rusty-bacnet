//! Segmented outgoing path limits retain their independent egress budget.
use super::*;

#[tokio::test]
async fn reason_4_during_segmented_send_prevents_window_retransmission() {
    let router = vec![2];
    let dadr = vec![3];
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::start(
            ClientConfig {
                apdu_timeout_ms: 40,
                apdu_retries: 3,
                ..ClientConfig::default()
            },
            transport,
        )
        .await
        .unwrap(),
    );
    client
        .configure_routed_path_max_npdu(&router, DNET, 128)
        .await
        .unwrap();

    let task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x77; 300],
    );
    let first = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(first.segmented);
    inject_reason_4(&inbound, &router, DNET).await;
    assert!(matches!(
        timeout(Duration::from_millis(100), task)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::RoutedPathTooLong { dnet: DNET })
    ));
    assert!(
        timeout(Duration::from_millis(150), outbound.recv())
            .await
            .is_err()
    );

    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}
