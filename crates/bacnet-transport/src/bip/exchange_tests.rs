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
    assert_eq!(registered.unwrap(), BvlcResultCode::SUCCESSFUL_COMPLETION);
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
    let refused = |result: Result<_, Error>, why: &str| {
        let error = result.map(drop).unwrap_err();
        assert!(error.to_string().contains(why), "{error}");
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
        "already in flight",
    );
    refused(
        begin(at(1), Bvll::DeleteForeignDeviceTableEntry(entry)),
        "already in flight",
    );
    for last in 3..7 {
        held.push(begin(at(last), Bvll::DeleteForeignDeviceTableEntry(entry)).unwrap());
    }
    // Eight are in flight.
    refused(
        begin(at(7), Bvll::ReadBroadcastDistributionTable),
        "every BVLC exchange slot is in use",
    );
    let snapshot = ctx.node.snapshot();
    assert_eq!(snapshot.read_bdt.busy, 2);
    assert_eq!(snapshot.delete_fdt_entry.busy, 1);
    // One ends, and its slot takes the next.
    held.pop();
    held.push(begin(at(7), Bvll::ReadBroadcastDistributionTable).unwrap());
}
