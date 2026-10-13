//! A request the node refuses with a NAK draws none when no reply could
//! reach its source (#1622), as on B/IPv6: the unspecified address, port 0,
//! or this node's own address, which never sends it a request. A group
//! source is dropped before its frame is read (`group_source_tests.rs`).
//! Each is counted, and the same request from a peer still gets its NAK.

use super::send_sink::SendSink;
use super::*;

const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 0xBAC0);
const BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 0xBAC0);
const PEER: ([u8; 4], u16) = ([192, 0, 2, 30], 0xBAC0);
/// Sources a reply can't reach.
const UNANSWERABLE: [([u8; 4], u16); 3] = [
    ([0, 0, 0, 0], 0xBAC0),
    ([192, 0, 2, 30], 0),
    ([192, 0, 2, 10], 0xBAC0),
];

/// A node at 192.0.2.10:47808, a BBMD when `bbmd` is set, and what it sends.
async fn node(bbmd: bool) -> (RecvContext, SendSink) {
    let sink = SendSink::default();
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
    let (npdu_tx, _) = mpsc::channel(4);
    let state = bbmd.then(|| {
        let mut state = BbmdState::new(OWN.ip().octets(), OWN.port());
        state.enable_foreign_device_registration(ForeignDevicePolicy::default());
        Arc::new(std::sync::Mutex::new(state))
    });
    let ctx = RecvContext::for_test(socket, npdu_tx, OWN, BROADCAST, state);
    (ctx, sink)
}

/// Write-BDT and the five requests only a BBMD serves, well formed, with
/// the NAK each draws.
fn requests() -> [(BvllMessage, u16); 6] {
    let message = |function, payload: &[u8]| BvllMessage {
        function,
        payload: bytes::Bytes::copy_from_slice(payload),
    };
    let row = [10, 0, 0, 1, 0xBA, 0xC0, 255, 255, 255, 255];
    let entry = [192, 0, 2, 40, 0xBA, 0xC0];
    [
        (
            message(BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE, &row),
            0x10,
        ),
        (
            message(BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE, &[]),
            0x20,
        ),
        (
            message(BvlcFunction::REGISTER_FOREIGN_DEVICE, &[0, 60]),
            0x30,
        ),
        (message(BvlcFunction::READ_FOREIGN_DEVICE_TABLE, &[]), 0x40),
        (
            message(BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY, &entry),
            0x50,
        ),
        (
            message(BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK, &[1, 0]),
            0x60,
        ),
    ]
}

/// The same requests with a payload that doesn't fit their function (any
/// payload fits a Distribute-Broadcast-To-Network).
fn malformed() -> Vec<BvllMessage> {
    requests()
        .into_iter()
        .filter(|(request, _)| request.function != BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK)
        .map(|(request, _)| BvllMessage {
            function: request.function,
            payload: bytes::Bytes::from_static(&[0; 3]),
        })
        .collect()
}

#[tokio::test]
async fn no_nak_goes_to_a_source_no_reply_can_reach() {
    let (ctx, sink) = node(false).await;
    for source in UNANSWERABLE {
        for (request, _) in requests() {
            handle_bvll_message(&request, source, Delivery::Unicast, &ctx).await;
        }
        for request in malformed() {
            handle_bvll_message(&request, source, Delivery::Unicast, &ctx).await;
        }
    }
    assert_eq!(sink.take(), [], "nothing is sent");
    let counters = ctx.drops.snapshot();
    // The node's own Distribute-Broadcast-To-Network is its own echo.
    let expected = ReceiveCounters {
        unanswerable_request: 3 * 6 - 1,
        own_echo: 1,
        malformed_payload: 3 * 5,
        ..ReceiveCounters::default()
    };
    assert_eq!(counters, expected);

    // From a peer, each draws its own NAK.
    for (request, code) in requests() {
        handle_bvll_message(&request, PEER, Delivery::Unicast, &ctx).await;
        let sent = sink.take();
        assert_eq!(sent.len(), 1, "{:?}", request.function);
        assert_eq!(
            sent[0].to,
            SocketAddrV4::new(Ipv4Addr::from(PEER.0), PEER.1)
        );
        assert_eq!(sent[0].bytes, [0x81, 0x00, 0x00, 0x06, 0x00, code as u8]);
    }
    for request in malformed() {
        handle_bvll_message(&request, PEER, Delivery::Unicast, &ctx).await;
        assert_eq!(sink.take().len(), 1, "{:?}", request.function);
    }
}

/// A BBMD still refuses Write-BDT with its NAK, but not to such a source;
/// the five it serves are its own to answer, as before.
#[tokio::test]
async fn a_bbmd_refuses_write_bdt_only_where_a_reply_can_reach() {
    let (ctx, sink) = node(true).await;
    let (write, code) = requests()[0].clone();
    for source in UNANSWERABLE {
        handle_bvll_message(&write, source, Delivery::Unicast, &ctx).await;
    }
    assert_eq!(sink.take(), []);
    assert_eq!(ctx.drops.snapshot().unanswerable_request, 3);
    handle_bvll_message(&write, PEER, Delivery::Unicast, &ctx).await;
    let sent = sink.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].bytes, [0x81, 0x00, 0x00, 0x06, 0x00, code as u8]);
}
