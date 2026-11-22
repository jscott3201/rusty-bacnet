use std::sync::Arc;

use bacnet_npdu::{RejectMessageReason, RejectMessageToNetwork};

use super::super::routed_path_limit_tests::{
    DNET, confirmed_request, harness, inject_reason_4, routed_request,
};
use super::*;

#[test]
fn routers_are_kept_per_mac_with_every_network_once() {
    let mut routers = Vec::new();
    let mac = |octet: u8| MacAddr::from_slice(&[octet]);
    record(&mut routers, mac(1), vec![5, 2]);
    record(&mut routers, mac(9), vec![2]);
    record(&mut routers, mac(1), vec![3, 5]);
    assert_eq!(
        routers,
        [
            DiscoveredRouter {
                mac: mac(1),
                networks: vec![2, 3, 5],
            },
            DiscoveredRouter {
                mac: mac(9),
                networks: vec![2],
            },
        ]
    );
}

/// A subscriber to the reports takes nothing from the routed-path limits:
/// the reason-4 reject still completes the request it answers, and the
/// subscriber sees it too.
#[tokio::test]
async fn a_reject_reaches_the_path_limits_and_the_subscribers() {
    let router = vec![2];
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
    // On a wide path the request goes out whole: its frame is above the
    // conservative limit, so a reason 4 can be believed (#2665).
    client
        .configure_routed_path_max_npdu(&router, DNET, 1497)
        .unwrap();
    let mut reports = client.network_reports();
    let task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        vec![3],
        vec![0x44; 300],
    );
    confirmed_request(outbound.recv().await.unwrap(), &router);
    inject_reason_4(&inbound, &router, DNET).await;
    let outcome = tokio::time::timeout(Duration::from_millis(500), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        outcome,
        Err(Error::RoutedPathTooLong { dnet: DNET })
    ));
    let received = reports.recv().await.unwrap();
    assert_eq!(received.source_mac.as_slice(), router);
    assert_eq!(
        received.report,
        NetworkReport::RejectMessageToNetwork(RejectMessageToNetwork {
            reason: RejectMessageReason::MESSAGE_TOO_LONG,
            network: DNET,
        })
    );
    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn directed_router_discovery_collects_only_the_selected_local_station() {
    use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let mut client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    let send_replies = async {
        let sent = outbound.recv().await.unwrap();
        assert_eq!(sent.destination.as_slice(), [2]);
        assert_eq!(sent.npdu.as_ref(), [1, 0x80, 0, 0, 7]);
        for (source, bytes) in [
            (3, &[1, 0x80, 1, 0, 7, 0, 30][..]),         // another router
            (2, &[1, 0x88, 0, 8, 1, 4, 1, 0, 7, 0, 40]), // routed through the selected peer
            (2, &[1, 0x80, 1, 0, 90]),                   // outside the requested network
            (2, &[1, 0x80, 1, 0, 7, 0, 12]),
            (2, &[1, 0x80, 1, 0, 7, 0, 11]),
        ] {
            inbound
                .send(ReceivedNpdu {
                    npdu: bytes.to_vec().into(),
                    source_mac: MacAddr::from_slice(&[source]),
                    link_layer_group: false,
                    data_attributes: Vec::new(),
                    provenance: TransportProvenance::unverified(),
                    direct_response: None,
                    reply_tx: None,
                })
                .await
                .unwrap();
        }
    };
    let wait = std::time::Duration::from_secs(2);
    let (found, ()) = tokio::join!(
        client.discover_routers_directed(&[2], Some(7), wait),
        send_replies
    );
    assert_eq!(
        found.unwrap(),
        [DiscoveredRouter {
            mac: MacAddr::from_slice(&[2]),
            networks: vec![7, 11, 12]
        }]
    );
    assert!(
        client
            .discover_routers_directed(&[4], None, wait)
            .await
            .unwrap()
            .is_empty()
    );
    client.stop().await.unwrap();
}
