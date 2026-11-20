//! Correlate network-message responses to one station behind one peer.
use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_npdu::NetworkMessageType as Msg;

fn address(network: u16, octet: u8) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: [octet].as_slice().into(),
    }
}

#[tokio::test(start_paused = true)]
async fn a_routed_table_accepts_only_the_exact_hop_and_source_including_empty_acks() {
    let (mut client, inbound, mut outbound) = started().await;
    let station = Station::routed(&ROUTER, 9, &[3]);
    for payload in [&[][..], &[0], &[1, 0, 42, 1, 0]] {
        let (result, ()) = tokio::join!(
            client.routing_table(&station, RoutingTableWait::default()),
            async {
                let sent = outbound.recv().await.unwrap();
                let query = decode_npdu(sent.npdu).unwrap();
                assert_eq!(sent.destination.as_slice(), ROUTER);
                assert_eq!(query.destination, Some(address(9, 3)));
                assert!(query.expecting_reply);
                assert_eq!(query.message_type, Some(Msg::INITIALIZE_ROUTING_TABLE));
                assert_eq!(query.payload.as_ref(), [0]);
                for (hop, source) in [
                    (&[4][..], Some(address(9, 3))),
                    (&ROUTER, Some(address(8, 3))),
                    (&ROUTER, Some(address(9, 4))),
                    (&ROUTER, None),
                ] {
                    control(
                        &inbound,
                        hop,
                        Msg::INITIALIZE_ROUTING_TABLE_ACK,
                        &[1, 0, 99, 2, 0],
                        source.clone(),
                    )
                    .await;
                    control(
                        &inbound,
                        hop,
                        Msg::INITIALIZE_ROUTING_TABLE_ACK,
                        &[],
                        source,
                    )
                    .await;
                }
                control(
                    &inbound,
                    &ROUTER,
                    Msg::REJECT_MESSAGE_TO_NETWORK,
                    &[1, 0, 8],
                    None,
                )
                .await;
                control(
                    &inbound,
                    &ROUTER,
                    Msg::REJECT_MESSAGE_TO_NETWORK,
                    &[1, 0, 9],
                    Some(address(8, 3)),
                )
                .await;
                control(
                    &inbound,
                    &ROUTER,
                    Msg::INITIALIZE_ROUTING_TABLE_ACK,
                    payload,
                    Some(address(9, 3)),
                )
                .await;
            }
        );
        let table = result.unwrap();
        assert_eq!(table.acks, 1);
        assert_eq!(
            table.entries,
            if payload.len() > 1 {
                vec![entry(42, 1)]
            } else {
                vec![]
            }
        );
    }
    let (result, ()) = tokio::join!(
        client.routing_table(&station, RoutingTableWait::default()),
        async {
            outbound.recv().await.unwrap();
            control(
                &inbound,
                &ROUTER,
                Msg::REJECT_MESSAGE_TO_NETWORK,
                &[1, 0, 9],
                None,
            )
            .await;
        }
    );
    assert!(matches!(
        result,
        Err(RoutingTableError::Rejected(RejectMessageToNetwork {
            network: 9,
            ..
        }))
    ));
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn routing_table_discovers_only_local_hops_and_refuses_no_or_ambiguous_routes() {
    let (mut client, inbound, mut outbound) = started().await;
    let station = Station::remote(9, &[3]);
    for count in [0, 2] {
        let (result, ()) = tokio::join!(
            client.routing_table(&station, RoutingTableWait::default()),
            async {
                let query = decode_npdu(outbound.recv().await.unwrap().npdu).unwrap();
                assert_eq!(query.message_type, Some(Msg::WHO_IS_ROUTER_TO_NETWORK));
                assert_eq!(query.payload.as_ref(), [0, 9]);
                control(
                    &inbound,
                    &[8],
                    Msg::I_AM_ROUTER_TO_NETWORK,
                    &[0, 9],
                    Some(address(8, 3)),
                )
                .await;
                for peer in 0..count {
                    control(
                        &inbound,
                        &[2 + peer as u8],
                        Msg::I_AM_ROUTER_TO_NETWORK,
                        &[0, 9],
                        None,
                    )
                    .await;
                }
            }
        );
        assert!(
            matches!(result, Err(RoutingTableError::RouteUnresolved { network: 9, candidates }) if candidates == count)
        );
        assert!(outbound.try_recv().is_err());
    }
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn routing_table_localizes_own_network_and_reuses_an_exactly_cached_route() {
    use crate::discovery::RoutedDeviceConfig;
    use bacnet_types::enums::Segmentation;
    let (mut client, inbound, mut outbound) = started().await;
    client
        .network
        .local_network_number()
        .publish(bacnet_types::network_number::NetworkNumber::configured(7).unwrap());
    client
        .add_routed_device(RoutedDeviceConfig {
            instance: 2612,
            router_mac: ROUTER.to_vec(),
            remote_network: 9,
            remote_mac: vec![3],
            max_apdu_length: 480,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();

    for (station, hop, routed) in [
        (Station::routed(&[44], 7, &[3]), &[3][..], None),
        (Station::remote(9, &[3]), &ROUTER, Some(address(9, 3))),
    ] {
        let (table, ()) = tokio::join!(
            client.routing_table(&station, RoutingTableWait::default()),
            async {
                let frame = outbound.recv().await.unwrap();
                let query = decode_npdu(frame.npdu).unwrap();
                assert_eq!(query.message_type, Some(Msg::INITIALIZE_ROUTING_TABLE));
                assert_eq!(frame.destination.as_slice(), hop);
                assert_eq!(query.destination, routed);
                control(
                    &inbound,
                    hop,
                    Msg::INITIALIZE_ROUTING_TABLE_ACK,
                    &[0],
                    routed,
                )
                .await;
            }
        );
        assert_eq!(table.unwrap().acks, 1);
    }
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn routing_table_refuses_an_apparently_unique_hop_after_losing_reports() {
    let (inbound, inbound_rx) = mpsc::channel(1024);
    let (sent, mut frames) = mpsc::unbounded_channel();
    let transport = TestTransport::builder()
        .local_mac(&[1])
        .inbound(inbound_rx)
        .on_send(move |frame| {
            sent.send(frame.npdu).unwrap();
            async { Ok(()) }
        })
        .build();
    let mut client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    // Only the first report announces one peer. All retained reports
    // can name the other peer once the subscriber overruns its queue.
    inbound.try_send(i_am_router_npdu(&[2])).unwrap();
    for _ in 0..300 {
        inbound.try_send(i_am_router_npdu(&[3])).unwrap();
    }
    let error = client
        .routing_table(&Station::remote(9, &[4]), RoutingTableWait::default())
        .await
        .unwrap_err();
    assert!(
        matches!(error, RoutingTableError::DiscoveryIncomplete { network: 9, lost } if lost > 0)
    );
    assert!(error.to_string().contains("explicit next hop"));
    let question = decode_npdu(frames.try_recv().unwrap()).unwrap();
    assert_eq!(question.message_type, Some(Msg::WHO_IS_ROUTER_TO_NETWORK));
    assert_eq!(question.payload.as_ref(), [0, 9]);
    assert!(
        frames.try_recv().is_err(),
        "no routing-table query may follow"
    );
    client.stop().await.unwrap();
}
