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
    inbound.send(ack_npdu(from, entries)).await.unwrap();
}

fn ack_npdu(from: &[u8], entries: &[(u16, u8)]) -> ReceivedNpdu {
    let mut data = vec![entries.len() as u8];
    for &(network, port) in entries {
        data.extend_from_slice(&network.to_be_bytes());
        data.extend_from_slice(&[port, 0]);
    }
    control_npdu(
        from,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &data,
        None,
    )
}

fn i_am_router_npdu(from: &[u8]) -> ReceivedNpdu {
    control_npdu(
        from,
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        &[0, 9],
        None,
    )
}

async fn control(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    from: &[u8],
    message_type: NetworkMessageType,
    payload: &[u8],
    source: Option<NpduAddress>,
) {
    let npdu = control_npdu(from, message_type, payload, source);
    inbound.send(npdu).await.unwrap();
}

fn control_npdu(
    from: &[u8],
    message_type: NetworkMessageType,
    payload: &[u8],
    source: Option<NpduAddress>,
) -> ReceivedNpdu {
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
    ReceivedNpdu {
        direct_response: None,
        npdu: buf.freeze(),
        source_mac: MacAddr::from_slice(from),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
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
        Err(error @ RoutingTableError::NoAnswer { waited, routed }) => {
            assert_eq!((waited, routed), (Duration::from_secs(2), None));
            assert!(!error.to_string().contains("own link"), "{error}");
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
        first_answer: Duration::from_millis(500),
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
    let late = RoutingTableWait {
        first_answer: Duration::from_secs(11),
        ..RoutingTableWait::default()
    };
    for wait in [empty, late] {
        assert!(matches!(
            client.routing_table(&ROUTER, wait).await,
            Err(RoutingTableError::InvalidArgument(_))
        ));
    }
    assert!(matches!(
        client.routing_table(&[], RoutingTableWait::default()).await,
        Err(RoutingTableError::InvalidArgument(_))
    ));
    assert!(outbound.try_recv().is_err(), "nothing was sent");
    client.stop().await.unwrap();
}

/// A link that refuses the query for the router's address, which the call
/// was given, refuses the call as an invalid argument, which the bindings
/// raise as `ValueError`. One that refuses it for its own state (a
/// BACnet/SC link with no hub connection), configuration, platform or
/// limits, a stopped link and a send too long are send failures, which
/// keep the link's error and which the bindings raise as they raise that
/// error (#1833).
#[tokio::test(start_paused = true)]
async fn a_link_refusing_the_query_refuses_the_call() {
    use bacnet_types::error::Lifecycle;
    const REFUSED: &str = "BACnet/SC transport not in Connected state";
    let cases: [(fn() -> Error, bool); 7] = [
        (|| Error::state(Lifecycle::NotConnected, REFUSED), false),
        (|| Error::invalid_config(REFUSED), false),
        (|| Error::invalid_argument(REFUSED), true),
        (|| Error::unsupported(REFUSED), false),
        (|| Error::limit_reached(REFUSED), false),
        (
            || {
                Error::Transport(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "stopped",
                ))
            },
            false,
        ),
        (
            || Error::SendTooLong {
                length: 600,
                limit: 480,
            },
            false,
        ),
    ];
    for (refusal, argument) in cases {
        let (_inbound, inbound_rx) = mpsc::channel(1);
        let transport = TestTransport::builder()
            .local_mac(&[1])
            .inbound(inbound_rx)
            .egress_limit(1486)
            .on_send(move |_| async move { Err(refusal()) })
            .build();
        let mut client = BACnetClient::start(ClientConfig::default(), transport)
            .await
            .unwrap();
        let result = client
            .routing_table(&ROUTER, RoutingTableWait::default())
            .await;
        match result {
            Err(RoutingTableError::InvalidArgument(why)) if argument => {
                assert_eq!(why, REFUSED);
            }
            Err(RoutingTableError::Send(error)) if !argument => {
                assert_eq!(error.to_string(), refusal().to_string());
            }
            other => panic!("{:?}: {other:?}", refusal()),
        }
        client.stop().await.unwrap();
    }
}

async fn i_am_router(inbound: &mpsc::Sender<ReceivedNpdu>, from: &[u8]) {
    control(
        inbound,
        from,
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        &[0, 9],
        None,
    )
    .await;
}

/// Only an accepted ack restarts the quiet interval: another station's
/// announcements every 400 ms do not keep a finished table running to the
/// total bound.
#[tokio::test(start_paused = true)]
async fn other_reports_do_not_restart_the_quiet_interval() {
    let (result, _) = query(RoutingTableWait::default(), |inbound| async move {
        ack(&inbound, &ROUTER, &[(2, 1), (3, 1)]).await;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(400)).await;
            i_am_router(&inbound, &[9]).await;
        }
    })
    .await;
    let table = result.unwrap();
    assert_eq!(table.entries.len(), 2);
    assert!(!table.truncated, "{table:?}");
}

/// A reject for a DNET (reasons 1, 2 and 4) is for our routed requests, not
/// this local query; the others from the router end the call.
#[tokio::test(start_paused = true)]
async fn rejects_for_a_dnet_are_ignored_and_the_others_end_the_call() {
    let reject = |reason: RejectMessageReason| async move {
        query(RoutingTableWait::default(), |inbound| async move {
            control(
                &inbound,
                &ROUTER,
                NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
                &[reason.to_raw(), 0, 77],
                None,
            )
            .await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            ack(&inbound, &ROUTER, &[(2, 1)]).await;
        })
        .await
        .0
    };
    for ignored in [
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        RejectMessageReason::ROUTER_BUSY,
        RejectMessageReason::MESSAGE_TOO_LONG,
    ] {
        let table = reject(ignored).await.unwrap();
        assert_eq!(table.entries, [entry(2, 1)], "{ignored}");
    }
    for ending in [
        RejectMessageReason::OTHER,
        RejectMessageReason::UNKNOWN_MESSAGE_TYPE,
        RejectMessageReason::ADDRESSING_ERROR,
        RejectMessageReason::from_raw(5),
        RejectMessageReason::from_raw(9),
    ] {
        assert!(
            matches!(reject(ending).await, Err(RoutingTableError::Rejected(_))),
            "{ending}"
        );
    }
}

/// Two calls to one router run one after the other, so each gets the
/// 2-entry table once, not both answers.
#[tokio::test(start_paused = true)]
async fn concurrent_queries_are_serialized() {
    let (mut client, inbound, mut outbound) = started().await;
    let wait = RoutingTableWait::default();
    let (a, b, ()) = tokio::join!(
        client.routing_table(&ROUTER, wait),
        client.routing_table(&ROUTER, wait),
        async {
            for _ in 0..2 {
                outbound.recv().await.expect("a query");
                ack(&inbound, &ROUTER, &[(2, 1), (3, 1)]).await;
            }
        }
    );
    for table in [a.unwrap(), b.unwrap()] {
        assert_eq!(table.entries.len(), 2, "{table:?}");
        assert_eq!(table.acks, 1);
    }
    client.stop().await.unwrap();
}

/// An older router's ack with no data, while the query is outstanding, is an
/// empty table; outside a call it is no report at all.
#[tokio::test(start_paused = true)]
async fn a_data_less_ack_answers_a_query_only_while_one_waits() {
    let (result, _) = query(RoutingTableWait::default(), |inbound| async move {
        control(
            &inbound,
            &ROUTER,
            NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
            &[],
            None,
        )
        .await;
    })
    .await;
    let table = result.unwrap();
    assert!(table.entries.is_empty());
    assert_eq!(table.acks, 1);

    let (mut client, inbound, _outbound) = started().await;
    let mut reports = client.network_reports();
    control(
        &inbound,
        &ROUTER,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &[],
        None,
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), reports.recv())
            .await
            .is_err()
    );
    client.stop().await.unwrap();
}

/// The outstanding router is cleared when a call ends, on the same client:
/// a data-less ack afterwards is no report.
#[tokio::test(start_paused = true)]
async fn the_outstanding_query_clears_after_a_call_on_the_same_client() {
    let (mut client, inbound, mut outbound) = started().await;
    let (result, ()) = tokio::join!(
        client.routing_table(&ROUTER, RoutingTableWait::default()),
        async {
            outbound.recv().await.expect("the query");
            ack(&inbound, &ROUTER, &[(2, 1)]).await;
        }
    );
    result.unwrap();
    let mut reports = client.network_reports();
    control(
        &inbound,
        &ROUTER,
        NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        &[],
        None,
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), reports.recv())
            .await
            .is_err()
    );
    client.stop().await.unwrap();
}

/// While a query to one router waits, a data-less ack from another MAC (or
/// routed) is no report, so the call stays silent.
#[tokio::test(start_paused = true)]
async fn a_data_less_ack_from_another_source_is_no_report() {
    let (mut client, inbound, mut outbound) = started().await;
    let mut reports = client.network_reports();
    let (result, ()) = tokio::join!(
        client.routing_table(&ROUTER, RoutingTableWait::default()),
        async {
            outbound.recv().await.expect("the query");
            control(
                &inbound,
                &[9],
                NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
                &[],
                None,
            )
            .await;
            control(
                &inbound,
                &ROUTER,
                NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
                &[],
                Some(NpduAddress {
                    network: 5,
                    mac_address: [4].as_slice().into(),
                }),
            )
            .await;
        }
    );
    assert!(matches!(result, Err(RoutingTableError::NoAnswer { .. })));
    assert!(reports.try_recv().is_err(), "no report was published");
    client.stop().await.unwrap();
}

/// A link that closes mid-call ends the call at once as `ClientStopped`,
/// not as silence after the whole wait.
#[tokio::test(start_paused = true)]
async fn a_closed_link_ends_the_call_as_client_stopped() {
    let (mut client, inbound, mut outbound) = started().await;
    let started_at = tokio::time::Instant::now();
    let (result, ()) = tokio::join!(
        client.routing_table(&ROUTER, RoutingTableWait::default()),
        async {
            outbound.recv().await.expect("the query");
            drop(inbound);
        }
    );
    assert!(
        matches!(result, Err(RoutingTableError::ClientStopped)),
        "{result:?}"
    );
    assert!(started_at.elapsed() < Duration::from_secs(1));
    // A later call on the stopped client says so too, without waiting.
    assert!(matches!(
        client
            .routing_table(&ROUTER, RoutingTableWait::default())
            .await,
        Err(RoutingTableError::ClientStopped | RoutingTableError::Send(_))
    ));
    client.stop().await.unwrap();
}

/// Bounds too large for the clock are an argument error, nothing sent.
#[tokio::test(start_paused = true)]
async fn bounds_too_large_for_the_clock_are_refused() {
    let (mut client, _inbound, mut outbound) = started().await;
    for wait in [
        RoutingTableWait {
            total: Duration::MAX,
            first_answer: Duration::from_secs(1),
            ..RoutingTableWait::default()
        },
        RoutingTableWait {
            first_answer: Duration::MAX,
            total: Duration::MAX,
            ..RoutingTableWait::default()
        },
        RoutingTableWait {
            quiet: Duration::MAX,
            ..RoutingTableWait::default()
        },
    ] {
        assert!(matches!(
            client.routing_table(&ROUTER, wait).await,
            Err(RoutingTableError::InvalidArgument(_))
        ));
    }
    assert!(outbound.try_recv().is_err(), "nothing was sent");
    client.stop().await.unwrap();
}

/// A collector that falls behind loses reports and says so with
/// `truncated`.
#[tokio::test(start_paused = true)]
async fn a_lagging_collector_marks_the_table_truncated() {
    // A link whose queue holds the whole burst, filled before the call
    // reads any of it: the dispatch task publishes far more reports than
    // the channel holds before the collector runs.
    let (inbound, inbound_rx) = mpsc::channel(1024);
    let (outbound_tx, mut outbound) = mpsc::unbounded_channel();
    let transport = TestTransport::builder()
        .local_mac(&[1])
        .inbound(inbound_rx)
        .on_send(move |frame| {
            let _ = outbound_tx.send(frame.npdu);
            async { Ok(()) }
        })
        .build();
    let mut client = BACnetClient::start(ClientConfig::default(), transport)
        .await
        .unwrap();
    for _ in 0..300 {
        inbound
            .try_send(i_am_router_npdu(&[9]))
            .unwrap_or_else(|_| panic!("queue room"));
    }
    inbound
        .try_send(ack_npdu(&ROUTER, &[(2, 1)]))
        .unwrap_or_else(|_| panic!("queue room"));
    let table = client
        .routing_table(&ROUTER, RoutingTableWait::default())
        .await
        .unwrap();
    assert!(outbound.try_recv().is_ok());
    assert!(table.truncated, "{table:?}");
    client.stop().await.unwrap();
}

/// A call polled once while another holds the one-query gate, and never
/// again, holds the gate for nobody once that call ends (#2759): a third
/// call, polled, runs. It waited behind the parked one for good while the
/// gate was Tokio's fair mutex.
#[tokio::test(start_paused = true)]
async fn a_parked_call_holds_the_query_gate_for_nobody() {
    use std::task::{Context, Waker};
    let (mut client, inbound, mut outbound) = started().await;
    let wait = RoutingTableWait::default();
    let (go, gone) = tokio::sync::oneshot::channel();
    let mut parked = Box::pin(client.routing_table(&ROUTER, wait));
    let driver = async {
        outbound.recv().await.expect("the first query");
        let noop = &mut Context::from_waker(Waker::noop());
        assert!(parked.as_mut().poll(noop).is_pending());
        go.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        ack(&inbound, &ROUTER, &[(2, 1)]).await;
        let third = tokio::time::timeout(Duration::from_secs(60), outbound.recv());
        third.await.expect("the third call's query").unwrap();
        ack(&inbound, &ROUTER, &[(3, 1)]).await;
    };
    let third = async {
        gone.await.unwrap();
        client.routing_table(&ROUTER, wait).await
    };
    let (first, (), third) = tokio::join!(client.routing_table(&ROUTER, wait), driver, third);
    assert_eq!(first.unwrap().entries, [entry(2, 1)]);
    assert_eq!(third.unwrap().entries, [entry(3, 1)]);
    drop(parked);
    client.stop().await.unwrap();
}

#[path = "routing_table_routed_tests.rs"]
mod routed;
