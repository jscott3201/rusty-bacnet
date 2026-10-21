//! A real router over injected ingress, shared by the router's local-admission
//! tests (`admission_tests` and the modules under it).
//!
//! The router's dispatch and sender tasks run as in production, but each port
//! is an [`IngressTransport`]: the test hands it received frames through a
//! [`Peer`] and reads back what the router queued for the wire, with no
//! sockets or shared port numbers.

use super::*;
use bacnet_encoding::npdu::{decode_npdu, NpduAddress};
use bacnet_npdu::RejectMessageReason;
use bacnet_transport::port::ReceivedNpdu;
use tokio::sync::oneshot;
use tokio::time::timeout;

// Injection moves ReceivedNpdu, preserving the non-cloneable reply sender.
pub(super) struct IngressTransport {
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

                response: None,
                reject_origin: None,
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

                reject_origin: None,
            })
            .map_err(|_| Error::Encoding("test wire queue unavailable".into()))
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
}

/// The test's side of one [`IngressTransport`] port.
pub(super) struct Peer {
    pub(super) tx: mpsc::Sender<ReceivedNpdu>,
    pub(super) wire: mpsc::Receiver<SendRequest>,
}

pub(super) fn network(port: usize) -> u16 {
    100 + port as u16
}

pub(super) fn fixture(count: usize) -> (Vec<RouterPort<IngressTransport>>, Vec<Peer>) {
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

/// Start a router on `ports`, set up as `options` asks.
pub(super) async fn launch<A: LocalApduReceiver>(
    ports: Vec<RouterPort<IngressTransport>>,
    options: RouterOptions<A>,
) -> (BACnetRouter, A) {
    let started = BACnetRouter::start(ports, options).await.unwrap();
    (started.router, started.apdus)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum LocalBranch {
    GlobalBroadcast,
    DadrMatch,
    RemoteBroadcast,
    NoDnet,
}

pub(super) const BRANCHES: [LocalBranch; 4] = [
    LocalBranch::GlobalBroadcast,
    LocalBranch::DadrMatch,
    LocalBranch::RemoteBroadcast,
    LocalBranch::NoDnet,
];

pub(super) fn address(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}

/// The opaque APDU a test arrival `id` carries: an Unconfirmed-Request type
/// octet, the only type a broadcast may carry (#1491), then the id.
pub(super) fn apdu_of(id: u16) -> [u8; 3] {
    let [high, low] = id.to_be_bytes();
    [0x10, high, low]
}

/// The id [`apdu_of`] put in `apdu`.
pub(super) fn id_of(apdu: &[u8]) -> u16 {
    u16::from_be_bytes([apdu[1], apdu[2]])
}

pub(super) fn incoming(destination: Option<NpduAddress>, id: u16) -> ReceivedNpdu {
    let npdu = Npdu {
        destination,
        source: Some(address(300, &[0x55])),
        hop_count: 10,
        expecting_reply: true,
        payload: Bytes::copy_from_slice(&apdu_of(id)),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &npdu).unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: bytes.freeze(),
        // Capacity tests use at most 16 APDUs per key, so Full attribution
        // remains independent of the tracked receiver's per-source quota.
        source_mac: MacAddr::from_slice(&[0xA0 + (id / 16) as u8]),
        link_layer_group: true,
        data_attributes: vec![DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34],
        }],
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

pub(super) fn local(branch: LocalBranch, port: usize, id: u16) -> ReceivedNpdu {
    let destination = match branch {
        LocalBranch::GlobalBroadcast => Some(address(0xFFFF, &[])),
        LocalBranch::DadrMatch => Some(address(network(port), &[port as u8 + 1])),
        LocalBranch::RemoteBroadcast => Some(address(network(port), &[])),
        LocalBranch::NoDnet => None,
    };
    incoming(destination, id)
}

pub(super) async fn wire(peer: &mut Peer) -> SendRequest {
    timeout(Duration::from_secs(2), peer.wire.recv())
        .await
        .expect("router dispatch/sender stalled")
        .expect("wire closed")
}

/// Take the startup announcements off each peer's wire, in order: the
/// port's Network-Number-Is, configured (R3), then, with another port, its
/// I-Am-Router.
pub(super) async fn drain_announcements(peers: &mut [Peer]) {
    let ports = peers.len();
    for (port, peer) in peers.iter_mut().enumerate() {
        let mut number = network(port).to_be_bytes().to_vec();
        number.push(1);
        let mut expected = vec![(NetworkMessageType::NETWORK_NUMBER_IS, number)];
        if ports > 1 {
            let beyond = (0..ports)
                .filter(|&other| other != port)
                .flat_map(|other| network(other).to_be_bytes())
                .collect();
            expected.push((NetworkMessageType::I_AM_ROUTER_TO_NETWORK, beyond));
        }
        for (message_type, data) in expected {
            let SendRequest::Broadcast { npdu, .. } = wire(peer).await else {
                panic!("expected startup announcement")
            };
            let npdu = decode_npdu(npdu).unwrap();
            assert_eq!(npdu.message_type, Some(message_type));
            assert_eq!(npdu.payload[..], data[..]);
        }
    }
}

// An unknown-route reject behind local arrivals is a FIFO dispatch barrier.
// It also proves local overload did not stop the existing reject path.
// RB-06: unknowns also emit a bounded Who-Is solicitation on the *other*
// ports; a stale solicitation from another port's earlier unknown may sit on
// this peer's wire ahead of our reject, so discard Who-Is broadcasts here.
// The first unknown solicits, repeats within 5s coalesce (no new broadcast).
pub(super) async fn barrier(peer: &mut Peer) {
    peer.tx
        .send(incoming(Some(address(9000, &[9])), 0))
        .await
        .unwrap();
    let (npdu, mac, data_attributes) = loop {
        match wire(peer).await {
            SendRequest::Unicast {
                npdu,
                mac,
                data_attributes,
                ..
            } => break (npdu, mac, data_attributes),
            SendRequest::Broadcast { npdu, .. } => {
                let decoded = decode_npdu(npdu).unwrap();
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK),
                    "barrier expected reject, got unexpected broadcast"
                );
                assert_eq!(decoded.payload.as_ref(), 9000u16.to_be_bytes());
                continue;
            }
        }
    };
    let npdu = decode_npdu(npdu).unwrap();
    assert_eq!(mac.as_slice(), &[0xA0]);
    assert_eq!(
        npdu.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
    );
    assert_eq!(
        npdu.payload.as_ref(),
        [
            RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw(),
            0x23,
            0x28
        ]
    );
    // RB-03: locally-generated rejects are ingress-triggered, so the ingress
    // data attributes travel with the reject instead of being dropped.
    assert_eq!(data_attributes, incoming(None, 0).data_attributes);
}

pub(super) async fn branch_forward(peers: &mut [Peer], branch: LocalBranch, port: usize, id: u16) {
    let target = match branch {
        LocalBranch::GlobalBroadcast => 1 - port,
        LocalBranch::RemoteBroadcast => port,
        _ => return,
    };
    // RB-06: discard any stale discovery Who-Is (9000) ahead of the expected
    // forward; the first unknown solicits, repeats coalesce.
    let (npdu, data_attributes) = loop {
        match wire(&mut peers[target]).await {
            SendRequest::Broadcast {
                npdu,
                data_attributes,
                ..
            } => {
                let decoded = decode_npdu(npdu.clone()).unwrap();
                if decoded.message_type == Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
                    && decoded.payload.as_ref() == 9000u16.to_be_bytes()
                {
                    continue;
                }
                break (npdu, data_attributes);
            }
            SendRequest::Unicast { .. } => panic!("expected broadcast for {branch:?}"),
        }
    };
    assert_eq!(decode_npdu(npdu).unwrap().payload.as_ref(), apdu_of(id));
    assert_eq!(data_attributes, incoming(None, id).data_attributes);
}

pub(super) async fn forwarding_progress(peers: &mut [Peer], port: usize) {
    let target = 1 - port;
    peers[port]
        .tx
        .send(incoming(Some(address(network(target), &[9])), 1000))
        .await
        .unwrap();
    // RB-06: discard stale discovery Who-Is broadcasts ahead of the unicast.
    let (npdu, mac) = loop {
        match wire(&mut peers[target]).await {
            SendRequest::Unicast { npdu, mac, .. } => break (npdu, mac),
            SendRequest::Broadcast { npdu, .. } => {
                let decoded = decode_npdu(npdu).unwrap();
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
                );
                assert_eq!(decoded.payload.as_ref(), 9000u16.to_be_bytes());
                continue;
            }
        }
    };
    assert_eq!(mac.as_slice(), &[9]);
    assert_eq!(decode_npdu(npdu).unwrap().payload.as_ref(), apdu_of(1000));
    barrier(&mut peers[port]).await;
}

pub(super) fn assert_quiet(peers: &mut [Peer]) {
    // RB-06: a single coalesced discovery Who-Is (9000) may remain on a wire
    // with no later barrier to consume it; discard those, then require quiet.
    for peer in peers {
        while let Ok(req) = peer.wire.try_recv() {
            match req {
                SendRequest::Broadcast { npdu, .. } => {
                    let decoded = decode_npdu(npdu).unwrap();
                    assert_eq!(
                        decoded.message_type,
                        Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
                    );
                    assert_eq!(decoded.payload.as_ref(), 9000u16.to_be_bytes());
                }
                SendRequest::Unicast { .. } => panic!("expected quiet wire, got unicast"),
            }
        }
    }
}

pub(super) fn assert_apdu(apdu: &ReceivedApdu, branch: LocalBranch, port: usize, id: u16) {
    assert_eq!(apdu.apdu.as_ref(), apdu_of(id));
    assert_eq!(apdu.source_mac, incoming(None, id).source_mac);
    assert_eq!(apdu.ingress_network, Some(network(port)));
    assert_eq!(apdu.source_network, Some(address(300, &[0x55])));
    assert!(apdu.link_layer_group);
    assert_eq!(apdu.is_group, !matches!(branch, LocalBranch::DadrMatch));
    assert_eq!(apdu.data_attributes, incoming(None, id).data_attributes);
}

pub(super) async fn fill(peers: &mut [Peer]) -> oneshot::Receiver<Bytes> {
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

pub(super) async fn dropped_arrival(peers: &mut [Peer], branch: LocalBranch, port: usize) {
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
