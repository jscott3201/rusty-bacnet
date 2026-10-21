//! The B/IPv6 receive loop: what happens to one datagram once the socket has
//! admitted it to the selected link.

use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bacnet_encoding::npdu::decode_npdu;
use bacnet_types::MacAddr;
use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::npdu_channel::{self, Refused};
use crate::port::ReceivedNpdu;
use crate::udp_metadata::ReceivedDatagram;

use super::frame::encoded;
use super::ingress::{
    destination_vmac_matches, forwarded_address_resolution_is_trusted, forwarded_npdu_is_trusted,
    forwarded_source_is_usable, group_destination, is_answerable_source, is_configured_bbmd,
    original_destination_matches, own_frame, source_drop, LocalBinding,
};
use super::receive_counters::{DropReason, ReceiveDrops};
use super::reply_limit::ReplyLimiter;
use super::resolver::Resolver;
use super::socket::{Bip6Socket, Inbound};
use super::vmac_table::Heard;
use super::{Bip6Vmac, Bvlc6Function, Bvlc6ResultCode, Bvll6, Bvll6Error, Bvll6Frame};

/// Octets an NPDU starts with whatever else it holds: version and control
/// (Clause 6.2).
const MIN_NPDU_LENGTH: usize = 2;

/// A BVLL message the receive loop owes, and the address it goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Reply {
    /// Where the reply goes.
    pub(super) to: SocketAddr,
    /// The reply. No reply carries an NPDU, so none borrows the datagram
    /// it answers.
    pub(super) message: Bvll6<'static>,
}

/// What the receive loop needs besides the datagram itself.
pub(super) struct Receiver {
    /// The transport's socket, also used for the replies the loop sends.
    pub(super) socket: Arc<Bip6Socket>,
    /// Where NPDUs go up to the network layer.
    pub(super) tx: mpsc::Sender<ReceivedNpdu>,
    /// The socket's UDP port, which with `local_ip` makes this node's
    /// B/IPv6 address, to recognise its own frames.
    pub(super) local_port: u16,
    /// This node's virtual MAC.
    pub(super) vmac: Bip6Vmac,
    /// The address the socket answers unicast on.
    pub(super) local_ip: Ipv6Addr,
    /// Unicast addresses of the local interfaces.
    pub(super) unicast_ips: Vec<Ipv6Addr>,
    /// Whether the socket is bound to the wildcard address.
    pub(super) wildcard_bind: bool,
    /// The registration of a foreign device, whose BBMD can move while the
    /// loop runs; `None` for any other node.
    pub(super) foreign_bbmd: Option<Arc<super::foreign_target::ForeignTarget>>,
    /// The neighbour table, which learns from what arrives, and the
    /// resolutions an arrival can complete.
    pub(super) neighbours: Arc<Resolver>,
    /// Forwarded-NPDUs refused for a group origin (#1493), shared with the
    /// transport.
    pub(super) forwarded_group_origin_drops: Arc<AtomicU64>,
    /// Where the datagrams this loop drops, and the replies it holds back,
    /// are counted (#1643), shared with the transport.
    pub(super) drops: Arc<ReceiveDrops>,
    /// The quota the NAKs for BBMD requests count against.
    pub(super) nak_quota: ReplyLimiter,
    /// The quota the Address-Resolution-ACKs answering a
    /// Forwarded-Address-Resolution count against. It is separate from
    /// `nak_quota`, so a flood of BBMD requests, which anyone can send from
    /// many addresses, can't spend the budget these answers need.
    pub(super) far_ack_quota: ReplyLimiter,
}

/// Which of the receive loop's reply quotas an answer counts against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quota {
    /// [`Receiver::nak_quota`].
    Nak,
    /// [`Receiver::far_ack_quota`].
    FarAck,
}

/// Count and log a failed read, and say whether to read on. An
/// `InvalidData` error drops one datagram (too large for the buffer, without
/// usable metadata, or on Windows an error reported for an earlier send; see
/// udp_metadata.rs) and reading goes on. Any other error stops the loop.
pub(super) fn receive_failed(error: &io::Error, drops: &ReceiveDrops) -> bool {
    drops.count(DropReason::ReceiveError);
    if error.kind() == io::ErrorKind::InvalidData {
        debug!(%error, "Dropping UDP datagram with invalid destination metadata");
        return true;
    }
    warn!(%error, "IPv6 UDP recv error");
    false
}

impl Receiver {
    /// The BBMD this node is registered with as a foreign device, read at
    /// the moment of the check, so a moved registration applies at once.
    fn foreign_bbmd(&self) -> Option<(Ipv6Addr, u16)> {
        self.foreign_bbmd.as_ref().map(|target| target.bbmd())
    }

    /// Receive until the socket fails.
    pub(super) async fn run(self) {
        let mut recv_buf = vec![0u8; 2048];
        loop {
            let read = self.socket.recv_from(&mut recv_buf).await;
            if !self.on_read(read, &recv_buf).await {
                break;
            }
        }
    }

    /// Act on one read of the socket, whose datagram is at the start of
    /// `buf`: handle it and send the reply it calls for, or count what was
    /// dropped. Returns whether to read on.
    pub(super) async fn on_read(&self, read: io::Result<Inbound>, buf: &[u8]) -> bool {
        match read {
            Ok(Inbound::Datagram(received)) => {
                if let Some(reply) = self.handle_datagram(&buf[..received.len], &received).await {
                    self.send(reply).await;
                }
                true
            }
            Ok(Inbound::OffLink) => {
                self.drops.count(DropReason::LinkMismatch);
                debug!("Dropping a datagram outside the selected IPv6 link or address");
                true
            }
            Err(error) => receive_failed(&error, &self.drops),
        }
    }

    /// This node's B/IPv6 address.
    fn local(&self) -> SocketAddrV6 {
        SocketAddrV6::new(self.local_ip, self.local_port, 0, 0)
    }

    fn binding(&self) -> LocalBinding<'_> {
        LocalBinding {
            ip: self.local_ip,
            vmac: self.vmac,
            unicast_ips: &self.unicast_ips,
            wildcard_bind: self.wildcard_bind,
        }
    }

    /// Decode one datagram, drop it if its addressing or its sender doesn't
    /// fit its BVLC function, and otherwise act on it. Returns the reply it
    /// calls for, which the receive loop sends. Each datagram dropped is
    /// counted once, under the first check it fails (#1643).
    pub(super) async fn handle_datagram(
        &self,
        data: &[u8],
        received: &ReceivedDatagram,
    ) -> Option<Reply> {
        // An IPv6 socket reports only IPv6 peers: any other is unusable
        // metadata, counted as a failed read is.
        let SocketAddr::V6(peer) = received.peer else {
            self.drops.count(DropReason::ReceiveError);
            return None;
        };
        // Per datagram, so at debug: a flood of them must not flood the log
        // at warn. The counters carry the signal.
        let frame = match Bvll6Frame::decode(data) {
            Ok(frame) => frame,
            Err(error) => {
                self.drops.count(DropReason::of_decode_error(&error));
                debug!(%error, "Failed to decode BVLC6 frame");
                return None;
            }
        };
        let message = match frame.message() {
            Ok(message) => message,
            Err(error) => return self.malformed(frame.function, error, peer, received),
        };
        let function = message.function();
        if !destination_vmac_matches(&message, self.vmac) {
            self.drops.count(DropReason::DestinationVmacMismatch);
            debug!(
                function = function.to_raw(),
                destination_vmac = ?message.destination_vmac(),
                "Dropping BVLC6 message addressed to another VMAC"
            );
            return None;
        }
        if !original_destination_matches(
            function,
            received.destination,
            message.destination_vmac(),
            &self.binding(),
            received.os_group_delivery,
        ) {
            self.drops.count(DropReason::DestinationMismatch);
            debug!(
                function = function.to_raw(),
                destination = %received.destination,
                "Dropping BVLC6/IP or Destination-VMAC mismatch"
            );
            return None;
        }
        if function == Bvlc6Function::FORWARDED_NPDU
            && !forwarded_npdu_is_trusted(
                received.peer,
                received.destination,
                received.os_group_delivery,
                self.local_ip,
                &self.unicast_ips,
                self.wildcard_bind,
                self.foreign_bbmd(),
            )
        {
            self.drops.count(DropReason::UntrustedForwardedNpdu);
            debug!(
                peer = %received.peer,
                destination = %received.destination,
                "Dropping Forwarded-NPDU from an untrusted path"
            );
            return None;
        }
        // A Forwarded-Address-Resolution names the address its answer goes
        // to, so the path it came by decides whether it is answered.
        if function == Bvlc6Function::FORWARDED_ADDRESS_RESOLUTION
            && !forwarded_address_resolution_is_trusted(
                received.peer,
                received.destination,
                received.os_group_delivery,
                &self.binding(),
                self.foreign_bbmd(),
            )
        {
            self.drops.count(DropReason::UntrustedForwardedResolution);
            debug!(
                peer = %received.peer,
                destination = %received.destination,
                "Dropping Forwarded-Address-Resolution from an untrusted path"
            );
            return None;
        }
        // This node's own frame come back, another node sending from this
        // node's VMAC, which would show the network layer a frame from
        // itself, or an NPDU from a source no answer could reach.
        if let Some(reason) = source_drop(&message, peer, self.local(), self.vmac) {
            self.own(reason);
            return None;
        }
        // Forwarded-NPDU's source VMAC identifies the original node, not the
        // forwarding BBMD. Its original address is learned from the function
        // payload in `dispatch`. A source no node sends from is never
        // learned: nothing could be sent back to it.
        if matches!(
            function,
            Bvlc6Function::ORIGINAL_UNICAST_NPDU
                | Bvlc6Function::ORIGINAL_BROADCAST_NPDU
                | Bvlc6Function::ADDRESS_RESOLUTION
                | Bvlc6Function::ADDRESS_RESOLUTION_ACK
                | Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION
                | Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK
        ) && is_answerable_source(received.peer)
        {
            self.neighbours
                .learn(message.source_vmac(), peer, Heard::Direct);
        }
        self.dispatch(message, peer, received).await
    }

    /// Count a frame [`source_drop`] or [`own_frame`] refused for its
    /// source, and warn, throttled, of another node using this node's VMAC.
    fn own(&self, reason: DropReason) {
        self.drops.count(reason);
        if reason == DropReason::OwnVmacSource {
            self.drops.vmac_collision(self.vmac);
        }
    }

    /// A frame whose header decoded, but whose function Annex U doesn't
    /// define or whose payload doesn't fit its function. It is counted and
    /// dropped, except that a malformed Register-Foreign-Device,
    /// Delete-Foreign-Device-Table-Entry or Distribute-Broadcast-To-Network
    /// gets the NAK a well-formed one would, under the same guards and the
    /// same quota (#1638). Annex U doesn't ask for a reply to a malformed
    /// request: this is for parity with B/IP, which NAKs one so that its
    /// sender isn't left waiting. A malformed request the guards refuse a
    /// NAK is not counted again.
    fn malformed(
        &self,
        function: Bvlc6Function,
        error: Bvll6Error,
        peer: SocketAddrV6,
        received: &ReceivedDatagram,
    ) -> Option<Reply> {
        self.drops.count(DropReason::of_decode_error(&error));
        // The guards a well-formed request passes first: sent to this node's
        // own address, not to a group.
        let refusable = matches!(
            function,
            Bvlc6Function::REGISTER_FOREIGN_DEVICE
                | Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY
                | Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK
        ) && original_destination_matches(
            function,
            received.destination,
            None,
            &self.binding(),
            received.os_group_delivery,
        );
        if !refusable {
            debug!(
                function = function.to_raw(),
                %error,
                "Dropping unknown or malformed BVLC6 message"
            );
            return None;
        }
        let source = self.reply_address(peer)?;
        debug!(
            function = function.to_raw(),
            %error,
            peer = %source,
            "Refusing a malformed BBMD request: this node is not a BBMD"
        );
        self.nak(function, source)
    }

    /// Send a reply that `handle_datagram` returned.
    async fn send(&self, reply: Reply) {
        match encoded(&reply.message) {
            Ok(frame) => {
                if let Err(e) = self.socket.send_to(&frame, reply.to).await {
                    debug!(error = %e, to = %reply.to, "Failed to send BVLC6 reply");
                }
            }
            Err(e) => debug!(error = %e, "Failed to encode BVLC6 reply"),
        }
    }

    /// `message` for `to`, unless `quota` for `to` is spent, when the reply
    /// is counted as held back.
    fn limited(&self, quota: Quota, to: SocketAddrV6, message: Bvll6<'static>) -> Option<Reply> {
        let (limiter, held_back) = match quota {
            Quota::Nak => (&self.nak_quota, DropReason::NakSuppressed),
            Quota::FarAck => (&self.far_ack_quota, DropReason::FarAckSuppressed),
        };
        if !limiter.admit_now(*to.ip()) {
            self.drops.count(held_back);
            debug!(
                to = %to,
                function = message.function().to_raw(),
                "Dropping a BVLC6 reply over the reply quota"
            );
            return None;
        }
        Some(Reply {
            to: to.into(),
            message,
        })
    }

    /// The NAK for `function`, one of the requests only a BBMD serves, to
    /// `to`, unless the NAK quota for `to` is spent.
    fn nak(&self, function: Bvlc6Function, to: SocketAddrV6) -> Option<Reply> {
        let code = function.nak()?;
        let nak = Bvll6::BvlcResult {
            source: self.vmac,
            code,
        };
        self.limited(Quota::Nak, to, nak)
    }

    /// Hand an NPDU up to the network layer, or count it dropped when the
    /// channel is full or closed. `frame` names its kind for the log.
    fn hand_up(
        &self,
        npdu: impl FnOnce() -> Bytes,
        source_mac: &[u8],
        link_layer_group: bool,
        frame: &'static str,
    ) {
        let source_mac = MacAddr::from_slice(source_mac);
        match npdu_channel::hand_up(&self.tx, npdu, source_mac, link_layer_group) {
            Ok(()) => {}
            Err(Refused::Full) => {
                self.drops.npdu_channel_full(frame);
            }
            Err(Refused::Closed) => {
                self.drops.count(DropReason::NpduChannelClosed);
                debug!(
                    frame,
                    "B/IPv6: NPDU channel closed, dropping an incoming frame"
                );
            }
        }
    }

    /// Act on a message whose addressing and sender `handle_datagram` has
    /// checked, and return the reply it calls for.
    ///
    /// This transport is never a B/IPv6 BBMD. Annex U's receive-side rules
    /// for a node that isn't one, against what the transport does. Two terms
    /// keep the rows short: an *answerable source* is a datagram source
    /// [`is_answerable_source`] admits, and an address *usable without a
    /// scope ID* is one a message carries that [`forwarded_source_is_usable`]
    /// admits. Only an answerable source is ever learned from.
    ///
    /// | Message | Annex U asks | This transport | Clause |
    /// | --- | --- | --- | --- |
    /// | BVLC-Result | No reply: it reports on a request this node sent | Logged, a NAK at error level at most once per interval. The configured BBMD's Address-Resolution NAK marks this node's resolutions refused; a peer's Virtual-Address-Resolution NAK fails the lookup of it | U.2.1, U.4.4 |
    /// | Original-Unicast-NPDU | Take the NPDU when the destination VMAC is this node's, else discard | Handed up, its source VMAC as the source MAC, when sent to this node's address and VMAC | U.1.3, U.2.2, U.3 |
    /// | Original-Broadcast-NPDU | Take the NPDU | Handed up, its source VMAC as the source MAC, when sent to a BACnet multicast group | U.1.3, U.2.3, U.4 |
    /// | Address-Resolution | When it targets this node's VMAC, unicast an Address-Resolution-ACK to the asker | Answered at an answerable source other than this node's own address, when sent to a BACnet group and targeting this VMAC; no quota | U.2.4, U.5 |
    /// | Forwarded-Address-Resolution | When it targets this node's VMAC, unicast an Address-Resolution-ACK to the asker's address it carries | Taken by a foreign device only by unicast from its configured BBMD, and by any other node only from a BACnet group with an answerable source. Answered at the carried address when the target is this VMAC and that address is usable without a scope ID; the asker is then learned. FAR-answer quota | U.2.5, U.4.4, U.5 |
    /// | Address-Resolution-ACK | Update the VMAC table; discard one for another VMAC | Learned when sent to this node's address and VMAC, which completes a resolution of the sender's VMAC | U.2.6, U.3, U.5 |
    /// | Virtual-Address-Resolution | Unicast a Virtual-Address-Resolution-ACK to the asker | Answered at an answerable source other than this node's own address, when sent to this node's address; no quota | U.2.7, U.5 |
    /// | Virtual-Address-Resolution-ACK | Update the VMAC table; discard one for another VMAC | Learned when sent to this node's address and VMAC, which completes a lookup of the sender's address | U.2.8, U.3, U.5 |
    /// | Forwarded-NPDU | Take the NPDU as from its original source | Handed up, the origin's VMAC as the source MAC, from a BACnet group, or in foreign mode from the configured BBMD, when the original address is usable without a scope ID; that address is learned unless the origin was heard directly within the last minute | U.2.9, U.4.4 |
    /// | Register-Foreign-Device | Reply with its NAK | Its NAK at an answerable source other than this node's own address, when sent to this node's address. NAK quota | U.4.5.3 |
    /// | Delete-Foreign-Device-Table-Entry | Reply with its NAK | The same | U.4.5.4 |
    /// | Distribute-Broadcast-To-Network | Reply with its NAK | The same | U.4.4 |
    /// | Function 0x0B | Nothing: its clause was removed | Dropped as unknown before dispatch | U.2.12 |
    ///
    /// Before any of this, a frame from this node's own address, one that
    /// names this node's VMAC as its sender's from another address, and an
    /// Original-Unicast-NPDU or Original-Broadcast-NPDU from a source that
    /// isn't answerable are dropped and counted ([`source_drop`],
    /// [`own_frame`]), and this node's VMAC is never learned.
    ///
    /// A malformed Register-Foreign-Device, Delete-Foreign-Device-Table-Entry
    /// or Distribute-Broadcast-To-Network never reaches this function, but
    /// gets the same NAK under the same conditions (#1638). Annex U doesn't
    /// ask for that; it matches B/IP.
    ///
    /// The quotas are two [`ReplyLimiter`]s. A reply goes either to the
    /// datagram's source or, for a Forwarded-Address-Resolution that came by
    /// a trusted path ([`forwarded_address_resolution_is_trusted`]), to the
    /// asker's address that message carries; never anywhere else.
    pub(super) async fn dispatch(
        &self,
        message: Bvll6<'_>,
        peer: SocketAddrV6,
        received: &ReceivedDatagram,
    ) -> Option<Reply> {
        match message {
            Bvll6::OriginalUnicastNpdu { source, npdu, .. }
            | Bvll6::OriginalBroadcastNpdu { source, npdu } => {
                // `handle_datagram` dropped this node's own frames. A group
                // delivery if either the function or the address
                // says so: an Original-Unicast-NPDU sent to a multicast group
                // reached every node in it, so a confirmed request in it is
                // never answered as a directed one (Clause 5.4.5.1).
                let broadcast = matches!(message, Bvll6::OriginalBroadcastNpdu { .. });
                let link_layer_group = broadcast
                    || group_destination(received.destination, received.os_group_delivery);
                let frame = if broadcast { "broadcast" } else { "unicast" };
                let npdu = || Bytes::copy_from_slice(npdu);
                self.hand_up(npdu, &source.octets(), link_layer_group, frame);
                None
            }

            Bvll6::ForwardedNpdu {
                original_source,
                original_address,
                npdu,
            } => {
                // Too short to start an NPDU: dropped before anything else,
                // the origin included, is looked at.
                if npdu.len() < MIN_NPDU_LENGTH {
                    self.drops.count(DropReason::MalformedForwardedNpdu);
                    debug!(
                        length = npdu.len(),
                        "Dropping Forwarded-NPDU too short to hold an NPDU"
                    );
                    return None;
                }
                // The 18 octets carry no scope ID, so the origin gets none.
                let source_addr = SocketAddrV6::from(original_address);
                // This node's own broadcast, come back through a BBMD, as
                // where two BBMDs' multicast scopes overlap, or another
                // node's from this node's VMAC. Handing it up would show the
                // network layer a frame from itself (#1643).
                let own = own_frame(original_source, source_addr, self.local(), self.vmac);
                if let Some(reason) = own {
                    self.own(reason);
                    debug!("Dropping a Forwarded-NPDU whose origin is this node");
                    return None;
                }
                // The origin is learned as the NPDU source's address, so a
                // group there would bind a forged I-Am to every node in it,
                // or send a request's answer to them all. No node sends from
                // one: the frame is malformed, and counted (#1493).
                if original_address.ip.is_multicast() {
                    self.forwarded_group_origin_drops
                        .fetch_add(1, Ordering::Relaxed);
                    debug!(
                        source = %source_addr,
                        "Dropping Forwarded-NPDU whose origin is a group address"
                    );
                    return None;
                }
                if !forwarded_source_is_usable(source_addr) {
                    self.drops.count(DropReason::UnusableAddress);
                    debug!(
                        source = %source_addr,
                        "Dropping Forwarded-NPDU with unusable origin"
                    );
                    return None;
                }
                // Decoding needs the NPDU's own octets, so this copy comes
                // before the channel's slot is reserved.
                let npdu = Bytes::copy_from_slice(npdu);
                if decode_npdu(npdu.clone()).is_err() {
                    self.drops.count(DropReason::MalformedForwardedNpdu);
                    debug!("Dropping Forwarded-NPDU with malformed NPDU");
                    return None;
                }
                self.neighbours
                    .learn(original_source, source_addr, Heard::Carried);
                self.hand_up(|| npdu, &original_source.octets(), true, "forwarded");
                None
            }

            Bvll6::VirtualAddressResolution { source } => {
                // A node receiving VAR at its unicast B/IPv6 address
                // answers with its own VMAC and the requester's VMAC.
                Some(Reply {
                    to: self.answer_address(peer)?.into(),
                    message: Bvll6::VirtualAddressResolutionAck {
                        source: self.vmac,
                        destination: source,
                    },
                })
            }

            Bvll6::AddressResolution { source, target } => {
                // AR: sender wants to know our B/IPv6 address from our VMAC.
                // Outside the reply quotas: a flood must not stop this node
                // answering another node's VMAC collision probe, which is
                // an AR for the prober's own VMAC.
                if target != self.vmac {
                    return None;
                }
                debug!(vmac = ?self.vmac, "Received AR for our VMAC, sending AR-Ack");
                Some(Reply {
                    to: self.answer_address(peer)?.into(),
                    message: Bvll6::AddressResolutionAck {
                        source: self.vmac,
                        destination: source,
                    },
                })
            }

            Bvll6::ForwardedAddressResolution {
                original_source,
                target,
                original_address,
            } => {
                if target != self.vmac {
                    return None;
                }
                // The answer goes to the asker, at the address the message
                // carries. That address has no scope ID, so it must be one
                // usable without one, as a Forwarded-NPDU's origin must.
                let asker = SocketAddrV6::from(original_address);
                if !forwarded_source_is_usable(asker) {
                    self.drops.count(DropReason::UnusableAddress);
                    debug!(
                        asker = %asker,
                        "Dropping Forwarded-Address-Resolution with an unusable original address"
                    );
                    return None;
                }
                // The VMAC table follows incoming messages (U.5). The asker
                // is learned under the same guards as the answer (a trusted
                // path, this node's VMAC as the target, a usable address),
                // even when the quota then holds the answer back.
                self.neighbours
                    .learn(original_source, asker, Heard::Carried);
                debug!(
                    vmac = ?self.vmac,
                    asker = %asker,
                    "Received FAR for our VMAC, sending AR-Ack"
                );
                let ack = Bvll6::AddressResolutionAck {
                    source: self.vmac,
                    destination: original_source,
                };
                self.limited(Quota::FarAck, asker, ack)
            }

            Bvll6::AddressResolutionAck { source, .. } => {
                // `handle_datagram` learned the sender's address.
                debug!(
                    vmac = ?source,
                    addr = %received.peer,
                    "Received AR-Ack"
                );
                None
            }

            Bvll6::VirtualAddressResolutionAck { source, .. } => {
                // `handle_datagram` learned the sender; this completes the
                // lookup of its address, if one is running.
                self.neighbours.virtual_answered(peer, source);
                None
            }

            Bvll6::BvlcResult { code, .. } => {
                if code == Bvlc6ResultCode::SUCCESSFUL_COMPLETION {
                    debug!("BIP6: BVLC-Result successful");
                    return None;
                }
                self.drops.result_nak(code);
                // The configured BBMD won't pass this node's
                // Address-Resolutions on (U.4.4), as when it holds no
                // registration for it; a peer refused its VAR.
                if code == Bvlc6ResultCode::ADDRESS_RESOLUTION_NAK
                    && is_configured_bbmd(peer.into(), self.foreign_bbmd())
                {
                    self.neighbours.refuse_resolutions();
                } else if code == Bvlc6ResultCode::VIRTUAL_ADDRESS_RESOLUTION_NAK {
                    self.neighbours.virtual_refused(peer);
                }
                None
            }

            // Requests only a BBMD carries out. This node isn't one, so each
            // gets its own NAK (U.4.4, U.4.5.3, U.4.5.4).
            Bvll6::RegisterForeignDevice { .. }
            | Bvll6::DeleteForeignDeviceTableEntry { .. }
            | Bvll6::DistributeBroadcastToNetwork { .. } => {
                let source = self.answer_address(peer)?;
                debug!(
                    function = message.function().to_raw(),
                    peer = %source,
                    "Refusing a BBMD request: this node is not a BBMD"
                );
                self.nak(message.function(), source)
            }
        }
    }

    /// The address to answer a request at: the datagram's source, unless it
    /// isn't an answerable source ([`is_answerable_source`]) or it is this
    /// node's own address. This node never sends itself a request, so one
    /// from its own address is forged, and answering it would only send
    /// this node its own answer (a Virtual-Address-Resolution-ACK from its
    /// own VMAC would even look like a collision).
    fn reply_address(&self, peer: SocketAddrV6) -> Option<SocketAddrV6> {
        if !is_answerable_source(peer.into()) {
            debug!(peer = %peer, "Not answering a request from an address no node sends from");
            return None;
        }
        if *peer.ip() == self.local_ip && peer.port() == self.local_port {
            debug!("Not answering a request from this node's own address");
            return None;
        }
        Some(peer)
    }

    /// [`Self::reply_address`] for a request that decoded, counting the
    /// request dropped when there is none (#1643).
    fn answer_address(&self, peer: SocketAddrV6) -> Option<SocketAddrV6> {
        let address = self.reply_address(peer);
        if address.is_none() {
            self.drops.count(DropReason::UnanswerableRequest);
        }
        address
    }
}
