use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort};
use bacnet_types::enums::NetworkPriority;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, oneshot, Notify};
use tokio::time::{timeout, Duration};

use super::*;

const WAIT: Duration = Duration::from_secs(1);

struct TestTransport {
    receiver: Option<mpsc::Receiver<ReceivedNpdu>>,
    sent: mpsc::Sender<(Vec<u8>, Vec<u8>)>,
    starts: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
    local_mac: MacAddr,
}

struct TestTransportHandle {
    sender: mpsc::Sender<ReceivedNpdu>,
    sent: mpsc::Receiver<(Vec<u8>, Vec<u8>)>,
    starts: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
}

fn test_transport() -> (TestTransport, TestTransportHandle) {
    let (sender, receiver) = mpsc::channel(32);
    let (sent_tx, sent_rx) = mpsc::channel(32);
    let starts = Arc::new(AtomicUsize::new(0));
    let stops = Arc::new(AtomicUsize::new(0));
    (
        TestTransport {
            receiver: Some(receiver),
            sent: sent_tx,
            starts: Arc::clone(&starts),
            stops: Arc::clone(&stops),
            local_mac: MacAddr::from_slice(&[0xaa]),
        },
        TestTransportHandle {
            sender,
            sent: sent_rx,
            starts,
            stops,
        },
    )
}

impl TransportPort for TestTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.receiver
            .take()
            .ok_or_else(|| Error::Encoding("test transport already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sent
            .send((npdu.to_vec(), mac.to_vec()))
            .await
            .map_err(|_| Error::Encoding("test sent-frame receiver closed".into()))
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

struct BlockingTransport {
    receiver: Option<mpsc::Receiver<ReceivedNpdu>>,
    entered_send: Arc<Notify>,
    stops: Arc<AtomicUsize>,
    local_mac: MacAddr,
}

struct BlockingTransportHandle {
    sender: mpsc::Sender<ReceivedNpdu>,
    entered_send: Arc<Notify>,
    stops: Arc<AtomicUsize>,
}

fn blocking_transport() -> (BlockingTransport, BlockingTransportHandle) {
    let (sender, receiver) = mpsc::channel(1);
    let entered_send = Arc::new(Notify::new());
    let stops = Arc::new(AtomicUsize::new(0));
    (
        BlockingTransport {
            receiver: Some(receiver),
            entered_send: Arc::clone(&entered_send),
            stops: Arc::clone(&stops),
            local_mac: MacAddr::from_slice(&[0xaa]),
        },
        BlockingTransportHandle {
            sender,
            entered_send,
            stops,
        },
    )
}

impl TransportPort for BlockingTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.receiver
            .take()
            .ok_or_else(|| Error::Encoding("blocking transport already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        self.entered_send.notify_one();
        std::future::pending::<()>().await;
        unreachable!("blocking test send is cancelled by endpoint stop")
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

fn npdu_bytes(apdu: &[u8]) -> Bytes {
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        payload: Bytes::copy_from_slice(apdu),
        ..Npdu::default()
    };
    let mut buffer = BytesMut::new();
    encode_npdu(&mut buffer, &npdu).unwrap();
    buffer.freeze()
}

fn received_npdu(apdu: &[u8]) -> ReceivedNpdu {
    ReceivedNpdu {
        npdu: npdu_bytes(apdu),
        source_mac: MacAddr::from_slice(&[0x11]),
        link_layer_group: false,
        data_attributes: Vec::new(),
        reply_tx: None,
    }
}

async fn inject(handle: &TestTransportHandle, apdu: &[u8]) {
    handle.sender.send(received_npdu(apdu)).await.unwrap();
}

async fn receive<T>(receiver: &mut mpsc::Receiver<T>) -> T {
    timeout(WAIT, receiver.recv())
        .await
        .expect("ingress receive timed out")
        .expect("ingress route closed")
}

#[tokio::test]
async fn loopback_routes_all_apdu_classes_once_and_in_order() {
    let (transport, peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut endpoint = EndpointIngress::new(transport, 8);
    let mut ingress = endpoint.start().await.unwrap();
    let apdus: [&[u8]; 8] = [
        &[0x00, 0x05, 0x11, 0x0c],
        &[0x20, 0x12, 0x0c],
        &[0x10, 0x08],
        &[0x30, 0x13, 0x0c],
        &[0x50, 0x14, 0x0c, 0x91, 0x00, 0x91, 0x00],
        &[0x60, 0x15, 0x00],
        &[0x70, 0x16, 0x00],
        &[0x40, 0x17, 0x00, 0x01],
    ];

    for apdu in apdus {
        peer.send_unicast(&npdu_bytes(apdu), &[0x01]).await.unwrap();
    }

    let mut inbound_types = Vec::new();
    for _ in 0..2 {
        inbound_types.push(receive(&mut ingress.inbound_requests).await.apdu[0] >> 4);
    }
    let mut terminal_types = Vec::new();
    for _ in 0..6 {
        terminal_types.push(receive(&mut ingress.terminal_or_segment).await.apdu[0] >> 4);
    }

    assert_eq!(inbound_types, [0, 1]);
    assert_eq!(terminal_types, [2, 3, 5, 6, 7, 4]);
    assert!(ingress.inbound_requests.try_recv().is_err());
    assert!(ingress.terminal_or_segment.try_recv().is_err());
    assert!(ingress.policy_outcomes.try_recv().is_err());
    assert!(matches!(
        endpoint.stop().await.unwrap(),
        ClassifierExit::Cancelled
    ));
}

#[tokio::test]
async fn direct_egress_preserves_npdu_flags_priority_and_destination() {
    let (transport, mut handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress.clone();

    egress
        .send_direct(
            vec![0x20, 0x2a, 0x0c],
            MacAddr::from_slice(&[0x44, 0x55]),
            true,
            NetworkPriority::URGENT,
        )
        .await
        .unwrap();

    let (npdu, destination) = timeout(WAIT, handle.sent.recv()).await.unwrap().unwrap();
    let decoded = decode_npdu(Bytes::from(npdu)).unwrap();
    assert_eq!(destination, [0x44, 0x55]);
    assert!(decoded.expecting_reply);
    assert_eq!(decoded.priority, NetworkPriority::URGENT);
    assert_eq!(decoded.payload.as_ref(), &[0x20, 0x2a, 0x0c]);

    endpoint.stop().await.unwrap();
    assert!(matches!(
        egress
            .send_direct(
                vec![0x20, 0x2b, 0x0c],
                MacAddr::from_slice(&[0x44]),
                false,
                NetworkPriority::NORMAL,
            )
            .await,
        Err(Error::Encoding(message)) if message == "endpoint shutdown"
    ));
}

#[tokio::test]
async fn saturated_egress_is_bounded_and_stop_cancels_accepted_commands() {
    let (transport, handle) = blocking_transport();
    let mut endpoint = EndpointIngress::new(transport, 1);
    let mut ingress = endpoint.start().await.unwrap();
    let egress = ingress.egress.clone();

    let first_egress = egress.clone();
    let first = tokio::spawn(async move {
        first_egress
            .send_direct(
                vec![0x20, 0x01, 0x0c],
                MacAddr::from_slice(&[0x01]),
                false,
                NetworkPriority::NORMAL,
            )
            .await
    });
    timeout(WAIT, handle.entered_send.notified()).await.unwrap();

    let second_egress = egress.clone();
    let second = tokio::spawn(async move {
        second_egress
            .send_direct(
                vec![0x20, 0x02, 0x0c],
                MacAddr::from_slice(&[0x01]),
                false,
                NetworkPriority::NORMAL,
            )
            .await
    });
    tokio::task::yield_now().await;

    assert!(matches!(
        egress
            .send_direct(
                vec![0x20, 0x03, 0x0c],
                MacAddr::from_slice(&[0x01]),
                false,
                NetworkPriority::NORMAL,
            )
            .await,
        Err(Error::Encoding(message)) if message == "endpoint egress queue is full"
    ));

    handle
        .sender
        .send(received_npdu(&[0x20, 0x44, 0x0c]))
        .await
        .unwrap();
    let terminal = timeout(WAIT, ingress.terminal_or_segment.recv())
        .await
        .expect("blocked egress starved terminal classification")
        .expect("terminal route closed while egress was blocked");
    assert_eq!(terminal.apdu.as_ref(), &[0x20, 0x44, 0x0c]);

    assert!(matches!(
        timeout(WAIT, endpoint.stop()).await.unwrap().unwrap(),
        ClassifierExit::Cancelled
    ));
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
    for result in [first.await.unwrap(), second.await.unwrap()] {
        assert!(matches!(
            result,
            Err(Error::Encoding(message)) if message == "endpoint shutdown"
        ));
    }
}

#[tokio::test]
async fn malformed_and_unsupported_apdus_have_policy_outcomes() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 4);
    let mut ingress = endpoint.start().await.unwrap();

    inject(&handle, &[]).await;
    inject(&handle, &[0x80]).await;

    let malformed = receive(&mut ingress.policy_outcomes).await;
    assert_eq!(malformed.reason, PolicyReason::MalformedApdu);
    assert!(malformed.received.apdu.is_empty());
    let unsupported = receive(&mut ingress.policy_outcomes).await;
    assert_eq!(unsupported.reason, PolicyReason::UnsupportedPduType(8));
    assert_eq!(unsupported.received.apdu.as_ref(), &[0x80]);
    assert!(ingress.inbound_requests.try_recv().is_err());
    assert!(ingress.terminal_or_segment.try_recv().is_err());

    endpoint.stop().await.unwrap();
}

#[tokio::test]
async fn request_route_preserves_the_complete_envelope_and_reply_sender() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 2);
    let mut ingress = endpoint.start().await.unwrap();
    let apdu = [0x00, 0x05, 0x33, 0x0c];
    let source_network = NpduAddress {
        network: 77,
        mac_address: MacAddr::from_slice(&[0x44, 0x55]),
    };
    let destination = NpduAddress {
        network: 0xffff,
        mac_address: MacAddr::new(),
    };
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: true,
        priority: NetworkPriority::URGENT,
        destination: Some(destination),
        source: Some(source_network.clone()),
        hop_count: 255,
        payload: Bytes::copy_from_slice(&apdu),
        ..Npdu::default()
    };
    let mut buffer = BytesMut::new();
    encode_npdu(&mut buffer, &npdu).unwrap();
    let attributes = vec![DataAttribute {
        option_type: 9,
        must_understand: true,
        data: vec![1, 2, 3],
    }];
    let (reply_tx, reply_rx) = oneshot::channel();
    handle
        .sender
        .send(ReceivedNpdu {
            npdu: buffer.freeze(),
            source_mac: MacAddr::from_slice(&[0xde, 0xad]),
            link_layer_group: true,
            data_attributes: attributes.clone(),
            reply_tx: Some(reply_tx),
        })
        .await
        .unwrap();

    let mut received = receive(&mut ingress.inbound_requests).await;
    assert_eq!(received.apdu.as_ref(), apdu);
    assert_eq!(received.source_mac.as_slice(), &[0xde, 0xad]);
    assert_eq!(received.source_network, Some(source_network));
    assert!(received.link_layer_group);
    assert!(received.is_group);
    assert_eq!(received.data_attributes, attributes);
    received
        .reply_tx
        .take()
        .expect("reply sender was lost")
        .send(Bytes::from_static(b"reply"))
        .unwrap();
    assert_eq!(reply_rx.await.unwrap().as_ref(), b"reply");
    assert!(ingress.terminal_or_segment.try_recv().is_err());
    assert!(ingress.policy_outcomes.try_recv().is_err());

    endpoint.stop().await.unwrap();
}

#[tokio::test]
async fn duplicate_start_is_rejected_before_transport_and_stop_is_single_owner() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 1);
    let mut ingress = endpoint.start().await.unwrap();
    assert_eq!(handle.starts.load(Ordering::SeqCst), 1);

    assert!(endpoint.start().await.is_err());
    assert_eq!(handle.starts.load(Ordering::SeqCst), 1);
    assert!(matches!(
        endpoint.stop().await.unwrap(),
        ClassifierExit::Cancelled
    ));
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
    assert!(endpoint.stop().await.is_err());
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
    assert!(ingress.inbound_requests.recv().await.is_none());
    assert!(ingress.terminal_or_segment.recv().await.is_none());
    assert!(ingress.policy_outcomes.recv().await.is_none());
}

#[tokio::test]
async fn zero_capacity_is_rejected_before_transport_start() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 0);

    assert!(endpoint.start().await.is_err());
    assert_eq!(handle.starts.load(Ordering::SeqCst), 0);
    assert_eq!(handle.stops.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn full_and_closed_role_routes_are_explicit_and_stop_does_not_block() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 1);
    let ingress = endpoint.start().await.unwrap();
    let mut inbound = ingress.inbound_requests;
    let terminal = ingress.terminal_or_segment;
    let mut policies = ingress.policy_outcomes;

    inject(&handle, &[0x00, 0x05, 0x01, 0x0c]).await;
    inject(&handle, &[0x00, 0x05, 0x02, 0x0c]).await;
    let full = receive(&mut policies).await;
    assert_eq!(
        full.reason,
        PolicyReason::RouteFull(IngressRoute::InboundRequest)
    );
    assert_eq!(full.received.apdu[2], 2);

    drop(terminal);
    inject(&handle, &[0x20, 0x03, 0x0c]).await;
    let closed = receive(&mut policies).await;
    assert_eq!(
        closed.reason,
        PolicyReason::RouteClosed(IngressRoute::TerminalOrSegment)
    );
    assert_eq!(closed.received.apdu[1], 3);

    timeout(WAIT, endpoint.stop())
        .await
        .expect("stop blocked on a saturated role route")
        .unwrap();
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
    assert_eq!(inbound.recv().await.unwrap().apdu[2], 1);
    assert!(inbound.recv().await.is_none());
    assert!(policies.recv().await.is_none());
}

#[tokio::test]
async fn full_policy_route_returns_the_unrouted_envelope_on_reclaim() {
    let (transport, handle) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, 1);
    let mut ingress = endpoint.start().await.unwrap();

    inject(&handle, &[0x00, 0x05, 0x01, 0x0c]).await;
    inject(&handle, &[0x00, 0x05, 0x02, 0x0c]).await;
    inject(&handle, &[0x00, 0x05, 0x03, 0x0c]).await;

    assert!(timeout(WAIT, ingress.terminal_or_segment.recv())
        .await
        .expect("classifier did not stop when its policy route filled")
        .is_none());

    let exit = timeout(WAIT, endpoint.stop()).await.unwrap().unwrap();
    match exit {
        ClassifierExit::PolicyRouteFull(outcome) => {
            assert_eq!(
                outcome.reason,
                PolicyReason::RouteFull(IngressRoute::InboundRequest)
            );
            assert_eq!(outcome.received.apdu[2], 3);
        }
        other => panic!("unexpected classifier exit: {other:?}"),
    }
    assert_eq!(handle.stops.load(Ordering::SeqCst), 1);
    drop(ingress);
}
