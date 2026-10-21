//! Routed paths refuse MACs past `bacnet_npdu::MAX_MAC_LEN` up front (#1267).
use super::*;

/// A router MAC or forwarding source one octet past the bound fails a routed
/// request before registration or emission, and the path API refuses such a
/// router MAC.
#[tokio::test]
async fn over_long_routed_path_macs_fail_before_registration_or_emission() {
    let too_long = vec![2; bacnet_npdu::MAX_MAC_LEN + 1];
    for (local, router) in [
        (vec![1; 6], too_long.clone()),
        (too_long.clone(), vec![2; 6]),
    ] {
        let (transport, _inbound, mut outbound) = harness(&local, 1490);
        let config = ClientConfig {
            apdu_timeout_ms: 20,
            apdu_retries: 0,
            ..ClientConfig::default()
        };
        let mut client = BACnetClient::start(config, transport).await.unwrap();
        let result = client
            .confirmed_request_routed(
                &router,
                DNET,
                &[3; 6],
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x0c],
            )
            .await;
        assert!(matches!(result, Err(Error::Encoding(_))), "{result:?}");
        assert!(outbound.try_recv().is_err());
        assert_eq!(client.tsm.lock().await.pending_count(), 0);
        for refused in [
            client
                .configure_routed_path_max_npdu(&too_long, DNET, 1497)
                .await,
            client.clear_routed_path_limit(&too_long, DNET).await,
        ] {
            assert!(matches!(refused, Err(Error::Encoding(_))));
        }
        client.stop().await.unwrap();
    }
}
