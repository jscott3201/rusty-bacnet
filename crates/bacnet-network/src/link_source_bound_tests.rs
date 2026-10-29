//! Network-layer admission of a frame from a link-layer source MAC longer
//! than [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) (#1198).
//!
//! No built-in transport reports such a MAC, so [`Injector`] ports stand in
//! for a custom transport: the test hands them received frames from source
//! MACs of any length. The non-router [`NetworkLayer`] and the router drop and
//! count each refused frame before decoding it. Frames from an 18-octet MAC
//! follow the refused ones on the same port, and the first thing to come out
//! is from them, which shows the refused frames went nowhere: no APDU or
//! control was delivered, nothing was forwarded and no route was learned.
//!
//! [`MAX_MAC_LEN`]: bacnet_npdu::MAX_MAC_LEN

use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::layer::NetworkLayer;
use crate::loopback_fixture::{
    FromRouter, LONGEST, REMOTE, TOO_LONG, next_from_router, pump, recv, reject_of, wire,
};
use crate::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};

/// A port whose received frames the test supplies. What the stack sends
/// through it comes back out of `sent`, as a [`ReceivedNpdu`] whose
/// `link_layer_group` marks a broadcast, with each unicast's MAC, so the
/// loopback fixture's helpers read it.
struct Injector {
    received: Option<mpsc::Receiver<ReceivedNpdu>>,
    sent: mpsc::Sender<ReceivedNpdu>,
    unicast_macs: mpsc::UnboundedSender<MacAddr>,
    mac: [u8; 1],
}

/// The test's side of an [`Injector`].
struct Link {
    deliver: mpsc::Sender<ReceivedNpdu>,
    sent: FromRouter,
}

impl Link {
    async fn from(&self, source: &[u8], npdu: Vec<u8>) {
        let frame = ReceivedNpdu::unverified(
            Bytes::from(npdu),
            MacAddr::from_slice(source),
            false,
            Vec::new(),
            None,
        );
        self.deliver.send(frame).await.unwrap();
    }
}

fn injector(mac: u8) -> (Injector, Link) {
    let (deliver, received) = mpsc::channel(16);
    let (sent_tx, sent) = mpsc::channel(64);
    let (unicast_macs, macs) = mpsc::unbounded_channel();
    let port = Injector {
        received: Some(received),
        sent: sent_tx,
        unicast_macs,
        mac: [mac],
    };
    let sent = FromRouter::new(sent, macs);
    (port, Link { deliver, sent })
}

impl Injector {
    /// Queue `npdu` for the test: to `unicast` when given, or as a broadcast.
    fn record(&self, npdu: &[u8], unicast: Option<&[u8]>) {
        let frame = ReceivedNpdu::unverified(
            Bytes::copy_from_slice(npdu),
            MacAddr::from_slice(&self.mac),
            unicast.is_none(),
            Vec::new(),
            None,
        );
        if self.sent.try_send(frame).is_ok()
            && let Some(mac) = unicast
        {
            let _ = self.unicast_macs.send(MacAddr::from_slice(mac));
        }
    }
}

impl TransportPort for Injector {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.received
            .take()
            .ok_or_else(|| Error::Encoding("already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.record(npdu, Some(mac));
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.record(npdu, None);
        Ok(())
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
}

/// A link-layer source MAC of `length` octets.
fn mac(length: u8) -> MacAddr {
    (0..length).map(|i| 0xA0u8.wrapping_add(i)).collect()
}

const I_AM: Option<NetworkMessageType> = Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK);

#[tokio::test]
async fn non_router_drops_and_counts_a_frame_from_an_over_long_source_mac() {
    let (port, link) = injector(0x01);
    let mut network = NetworkLayer::new(port);
    let mut controls = network.enable_network_control_receiver().unwrap();
    let mut apdus = network.start().await.unwrap();

    for length in [TOO_LONG, 255, LONGEST] {
        link.from(&mac(length), wire(None, None, None)).await;
        link.from(&mac(length), wire(None, None, I_AM)).await;
    }

    let apdu = recv(&mut apdus).await;
    assert_eq!(apdu.source_mac, mac(LONGEST));
    // The controls behind the last APDU reach their receiver once the
    // APDU receiver is polled again (#1713).
    pump(&mut apdus);
    let control = recv(&mut controls).await;
    assert_eq!(control.source_mac, mac(LONGEST));
    assert_eq!(network.address_length_drops(), 4);
    assert!(apdus.try_recv().is_err() && controls.try_recv().is_err());

    network.stop().await.unwrap();
}

#[tokio::test]
async fn router_drops_and_counts_a_frame_from_an_over_long_source_mac() {
    let (port_a, mut link_a) = injector(0x01);
    let (port_b, mut link_b) = injector(0x02);
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
    let StartedRouter {
        mut router,
        apdus: mut local,
        ..
    } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();

    // From a 19-octet MAC on network 1000: a local APDU, a route to network
    // 3000 through that MAC, and an APDU to relay to network 2000.
    let long = mac(TOO_LONG);
    link_a.from(&long, wire(None, None, None)).await;
    link_a.from(&long, wire(None, None, I_AM)).await;
    link_a.from(&long, wire(Some((2000, 1)), None, None)).await;
    link_a.from(&mac(LONGEST), wire(None, None, None)).await;

    let delivered = recv(&mut local).await;
    assert_eq!(delivered.source_mac, mac(LONGEST));
    assert_eq!(delivered.ingress_network, Some(1000));
    assert_eq!(router.address_length_drops(), 3);

    // The route was not learned: network 3000 is still unknown, so the router
    // looks for it on network 1000 and rejects the APDU with reason 1.
    link_b
        .from(&[0x0B], wire(Some((REMOTE, 1)), None, None))
        .await;
    let (reject, _) = next_from_router(&mut link_b.sent).await;
    assert_eq!(
        reject_of(&reject).reason,
        RejectMessageReason::NOT_DIRECTLY_CONNECTED
    );
    let (solicit, to) = next_from_router(&mut link_a.sent).await;
    assert!(to.is_none(), "the solicitation is a broadcast");
    assert_eq!(
        solicit.message_type,
        Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
    );
    assert!(local.try_recv().is_err());
    assert_eq!(router.address_length_drops(), 3);

    router.stop().await.unwrap();
}
