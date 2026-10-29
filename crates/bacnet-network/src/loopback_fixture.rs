//! Wire-level router fixtures shared by the loopback test modules
//! (`address_bound_tests`, `link_source_bound_tests`, `reject_route_tests`).
//!
//! Raw NPDU bytes go in through [`LoopbackTransport`] peers, and the frames
//! the router sends come back out of them, so a test sees exactly what a
//! neighbouring node would. [`FromRouter`] also says which MAC each unicast
//! was sent to (#1243), and [`RouterFixture`]'s links carry data attributes
//! both ways (#1289).

use bacnet_encoding::npdu::{Npdu, decode_npdu};
use bacnet_npdu::{NetworkMessageType, RejectMessageToNetwork};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

use crate::layer::{ReceivedApdu, ReceivedNetworkControl};
use crate::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};

pub(crate) const APDU: [u8; 2] = [0x10, 0x08];
pub(crate) const TOO_LONG: u8 = bacnet_npdu::MAX_MAC_LEN as u8 + 1;
pub(crate) const LONGEST: u8 = bacnet_npdu::MAX_MAC_LEN as u8;
/// The original source network in routed frames.
pub(crate) const ORIGIN: u16 = 4000;
/// The network the fixture router reaches through peer B.
pub(crate) const REMOTE: u16 = 3000;

/// Hand-built NPDU bytes, since the encoder refuses the lengths under test.
/// `destination` is (DNET, DLEN) and `source` is (SNET, SLEN); each address
/// holds as many octets as its length announces. `control` makes it a network
/// message of that type carrying network 3000, in place of the APDU.
pub(crate) fn wire(
    destination: Option<(u16, u8)>,
    source: Option<(u16, u8)>,
    control: Option<NetworkMessageType>,
) -> Vec<u8> {
    let mut flags = 0;
    if control.is_some() {
        flags |= 0x80;
    }
    if destination.is_some() {
        flags |= 0x20;
    }
    if source.is_some() {
        flags |= 0x08;
    }
    let mut out = vec![0x01, flags];
    for ((network, length), first) in [(destination, 0xD0u8), (source, 0x50)]
        .into_iter()
        .filter_map(|(address, first)| address.map(|address| (address, first)))
    {
        out.extend_from_slice(&network.to_be_bytes());
        out.push(length);
        out.extend((0..length).map(|i| first.wrapping_add(i)));
    }
    if destination.is_some() {
        out.push(255);
    }
    match control {
        Some(message_type) => out.extend_from_slice(&[message_type.to_raw(), 0x0B, 0xB8]),
        None => out.extend_from_slice(&APDU),
    }
    out
}

pub(crate) async fn recv<R: Receive>(rx: &mut R) -> R::Item {
    timeout(Duration::from_secs(2), rx.receive())
        .await
        .expect("timed out waiting for the network layer")
        .expect("channel closed")
}

/// Poll `apdus` once more, finding no APDU, so the network messages the
/// transport queued after the last APDU are decoded and reach the control
/// receiver: the layer decodes only while its APDU receiver is polled (#1713).
pub(crate) fn pump(apdus: &mut crate::layer::ApduReceiver) {
    assert!(matches!(
        apdus.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

/// A receiver [`recv`] can wait on: a channel, or the layer's APDU receiver.
pub(crate) trait Receive {
    type Item;
    fn receive(&mut self) -> impl std::future::Future<Output = Option<Self::Item>>;
}

impl<T> Receive for mpsc::Receiver<T> {
    type Item = T;
    fn receive(&mut self) -> impl std::future::Future<Output = Option<T>> {
        self.recv()
    }
}

impl Receive for crate::layer::ApduReceiver {
    type Item = crate::layer::ReceivedApdu;
    fn receive(&mut self) -> impl std::future::Future<Output = Option<Self::Item>> {
        self.recv()
    }
}

/// What a router sends one peer: the frames, and the MAC each unicast among
/// them was sent to.
pub(crate) struct FromRouter {
    frames: mpsc::Receiver<ReceivedNpdu>,
    /// The destination of each unicast in `frames`, in the same order.
    unicast_macs: mpsc::UnboundedReceiver<MacAddr>,
}

impl FromRouter {
    /// Frames from a custom port, which reports each unicast's MAC to
    /// `unicast_macs` in the order it queues the frames.
    pub(crate) fn new(
        frames: mpsc::Receiver<ReceivedNpdu>,
        unicast_macs: mpsc::UnboundedReceiver<MacAddr>,
    ) -> Self {
        Self {
            frames,
            unicast_macs,
        }
    }

    /// Start `peer`, and record where `port`, the router's side of the
    /// pair, sends each unicast.
    pub(crate) async fn start(port: &mut LoopbackTransport, peer: &mut LoopbackTransport) -> Self {
        let unicast_macs = port.record_unicast_destinations();
        Self::new(peer.start().await.unwrap(), unicast_macs)
    }

    /// The next frame, with the MAC it was unicast to, or `None` for a
    /// data-link broadcast.
    pub(crate) async fn recv(&mut self) -> (ReceivedNpdu, Option<MacAddr>) {
        let frame = recv(&mut self.frames).await;
        let to = (!frame.link_layer_group).then(|| {
            self.unicast_macs
                .try_recv()
                .expect("each unicast has its MAC recorded")
        });
        (frame, to)
    }
}

/// A two-port router with a loopback peer on each port.
pub(crate) struct RouterFixture {
    pub(crate) router: BACnetRouter,
    pub(crate) local: mpsc::Receiver<ReceivedApdu>,
    /// Peer on port 0 (network 1000), MAC 0x0A.
    pub(crate) peer_a: LoopbackTransport,
    pub(crate) from_router_a: FromRouter,
    /// Peer on port 1 (network 2000), MAC 0x0B, the next hop to network 3000.
    pub(crate) peer_b: LoopbackTransport,
    pub(crate) from_router_b: FromRouter,
}

impl RouterFixture {
    pub(crate) async fn start() -> Self {
        Self::launch(false, None).await.0
    }

    /// [`Self::start`], with the router's network-control receiver (#1175).
    pub(crate) async fn start_with_network_control()
    -> (Self, mpsc::Receiver<ReceivedNetworkControl>) {
        let (fixture, controls) = Self::launch(true, None).await;
        (fixture, controls.expect("opted in"))
    }

    /// [`Self::start`], with `mac` a group destination of the router's port
    /// B, as its broadcast MAC (#1504).
    pub(crate) async fn start_with_group_on_b(mac: u8) -> Self {
        Self::launch(false, Some(mac)).await.0
    }

    async fn launch(
        network_control: bool,
        group_on_b: Option<u8>,
    ) -> (Self, Option<mpsc::Receiver<ReceivedNetworkControl>>) {
        let (mut port_a, mut peer_a) = LoopbackTransport::pair(vec![0x01], vec![0x0A]);
        let (mut port_b, mut peer_b) = LoopbackTransport::pair(vec![0x02], vec![0x0B]);
        if let Some(mac) = group_on_b {
            port_b.set_broadcast_mac(vec![mac]);
        }
        // Frames sent without data attributes arrive without them, so only a
        // test that sends some sees any.
        for transport in [&mut port_a, &mut peer_a, &mut port_b, &mut peer_b] {
            transport.carry_data_attributes();
        }
        let from_router_a = FromRouter::start(&mut port_a, &mut peer_a).await;
        let from_router_b = FromRouter::start(&mut port_b, &mut peer_b).await;
        let ports = vec![
            RouterPort {
                transport: port_a,
                network_number: 1000,
            },
            RouterPort {
                transport: port_b,
                network_number: 2000,
            },
        ];
        let mut options = RouterOptions::new();
        if network_control {
            options = options.network_control_receiver();
        }
        let StartedRouter {
            router,
            apdus: local,
            network_control: controls,
        } = BACnetRouter::start(ports, options).await.unwrap();
        let mut fixture = Self {
            router,
            local,
            peer_a,
            from_router_a,
            peer_b,
            from_router_b,
        };
        // Teach the router that network 3000 lies behind peer B. Its relay of
        // the announcement to port A shows the route is in place.
        let i_am = Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK);
        fixture
            .peer_b
            .send_broadcast(&wire(None, None, i_am))
            .await
            .unwrap();
        loop {
            let (frame, _) = fixture.from_router_a.recv().await;
            let npdu = decode_npdu(frame.npdu).unwrap();
            if npdu.message_type == i_am && npdu.payload[..] == REMOTE.to_be_bytes() {
                break;
            }
        }
        (fixture, controls)
    }

    pub(crate) async fn send_from_a(&self, bytes: &[u8]) {
        self.peer_a.send_unicast(bytes, &[0x01]).await.unwrap();
    }

    /// [`Self::send_from_a`], with `data_attributes` on the frame.
    pub(crate) async fn send_from_a_with(&self, bytes: &[u8], data_attributes: &[DataAttribute]) {
        self.peer_a
            .send_unicast_with_data_attributes(bytes, &[0x01], data_attributes)
            .await
            .unwrap();
    }

    pub(crate) async fn send_from_b(&self, bytes: &[u8]) {
        self.peer_b.send_unicast(bytes, &[0x02]).await.unwrap();
    }

    pub(crate) async fn stop(mut self) {
        self.router.stop().await.unwrap();
        self.peer_a.stop().await.unwrap();
        self.peer_b.stop().await.unwrap();
    }
}

/// The next frame a router sends to a peer, past its startup announcements
/// (Network-Number-Is and I-Am-Router-To-Network), with the MAC it was
/// unicast to, or `None` when it went out as a data-link broadcast.
pub(crate) async fn next_received_from_router(
    rx: &mut FromRouter,
) -> (ReceivedNpdu, Option<MacAddr>) {
    let announcements = [
        NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        NetworkMessageType::NETWORK_NUMBER_IS,
    ]
    .map(Some);
    loop {
        let (frame, to) = rx.recv().await;
        let message_type = decode_npdu(frame.npdu.clone()).unwrap().message_type;
        if !announcements.contains(&message_type) {
            return (frame, to);
        }
    }
}

/// [`next_received_from_router`], as raw NPDU bytes.
pub(crate) async fn next_frame_from_router(rx: &mut FromRouter) -> (Bytes, Option<MacAddr>) {
    let (frame, to) = next_received_from_router(rx).await;
    (frame.npdu, to)
}

/// [`next_frame_from_router`], decoded.
pub(crate) async fn next_from_router(rx: &mut FromRouter) -> (Npdu, Option<MacAddr>) {
    let (frame, to) = next_frame_from_router(rx).await;
    (decode_npdu(frame).unwrap(), to)
}

pub(crate) fn reject_of(npdu: &Npdu) -> RejectMessageToNetwork {
    assert_eq!(
        npdu.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
    );
    RejectMessageToNetwork::decode(&npdu.payload).unwrap()
}
