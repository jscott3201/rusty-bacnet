//! Helpers the B/IP tests share: a receive context on a node, and one
//! message run through the receive path.

use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex};

use bacnet_bip_codec::BvlcFunction;
use bacnet_bip_core::node::{
    Addressed, Datagram, Discard, Link, LocalAddresses, Mode, Permissive, Received,
};
use bacnet_bip_core::{Instant as CoreInstant, LocalSubnet};
use tokio::sync::mpsc;

use super::client_snapshot::ClientRecord;
use super::io::{self, RecvContext};
use super::node::NodeHandle;
use super::rate_limit::ManagementRateLimiter;
use super::BipSocket;
use crate::bbmd::BbmdState;
use crate::bvll::{raw_frame, BvllMessage};
use crate::port::ReceivedNpdu;

/// Where a test's message was sent: to the node's own address, or to the
/// subnet's broadcast address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Delivery {
    /// To the broadcast address.
    Broadcast,
    /// To the node's own address.
    Unicast,
}

/// Run `msg` through the receive path as a datagram from `sender`, sent as
/// `delivery` says, on the primary socket.
pub(super) async fn handle_bvll_message(
    msg: &BvllMessage,
    sender: ([u8; 4], u16),
    delivery: Delivery,
    ctx: &RecvContext,
) {
    let wire = raw_frame(msg.function, &msg.payload);
    let to = Addressed {
        node: delivery == Delivery::Unicast,
        broadcast: delivery == Delivery::Broadcast,
    };
    let source = SocketAddrV4::new(Ipv4Addr::from(sender.0), sender.1);
    let datagram = Datagram::new(&wire, source, Some(to));
    io::receive(&datagram, ctx).await;
}

/// Whether a node at `local_ip` takes a `function` frame sent to
/// `destination`: `Some` unless its link drops the frame for not fitting
/// the address it was sent to (Annex J.2.11, J.2.12, J.3, J.4.1). A wildcard
/// bind (`wildcard_bind`) takes unicast to `local_unicast_ips` only (#952).
pub(super) fn admitted_delivery(
    function: BvlcFunction,
    destination: IpAddr,
    local_ip: Ipv4Addr,
    configured_broadcast: Ipv4Addr,
    local_unicast_ips: &[Ipv4Addr],
    wildcard_bind: bool,
    os_group_delivery: Option<bool>,
) -> Option<()> {
    let IpAddr::V4(destination) = destination else {
        return None;
    };
    let mut local = LocalAddresses::new(local_ip, configured_broadcast);
    local.wildcard = wildcard_bind.then_some(local_unicast_ips);
    let link = test_link(
        SocketAddrV4::new(local_ip, 0xBAC0),
        SocketAddrV4::new(configured_broadcast, 0xBAC0),
        Mode::Normal,
    );
    let frame = raw_frame(function, &[]);
    let datagram = Datagram::new(
        &frame,
        SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 7), 0xBAC0),
        Some(Addressed::classify(destination, os_group_delivery, &local)),
    );
    let received = link.receive(&datagram, CoreInstant::ZERO, &mut Permissive);
    (received != Received::Discarded(Discard::DestinationMismatch)).then_some(())
}

/// A link at `own`, whose local broadcasts go to `broadcast`, in `mode`,
/// sending NPDUs up to 1497 octets, as the transport runs it.
pub(super) fn test_link(own: SocketAddrV4, broadcast: SocketAddrV4, mode: Mode) -> Link {
    let mut link = Link::new(own, LocalSubnet::new(broadcast), mode).expect("a valid test link");
    link.budget = Some(super::DATAGRAM_BUDGET);
    link
}

impl RecvContext {
    /// A receive context on a node at `own` whose local broadcasts go to
    /// `broadcast`: a BBMD with `bbmd` as its state when one is given, and a
    /// normal node otherwise. See [`RecvContext::on_link`].
    pub(super) fn for_test(
        socket: Arc<BipSocket>,
        npdu_tx: mpsc::Sender<ReceivedNpdu>,
        own: SocketAddrV4,
        broadcast: SocketAddrV4,
        bbmd: Option<Arc<Mutex<BbmdState>>>,
    ) -> Self {
        let mode = if bbmd.is_some() {
            Mode::Bbmd
        } else {
            Mode::Normal
        };
        Self::on_link(socket, npdu_tx, test_link(own, broadcast, mode), bbmd)
    }

    /// A receive context on a node with `link`, sending from `socket` and
    /// handing NPDUs to `npdu_tx`; with `bbmd` as its BBMD state, which a
    /// [`Mode::Bbmd`] link needs. No fan-out worker, so a BBMD forwards
    /// directly, and no driver task: spawn [`NodeHandle::drive`] for
    /// timeouts and registration.
    pub(super) fn on_link(
        socket: Arc<BipSocket>,
        npdu_tx: mpsc::Sender<ReceivedNpdu>,
        link: Link,
        bbmd: Option<Arc<Mutex<BbmdState>>>,
    ) -> Self {
        let record = Arc::new(ClientRecord::default());
        let node =
            NodeHandle::new(link, socket, record, Arc::default()).expect("a valid test link");
        RecvContext {
            node: Arc::new(node),
            npdu_tx,
            bbmd,
            management_limiter: Arc::new(Mutex::new(ManagementRateLimiter::new())),
            fanout: None,
            group_drops: Default::default(),
            drops: Arc::default(),
            force_dbtn_forward_failure: false,
        }
    }
}

impl super::BipTransport {
    /// Wire a transport that wasn't started around `socket`, as `start()`
    /// would: its node at the socket's address, a BBMD when it has BBMD
    /// state, broadcasting to the configured address at its port.
    pub(super) fn wire_for_test(&mut self, socket: Arc<BipSocket>) {
        let std::net::SocketAddr::V4(own) = socket.local_addr().unwrap() else {
            panic!("an IPv4 test socket");
        };
        let mode = if self.bbmd.is_some() {
            Mode::Bbmd
        } else {
            Mode::Normal
        };
        let broadcast = SocketAddrV4::new(self.broadcast_address, self.port);
        let link = test_link(own, broadcast, mode);
        let record = Arc::clone(&self.client_record);
        let refusals = Arc::clone(&self.send_refusals);
        let node = NodeHandle::new(link, Arc::clone(&socket), record, refusals).unwrap();
        self.node = Some(Arc::new(node));
        self.socket = Some(socket);
    }
}
