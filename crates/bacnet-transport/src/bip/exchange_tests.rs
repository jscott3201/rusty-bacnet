//! This transport's own management exchanges run side by side (#1622): one
//! per peer and operation, in up to eight slots. A request waits its turn
//! only behind the same request to the same peer, or behind another one a
//! success answers, which that success couldn't tell apart from it.

use super::*;

async fn peer() -> (UdpSocket, [u8; 6]) {
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mac = encode_bip_mac(
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    (peer, mac)
}

/// The next datagram `peer` receives, and who sent it.
async fn received(peer: &UdpSocket) -> (Vec<u8>, std::net::SocketAddr) {
    let mut packet = [0; 64];
    let (len, source) = tokio::time::timeout(Duration::from_secs(2), peer.recv_from(&mut packet))
        .await
        .expect("a request arrives")
        .unwrap();
    (packet[..len].to_vec(), source)
}

/// Two table reads to one peer, and a table read and a registration to
/// another, are all in flight before any is answered, and each answer,
/// given in the reverse order, completes its own request.
#[tokio::test]
async fn requests_to_different_peers_or_operations_run_at_the_same_time() {
    let (a, a_mac) = peer().await;
    let (b, b_mac) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    let requests = async {
        tokio::join!(
            transport.read_bdt(&a_mac),
            transport.read_fdt(&a_mac),
            transport.read_bdt(&b_mac),
            transport.register_foreign_device_bvlc(&b_mac, 60),
        )
    };
    let answers = async {
        let mut at_a = [received(&a).await, received(&a).await];
        let mut at_b = [received(&b).await, received(&b).await];
        at_a.sort();
        at_b.sort();
        assert_eq!(at_a[0].0, [0x81, 2, 0, 4]);
        assert_eq!(at_a[1].0, [0x81, 6, 0, 4]);
        assert_eq!(at_b[0].0, [0x81, 2, 0, 4]);
        assert_eq!(at_b[1].0, [0x81, 5, 0, 6, 0, 60]);
        let source = at_a[0].1;
        b.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
        b.send_to(&[0x81, 3, 0, 4], source).await.unwrap();
        a.send_to(&[0x81, 7, 0, 4], source).await.unwrap();
        a.send_to(&[0x81, 3, 0, 4], source).await.unwrap();
    };
    let ((bdt_a, fdt_a, bdt_b, registered), ()) = tokio::join!(requests, answers);
    assert!(bdt_a.unwrap().is_empty());
    assert!(fdt_a.unwrap().is_empty());
    assert!(bdt_b.unwrap().is_empty());
    registered.unwrap();
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(
        (snapshot.read_bdt.sent, snapshot.read_bdt.acknowledgements),
        (2, 2)
    );
    assert_eq!(snapshot.read_fdt.acknowledgements, 1);
    assert_eq!(snapshot.register_foreign_device.results, 1);
    assert_eq!(snapshot.unmatched_responses, 0);
    transport.stop().await.unwrap();
}

/// A normal node at 192.0.2.10 that sends nowhere: requests are only
/// admitted here, never sent or answered.
async fn node_ctx() -> RecvContext {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let socket = Arc::new(BipSocket::recording(socket, send_sink::SendSink::default()));
    let (npdu_tx, _) = mpsc::channel(1);
    RecvContext::for_test(
        socket,
        npdu_tx,
        SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 47808),
        SocketAddrV4::new(Ipv4Addr::BROADCAST, 47808),
        None,
    )
}

fn at(last: u8) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, last), 47808)
}

/// The same request to the same peer is busy, and so is a second request a
/// success answers to one peer; other operations to it, and requests to
/// other peers, are admitted, until all eight slots are in use. Each
/// refusal counts as busy.
#[tokio::test]
async fn the_same_request_or_a_second_one_a_success_answers_is_busy() {
    let ctx = node_ctx().await;
    let timeout = Duration::from_secs(3);
    let begin = |peer, request| ctx.node.begin(peer, request, timeout);
    let refused = |result: Result<_, BvlcExchangeError>, why: ExchangeRefusal| match result
        .map(drop)
        .unwrap_err()
    {
        BvlcExchangeError::Refused(refusal) => assert_eq!(refusal, why),
        other => panic!("not refused: {other}"),
    };
    let entry = at(99);
    let mut held = vec![
        begin(at(1), Bvll::ReadBroadcastDistributionTable).unwrap(),
        begin(at(1), Bvll::RegisterForeignDevice { ttl: 60 }).unwrap(),
        begin(at(1), Bvll::ReadForeignDeviceTable).unwrap(),
        begin(at(2), Bvll::ReadBroadcastDistributionTable).unwrap(),
    ];
    refused(
        begin(at(1), Bvll::ReadBroadcastDistributionTable),
        ExchangeRefusal::SameExchange,
    );
    refused(
        begin(at(1), Bvll::DeleteForeignDeviceTableEntry(entry)),
        ExchangeRefusal::ResultClass,
    );
    for last in 3..7 {
        held.push(begin(at(last), Bvll::DeleteForeignDeviceTableEntry(entry)).unwrap());
    }
    // Eight are in flight.
    refused(
        begin(at(7), Bvll::ReadBroadcastDistributionTable),
        ExchangeRefusal::Full,
    );
    let snapshot = ctx.node.snapshot();
    assert_eq!(snapshot.read_bdt.busy, 2);
    assert_eq!(snapshot.delete_fdt_entry.busy, 1);
    // One ends, and its slot takes the next.
    held.pop();
    held.push(begin(at(7), Bvll::ReadBroadcastDistributionTable).unwrap());
}

/// A table reader taken before start reads through the transport's own
/// socket while it runs, keeps a NAK's code, is counted in the transport's
/// snapshot, fails as a read before start does while the transport is
/// stopped, and reads again after a restart (#2612).
#[tokio::test]
async fn a_table_reader_reads_through_the_running_transport() {
    let (peer, mac) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let reader = transport.bvlc_table_reader();
    let not_started = |error: BvlcExchangeError| match error {
        BvlcExchangeError::Transport(Error::State(Lifecycle::NotRunning, _)) => {}
        other => panic!("not the not-started error: {other}"),
    };
    not_started(reader.read_bdt(&mac).await.unwrap_err());
    // One BDT row: 127.0.0.2:47808, mask 255.255.255.255.
    let row = [127, 0, 0, 2, 0xBA, 0xC0, 255, 255, 255, 255];
    let mut ack = vec![0x81, 3, 0, 14];
    ack.extend_from_slice(&row);
    for round in 1..=2u64 {
        let _rx = transport.start().await.unwrap();
        let local = transport.local_mac().to_vec();
        let answer = async {
            let (request, source) = received(&peer).await;
            assert_eq!(request, [0x81, 2, 0, 4]);
            peer.send_to(&ack, source).await.unwrap();
            source
        };
        let (rows, source) = tokio::join!(reader.read_bdt(&mac), answer);
        let rows = rows.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].ip, rows[0].port), ([127, 0, 0, 2], 47808));
        // The request left from the transport's own socket.
        assert_eq!(source.port(), u16::from_be_bytes([local[4], local[5]]));
        // A Read-FDT NAK keeps its result code.
        let answer = async {
            let (request, source) = received(&peer).await;
            assert_eq!(request, [0x81, 6, 0, 4]);
            peer.send_to(&[0x81, 0, 0, 6, 0x00, 0x40], source)
                .await
                .unwrap();
        };
        let (refused, ()) = tokio::join!(reader.read_fdt(&mac), answer);
        assert_eq!(
            refused.unwrap_err().nak().map(|code| code.to_raw()),
            Some(0x0040)
        );
        let snapshot = transport.bvlc_client_snapshot();
        assert_eq!(snapshot.read_bdt.acknowledgements, round);
        assert_eq!(snapshot.read_fdt.results, round);
        transport.stop().await.unwrap();
        not_started(reader.read_fdt(&mac).await.unwrap_err());
    }
}

/// A read given up from elsewhere frees its slot at once, before its future
/// is dropped, so the same read can go out right after; one given up before
/// it starts sends nothing (#2612).
#[tokio::test]
async fn a_given_up_read_frees_its_slot_at_once() {
    let (peer, mac) = peer().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    let reader = transport.bvlc_table_reader();
    let give_up = BvlcGiveUp::default();
    let given_up = reader.with_give_up(&give_up);
    let first = given_up.read_bdt(&mac);
    tokio::pin!(first);
    tokio::select! {
        result = &mut first => panic!("the read ended unanswered: {result:?}"),
        (request, _) = received(&peer) => assert_eq!(request, [0x81, 2, 0, 4]),
    }
    give_up.give_up();
    // The first read's future is still alive; its slot is free anyway.
    let answer = async {
        let (request, source) = received(&peer).await;
        assert_eq!(request, [0x81, 2, 0, 4]);
        peer.send_to(&[0x81, 3, 0, 4], source).await.unwrap();
    };
    let (rows, ()) = tokio::join!(reader.read_bdt(&mac), answer);
    assert!(rows.unwrap().is_empty());
    // Given up while it waited: the given-up error, not a stop's.
    let given_up = |error: BvlcExchangeError| match error {
        BvlcExchangeError::Transport(Error::State(Lifecycle::Closed, message)) => {
            assert!(message.to_string().contains("given up"), "{message}");
        }
        other => panic!("not the given-up error: {other}"),
    };
    given_up(first.await.unwrap_err());
    // Given up before it starts: the same error, and nothing is sent.
    let early = BvlcGiveUp::default();
    early.give_up();
    given_up(
        reader
            .with_give_up(&early)
            .read_fdt(&mac)
            .await
            .unwrap_err(),
    );
    let mut packet = [0; 16];
    assert!(
        tokio::time::timeout(Duration::from_millis(100), peer.recv_from(&mut packet))
            .await
            .is_err()
    );
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(snapshot.read_bdt.sent, 2);
    assert_eq!(snapshot.read_fdt.sent, 0);
    transport.stop().await.unwrap();
}
