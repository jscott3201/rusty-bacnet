use super::*;
use crate::layer::QueueAdmissionSnapshot;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_transport::port::ReceivedNpdu;
use tokio::sync::oneshot;
use tokio::time::timeout;

// Real router dispatch/sender tasks, but no sockets or shared port numbers.
// Injection moves ReceivedNpdu, preserving the non-cloneable reply sender.
struct IngressTransport {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    outgoing: mpsc::Sender<SendRequest>,
    mac: [u8; 1],
}

impl TransportPort for IngressTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.incoming
            .take()
            .ok_or_else(|| Error::Encoding("already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.send_unicast_with_data_attributes(npdu, mac, &[]).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_broadcast_with_data_attributes(npdu, &[]).await
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.outgoing
            .try_send(SendRequest::Unicast {
                npdu: Bytes::copy_from_slice(npdu),
                mac: MacAddr::from_slice(mac),
                data_attributes: data_attributes.to_vec(),
            })
            .map_err(|_| Error::Encoding("test wire queue unavailable".into()))
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.outgoing
            .try_send(SendRequest::Broadcast {
                npdu: Bytes::copy_from_slice(npdu),
                data_attributes: data_attributes.to_vec(),
            })
            .map_err(|_| Error::Encoding("test wire queue unavailable".into()))
    }

    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
}

struct Peer {
    tx: mpsc::Sender<ReceivedNpdu>,
    wire: mpsc::Receiver<SendRequest>,
}

fn network(port: usize) -> u16 {
    100 + port as u16
}

fn fixture(count: usize) -> (Vec<RouterPort<IngressTransport>>, Vec<Peer>) {
    (0..count)
        .map(|port| {
            // Only injected transport backlog is larger than the production
            // local queue, so a stalled dispatch cannot hide behind injection.
            let (tx, rx) = mpsc::channel(1024);
            let (outgoing, wire) = mpsc::channel(32);
            (
                RouterPort {
                    transport: IngressTransport {
                        incoming: Some(rx),
                        outgoing,
                        mac: [port as u8 + 1],
                    },
                    network_number: network(port),
                },
                Peer { tx, wire },
            )
        })
        .unzip()
}

#[derive(Clone, Copy, Debug)]
enum LocalBranch {
    GlobalBroadcast,
    DadrMatch,
    RemoteBroadcast,
    NoDnet,
}

const BRANCHES: [LocalBranch; 4] = [
    LocalBranch::GlobalBroadcast,
    LocalBranch::DadrMatch,
    LocalBranch::RemoteBroadcast,
    LocalBranch::NoDnet,
];

fn address(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}

fn incoming(destination: Option<NpduAddress>, id: u16) -> ReceivedNpdu {
    let npdu = Npdu {
        destination,
        source: Some(address(300, &[0x55])),
        hop_count: 10,
        expecting_reply: true,
        payload: Bytes::copy_from_slice(&id.to_be_bytes()),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &npdu).unwrap();
    ReceivedNpdu {
        npdu: bytes.freeze(),
        source_mac: MacAddr::from_slice(&[0xA0]),
        link_layer_group: true,
        data_attributes: vec![DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34],
        }],
        reply_tx: None,
    }
}

fn local(branch: LocalBranch, port: usize, id: u16) -> ReceivedNpdu {
    let destination = match branch {
        LocalBranch::GlobalBroadcast => Some(address(0xFFFF, &[])),
        LocalBranch::DadrMatch => Some(address(network(port), &[port as u8 + 1])),
        LocalBranch::RemoteBroadcast => Some(address(network(port), &[])),
        LocalBranch::NoDnet => None,
    };
    incoming(destination, id)
}

async fn wire(peer: &mut Peer) -> SendRequest {
    timeout(Duration::from_secs(2), peer.wire.recv())
        .await
        .expect("router dispatch/sender stalled")
        .expect("wire closed")
}

async fn drain_announcements(peers: &mut [Peer]) {
    for peer in peers {
        let SendRequest::Broadcast { npdu, .. } = wire(peer).await else {
            panic!("expected startup announcement")
        };
        assert_eq!(
            decode_npdu(npdu).unwrap().message_type,
            Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
        );
    }
}

// An unknown-route reject behind local arrivals is a FIFO dispatch barrier.
// It also proves local overload did not stop the existing reject path.
async fn barrier(peer: &mut Peer) {
    peer.tx
        .send(incoming(Some(address(9000, &[9])), 0))
        .await
        .unwrap();
    let SendRequest::Unicast {
        npdu,
        mac,
        data_attributes,
    } = wire(peer).await
    else {
        panic!("expected unknown-route reject")
    };
    let npdu = decode_npdu(npdu).unwrap();
    assert_eq!(mac.as_slice(), &[0xA0]);
    assert_eq!(
        npdu.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
    );
    assert_eq!(
        npdu.payload.as_ref(),
        [
            RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw(),
            0x23,
            0x28
        ]
    );
    assert!(data_attributes.is_empty());
}

async fn branch_forward(peers: &mut [Peer], branch: LocalBranch, port: usize, id: u16) {
    let target = match branch {
        LocalBranch::GlobalBroadcast => 1 - port,
        LocalBranch::RemoteBroadcast => port,
        _ => return,
    };
    let SendRequest::Broadcast {
        npdu,
        data_attributes,
    } = wire(&mut peers[target]).await
    else {
        panic!("expected broadcast for {branch:?}")
    };
    assert_eq!(
        decode_npdu(npdu).unwrap().payload.as_ref(),
        id.to_be_bytes()
    );
    assert_eq!(data_attributes, incoming(None, id).data_attributes);
}

async fn forwarding_progress(peers: &mut [Peer], port: usize) {
    let target = 1 - port;
    peers[port]
        .tx
        .send(incoming(Some(address(network(target), &[9])), 1000))
        .await
        .unwrap();
    let SendRequest::Unicast { npdu, mac, .. } = wire(&mut peers[target]).await else {
        panic!("expected forwarded unicast")
    };
    assert_eq!(mac.as_slice(), &[9]);
    assert_eq!(
        decode_npdu(npdu).unwrap().payload.as_ref(),
        1000u16.to_be_bytes()
    );
    barrier(&mut peers[port]).await;
}

fn assert_quiet(peers: &mut [Peer]) {
    for peer in peers {
        assert!(matches!(
            peer.wire.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}

fn assert_apdu(apdu: &ReceivedApdu, branch: LocalBranch, id: u16) {
    assert_eq!(apdu.apdu.as_ref(), id.to_be_bytes());
    assert_eq!(apdu.source_mac, incoming(None, id).source_mac);
    assert_eq!(apdu.source_network, Some(address(300, &[0x55])));
    assert!(apdu.link_layer_group);
    assert_eq!(apdu.is_group, !matches!(branch, LocalBranch::DadrMatch));
    assert_eq!(apdu.data_attributes, incoming(None, id).data_attributes);
}

async fn fill(peers: &mut [Peer]) -> oneshot::Receiver<Bytes> {
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut first = incoming(None, 0);
    first.reply_tx = Some(reply_tx);
    peers[0].tx.send(first).await.unwrap();
    for (port, peer) in peers.iter_mut().enumerate() {
        for id in (port * 128).max(1)..(port + 1) * 128 {
            peer.tx.send(incoming(None, id as u16)).await.unwrap();
        }
        barrier(peer).await;
    }
    reply_rx
}

async fn dropped_arrival(peers: &mut [Peer], branch: LocalBranch, port: usize) {
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut arrival = local(branch, port, 256);
    arrival.reply_tx = Some(reply_tx);
    peers[port].tx.send(arrival).await.unwrap();
    branch_forward(peers, branch, port, 256).await;
    forwarding_progress(peers, port).await;
    assert!(timeout(Duration::from_secs(2), reply_rx)
        .await
        .expect("dropped reply sender retained")
        .is_err());
    assert_quiet(peers); // No wire rejection for the local admission drop.
}

#[tokio::test(start_paused = true)]
async fn full_shared_local_queue_drops_arrivals_in_all_four_branches_and_keeps_forwarding() {
    for branch in BRANCHES {
        let (ports, mut peers) = fixture(2);
        let (mut router, mut apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
        let counters = apdus.counters();
        drain_announcements(&mut peers).await;
        let mut accepted_reply = fill(&mut peers).await;
        for port in 0..2 {
            dropped_arrival(&mut peers, branch, port).await;
            assert_eq!(
                counters.snapshot(),
                QueueAdmissionSnapshot {
                    current_depth: 256,
                    high_water: 256,
                    full_drops: port as u64 + 1,
                    fairness_drops: 0,
                    closed_drops: 0,
                }
            );
        }
        assert_eq!(
            accepted_reply.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
        for id in 0..256 {
            let apdu = apdus.try_recv().unwrap();
            assert_apdu(&apdu, LocalBranch::NoDnet, id);
            if id == 0 {
                apdu.reply_tx
                    .unwrap()
                    .send(Bytes::from_static(b"reply"))
                    .unwrap();
            }
            assert_eq!(counters.snapshot().current_depth, 255 - usize::from(id));
        }
        assert_eq!(accepted_reply.await.unwrap(), Bytes::from_static(b"reply"));
        assert!(matches!(
            apdus.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        peers[0].tx.send(incoming(None, 257)).await.unwrap();
        let apdu = timeout(Duration::from_secs(2), apdus.recv())
            .await
            .unwrap()
            .unwrap();
        assert_apdu(&apdu, LocalBranch::NoDnet, 257);
        assert_eq!(
            counters.snapshot(),
            QueueAdmissionSnapshot {
                high_water: 256,
                full_drops: 2,
                ..Default::default()
            }
        );
        router.stop().await;
    }
}

#[tokio::test(start_paused = true)]
async fn admitted_local_branches_preserve_metadata_and_reply_ownership() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
    drain_announcements(&mut peers).await;
    for branch in BRANCHES {
        let (reply_tx, mut reply_rx) = oneshot::channel();
        let mut arrival = local(branch, 0, 42);
        arrival.reply_tx = Some(reply_tx);
        peers[0].tx.send(arrival).await.unwrap();
        branch_forward(&mut peers, branch, 0, 42).await;
        barrier(&mut peers[0]).await;
        assert_eq!(apdus.counters().snapshot().current_depth, 1);
        let apdu = apdus.recv().await.unwrap();
        assert_apdu(&apdu, branch, 42);
        if matches!(branch, LocalBranch::RemoteBroadcast) {
            assert!(apdu.reply_tx.is_none());
            assert!(reply_rx.await.is_err());
        } else {
            assert_eq!(
                reply_rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            );
            apdu.reply_tx
                .unwrap()
                .send(Bytes::from_static(b"reply"))
                .unwrap();
            assert_eq!(reply_rx.await.unwrap(), Bytes::from_static(b"reply"));
        }
        assert_eq!(
            apdus.counters().snapshot(),
            QueueAdmissionSnapshot {
                high_water: 1,
                ..Default::default()
            }
        );
    }
    assert_quiet(&mut peers);
    router.stop().await;
}

#[tokio::test(start_paused = true)]
async fn closing_full_local_queue_counts_each_closed_arrival_and_preserves_drain() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    let mut accepted_reply = fill(&mut peers).await;
    apdus.close();
    for branch in BRANCHES {
        for port in 0..2 {
            dropped_arrival(&mut peers, branch, port).await;
        }
    }
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 0,
            fairness_drops: 0,
            closed_drops: 8,
        }
    );
    assert_eq!(
        accepted_reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    for id in 0..256 {
        let apdu = if id % 2 == 0 {
            apdus.recv().await.unwrap()
        } else {
            apdus.try_recv().unwrap()
        };
        assert_apdu(&apdu, LocalBranch::NoDnet, id);
        assert_eq!(counters.snapshot().current_depth, 255 - usize::from(id));
    }
    assert!(accepted_reply.await.is_err());
    assert!(apdus.recv().await.is_none());
    assert!(matches!(
        apdus.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            closed_drops: 8,
            ..Default::default()
        }
    );
    router.stop().await;
}

#[tokio::test(start_paused = true)]
async fn dropping_local_receiver_releases_queued_replies_and_keeps_dispatch_alive() {
    let (ports, mut peers) = fixture(2);
    let (mut router, apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    let accepted_reply = fill(&mut peers).await;
    drop(apdus);
    assert!(accepted_reply.await.is_err());
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            ..Default::default()
        }
    );
    for port in 0..2 {
        dropped_arrival(&mut peers, LocalBranch::NoDnet, port).await;
    }
    assert_eq!(counters.snapshot().closed_drops, 2);
    router.stop().await;
    drop(router);
    assert_eq!(counters.snapshot().current_depth, 0);
}

#[tokio::test(start_paused = true)]
async fn stop_with_full_local_queue_is_bounded_and_leaves_items_drainable() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    let mut accepted_reply = fill(&mut peers).await;
    timeout(Duration::from_secs(2), router.stop())
        .await
        .unwrap();
    assert_eq!(
        accepted_reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    assert_eq!(counters.snapshot().current_depth, 256);
    assert!(peers.iter().all(|peer| peer.tx.is_closed()));
    for id in 0..256 {
        assert_apdu(&apdus.recv().await.unwrap(), LocalBranch::NoDnet, id);
    }
    assert!(apdus.recv().await.is_none());
    assert!(accepted_reply.await.is_err());
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            ..Default::default()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn legacy_local_receiver_retains_type_and_nonblocking_full_closed_policy() {
    let (ports, mut peers) = fixture(2);
    let (mut router, mut apdus): (_, mpsc::Receiver<ReceivedApdu>) =
        BACnetRouter::start(ports).await.unwrap();
    drain_announcements(&mut peers).await;
    let accepted_reply = fill(&mut peers).await;
    for branch in BRANCHES {
        dropped_arrival(&mut peers, branch, 0).await;
    }
    assert_eq!(apdus.len(), 256);
    for id in 0..256 {
        assert_apdu(&apdus.try_recv().unwrap(), LocalBranch::NoDnet, id);
    }
    assert!(accepted_reply.await.is_err());
    apdus.close();
    for branch in BRANCHES {
        dropped_arrival(&mut peers, branch, 1).await;
    }
    assert!(apdus.recv().await.is_none());
    router.stop().await;
}

#[tokio::test]
async fn concurrent_ports_share_one_capacity_and_exact_depth_during_receive() {
    let (ports, mut peers) = fixture(4);
    let (mut router, mut apdus) = BACnetRouter::start_with_admission(ports).await.unwrap();
    let counters = apdus.counters();
    drain_announcements(&mut peers).await;
    let start = Arc::new(tokio::sync::Barrier::new(4));
    let mut producers = Vec::new();
    for (port, mut peer) in peers.into_iter().enumerate() {
        let start = Arc::clone(&start);
        producers.push(tokio::spawn(async move {
            start.wait().await;
            for id in 0..256 {
                peer.tx
                    .send(incoming(None, (port * 256 + id) as u16))
                    .await
                    .unwrap();
            }
            barrier(&mut peer).await;
            peer
        }));
    }
    let mut peers = Vec::new();
    for producer in producers {
        peers.push(
            timeout(Duration::from_secs(5), producer)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 768,
            fairness_drops: 0,
            closed_drops: 0,
        }
    );
    let mut seen = std::collections::HashSet::new();
    while let Ok(apdu) = apdus.try_recv() {
        assert!(seen.insert(u16::from_be_bytes(apdu.apdu.as_ref().try_into().unwrap())));
    }
    assert_eq!(seen.len(), 256);
    assert_eq!(counters.snapshot().current_depth, 0);

    // Four live senders race the accounting receiver. At most 256 total items
    // arrive, so all must be received even with adversarial scheduling.
    let mut producers = Vec::new();
    for (port, mut peer) in peers.into_iter().enumerate() {
        producers.push(tokio::spawn(async move {
            for id in 0..64 {
                peer.tx
                    .send(incoming(None, (port * 64 + id) as u16))
                    .await
                    .unwrap();
            }
            barrier(&mut peer).await;
        }));
    }
    seen.clear();
    for _ in 0..256 {
        let apdu = timeout(Duration::from_secs(5), apdus.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(seen.insert(u16::from_be_bytes(apdu.apdu.as_ref().try_into().unwrap())));
        assert!(counters.snapshot().current_depth <= 256);
    }
    for producer in producers {
        timeout(Duration::from_secs(5), producer)
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            full_drops: 768,
            ..Default::default()
        }
    );
    router.stop().await;
}

#[test]
fn cloned_senders_account_exactly_across_threads_and_concurrent_dequeues() {
    let (tx, rx, counters) = AdmissionReceiver::<u16>::channel(true);
    let mut apdus = AdmissionReceiver::from_parts(rx, counters.clone());
    let start = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        for port in 0..4 {
            let tx = tx.clone();
            let start = &start;
            scope.spawn(move || {
                start.wait();
                for id in 0..256 {
                    let _ = tx.try_send(port * 256 + id);
                }
            });
        }
    });
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 768,
            fairness_drops: 0,
            closed_drops: 0,
        }
    );
    for _ in 0..256 {
        apdus.try_recv().unwrap();
    }

    let start = std::sync::Barrier::new(5);
    std::thread::scope(|scope| {
        for port in 0..4 {
            let tx = tx.clone();
            let start = &start;
            scope.spawn(move || {
                start.wait();
                for id in 0..64 {
                    tx.try_send(port * 64 + id).unwrap();
                }
            });
        }
        start.wait();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = std::collections::HashSet::new();
        while seen.len() < 256 {
            match apdus.try_recv() {
                Ok(id) => assert!(seen.insert(id)),
                Err(mpsc::error::TryRecvError::Empty) => {
                    assert!(std::time::Instant::now() < deadline, "sender stalled");
                    std::thread::yield_now();
                }
                Err(e) => panic!("unexpected receive error: {e}"),
            }
            assert!(counters.snapshot().current_depth <= 256);
        }
    });
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            full_drops: 768,
            ..Default::default()
        }
    );
}
