//! The routing-table query (#2612) against a scripted router: what it
//! sends, how acknowledgements are collected, and the typed results for
//! silence, a refusal, an empty table and the bounds. Paused time, so each
//! wait is exact.

use super::*;
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::TestTransport;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::routed_path_limit_tests::{SentFrame, harness};

const ROUTER: [u8; 1] = [2];

async fn started() -> (
    BACnetClient<TestTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<SentFrame>,
) {
    let (transport, inbound, outbound) = harness(&[1], 1486);
    let client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    (client, inbound, outbound)
}

/// An Initialize-Routing-Table-Ack from `from`, with `entries` as
/// (network, port) pairs and no port info, as one message.
async fn ack(inbound: &mpsc::Sender<ReceivedNpdu>, from: &[u8], entries: &[(u16, u8)]) {
    let mut data = vec![entries.len() as u8];
    for &(network, port) in entries {
        data.extend_from_slice(&network.to_be_bytes());
        data.extend_from_slice(&[port, 0]);
    }
    control(
        inbound,
        from,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &data,
        None,
    )
    .await;
}

async fn control(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    from: &[u8],
    message_type: NetworkMessageType,
    payload: &[u8],
    source: Option<NpduAddress>,
) {
    let mut buf = BytesMut::new();
    encode_npdu(
        &mut buf,
        &Npdu {
            message_type: Some(message_type),
            payload: Bytes::copy_from_slice(payload),
            source,
            ..Npdu::default()
        },
    )
    .unwrap();
    inbound
        .send(ReceivedNpdu {
            direct_response: None,
            npdu: buf.freeze(),
            source_mac: MacAddr::from_slice(from),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
}

fn entry(network: u16, port_id: u8) -> RoutingTableEntry {
    RoutingTableEntry {
        network,
        port_id,
        port_info: Vec::new(),
    }
}

/// Run one query, with `script` playing the router once the query is out.
async fn query<F, Fut>(
    wait: RoutingTableWait,
    script: F,
) -> (Result<RoutingTable, RoutingTableError>, Vec<u8>)
where
    F: FnOnce(mpsc::Sender<ReceivedNpdu>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let (mut client, inbound, mut outbound) = started().await;
    let (result, sent) = tokio::join!(client.routing_table(&ROUTER, wait), async {
        let sent = outbound.recv().await.expect("the query is sent");
        assert_eq!(sent.destination.as_slice(), ROUTER);
        // A clone: the link stays open until the call returns, as a closed
        // one ends the client's dispatch.
        script(inbound.clone()).await;
        sent.npdu.to_vec()
    });
    client.stop().await.unwrap();
    (result, sent)
}

#[tokio::test(start_paused = true)]
async fn the_query_is_a_zero_count_to_the_router_and_a_table_is_read() {
    let (result, sent) = query(RoutingTableWait::default(), |inbound| async move {
        ack(&inbound, &ROUTER, &[(2, 1), (3, 2)]).await;
    })
    .await;
    // Version 1, DER set, Initialize-Routing-Table, a count of zero.
    assert_eq!(sent, [1, 0x84, 0x06, 0]);
    let table = result.unwrap();
    assert_eq!(table.entries, [entry(2, 1), entry(3, 2)]);
    assert_eq!(table.acks, 1);
    assert!(!table.truncated);
}

/// A long table is several acks; the entries come back in order, and the
/// call ends on the quiet interval after the last.
#[tokio::test(start_paused = true)]
async fn a_table_split_over_several_acks_is_joined_in_order() {
    let started = tokio::time::Instant::now();
    let (result, _) = query(RoutingTableWait::default(), |inbound| async move {
        ack(&inbound, &ROUTER, &[(2, 1), (3, 1)]).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        ack(&inbound, &ROUTER, &[(4, 2)]).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        ack(&inbound, &ROUTER, &[(5, 2), (6, 3)]).await;
    })
    .await;
    let table = result.unwrap();
    assert_eq!(
        table.entries,
        [
            entry(2, 1),
            entry(3, 1),
            entry(4, 2),
            entry(5, 2),
            entry(6, 3)
        ]
    );
    assert_eq!(table.acks, 3);
    assert!(!table.truncated);
    // 200 ms of script, then the 500 ms quiet interval.
    assert_eq!(started.elapsed(), Duration::from_millis(700));
}

/// A router with an empty table answers with a count of zero: that is a
/// table, not silence.
#[tokio::test(start_paused = true)]
async fn an_empty_table_is_an_answer() {
    let (result, _) = query(RoutingTableWait::default(), |inbound| async move {
        ack(&inbound, &ROUTER, &[]).await;
    })
    .await;
    let table = result.unwrap();
    assert!(table.entries.is_empty());
    assert_eq!(table.acks, 1);
    assert!(!table.truncated);
}

/// No answer is an error that names the wait, after exactly that wait.
#[tokio::test(start_paused = true)]
async fn silence_is_no_answer_after_the_first_answer_wait() {
    let started = tokio::time::Instant::now();
    let wait = RoutingTableWait {
        first_answer: Duration::from_secs(2),
        ..RoutingTableWait::default()
    };
    let (result, _) = query(wait, |_inbound| async {}).await;
    match result {
        Err(RoutingTableError::NoAnswer { waited }) => {
            assert_eq!(waited, Duration::from_secs(2));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(started.elapsed(), Duration::from_secs(2));
}

/// A reject from the router before any ack is its refusal, with its reason.
#[tokio::test(start_paused = true)]
async fn a_reject_from_the_router_is_a_refusal() {
    let (result, _) = query(RoutingTableWait::default(), |inbound| async move {
        control(
            &inbound,
            &ROUTER,
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            &[RejectMessageReason::UNKNOWN_MESSAGE_TYPE.to_raw(), 0, 0],
            None,
        )
        .await;
    })
    .await;
    match result {
        Err(RoutingTableError::Rejected(reject)) => {
            assert_eq!(reject.reason, RejectMessageReason::UNKNOWN_MESSAGE_TYPE);
        }
        other => panic!("{other:?}"),
    }
}

/// What is not the router's answer is not taken: an ack or reject from
/// another station, and a routed one. The call ends as silence.
#[tokio::test(start_paused = true)]
async fn answers_from_others_are_ignored() {
    let (result, _) = query(RoutingTableWait::default(), |inbound| async move {
        ack(&inbound, &[9], &[(7, 1)]).await;
        control(
            &inbound,
            &[9],
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            &[RejectMessageReason::OTHER.to_raw(), 0, 0],
            None,
        )
        .await;
        control(
            &inbound,
            &ROUTER,
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            &[RejectMessageReason::OTHER.to_raw(), 0, 0],
            Some(NpduAddress {
                network: 5,
                mac_address: [4].as_slice().into(),
            }),
        )
        .await;
    })
    .await;
    assert!(matches!(result, Err(RoutingTableError::NoAnswer { .. })));
}

/// A reject that comes after an ack does not discard the table.
#[tokio::test(start_paused = true)]
async fn a_reject_after_an_ack_is_ignored() {
    let (result, _) = query(RoutingTableWait::default(), |inbound| async move {
        ack(&inbound, &ROUTER, &[(2, 1)]).await;
        control(
            &inbound,
            &ROUTER,
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            &[RejectMessageReason::OTHER.to_raw(), 0, 0],
            None,
        )
        .await;
    })
    .await;
    assert_eq!(result.unwrap().entries, [entry(2, 1)]);
}

/// A router that keeps answering is cut off at the total bound, and the
/// result says so.
#[tokio::test(start_paused = true)]
async fn the_total_bound_cuts_a_router_that_never_stops() {
    let wait = RoutingTableWait {
        total: Duration::from_secs(1),
        ..RoutingTableWait::default()
    };
    let (result, _) = query(wait, |inbound| async move {
        for network in 2..20u16 {
            ack(&inbound, &ROUTER, &[(network, 1)]).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    })
    .await;
    let table = result.unwrap();
    assert!(table.truncated);
    // Acks at 0, 300, 600 and 900 ms are in; the one at 1,200 ms is not.
    assert_eq!(table.entries.len(), 4, "{table:?}");
}

/// More entries than the bound keeps the first ones and says so; a table of
/// exactly the bound is complete.
#[tokio::test(start_paused = true)]
async fn the_entry_bound_keeps_the_first_entries() {
    let wait = RoutingTableWait {
        max_entries: 3,
        ..RoutingTableWait::default()
    };
    let (result, _) = query(wait, |inbound| async move {
        ack(&inbound, &ROUTER, &[(2, 1), (3, 1)]).await;
        ack(&inbound, &ROUTER, &[(4, 1), (5, 1)]).await;
    })
    .await;
    let table = result.unwrap();
    assert_eq!(table.entries, [entry(2, 1), entry(3, 1), entry(4, 1)]);
    assert!(table.truncated);

    let (result, _) = query(wait, |inbound| async move {
        ack(&inbound, &ROUTER, &[(2, 1), (3, 1), (4, 1)]).await;
    })
    .await;
    let table = result.unwrap();
    assert_eq!(table.entries.len(), 3);
    assert!(!table.truncated);
}

#[tokio::test(start_paused = true)]
async fn a_query_that_cannot_be_sent_is_refused_before_it_goes() {
    let (mut client, _inbound, mut outbound) = started().await;
    let empty = RoutingTableWait {
        max_entries: 0,
        ..RoutingTableWait::default()
    };
    assert!(matches!(
        client.routing_table(&ROUTER, empty).await,
        Err(RoutingTableError::Send(_))
    ));
    assert!(matches!(
        client.routing_table(&[], RoutingTableWait::default()).await,
        Err(RoutingTableError::Send(_))
    ));
    assert!(outbound.try_recv().is_err(), "nothing was sent");
    client.stop().await.unwrap();
}
