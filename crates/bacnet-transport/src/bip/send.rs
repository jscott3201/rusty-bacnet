//! What the transport sends for an NPDU: the node's link decides the
//! message and where it goes; only a foreign device's broadcast asks the
//! node itself.
//!
//! A send decides and makes its first, non-blocking try under the one read
//! lock every send takes, which also holds the socket (#2617): nothing is
//! copied or counted for it, and a stop, which takes the socket under the
//! write lock, waits for the try and then fails every later one. A socket
//! that would block leaves the datagram waiting, holding no socket, for
//! [`NodeHandle::relay_writable`] to say it can go or for the stop.

use std::future::Future;
use std::io;
use std::net::SocketAddrV4;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bacnet_bip_codec as codec;
use bacnet_bip_core::node::{Link, Mode, SendRefusal};
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::BipSocket;
use super::datagram::OutgoingDatagram;
use super::node::NodeHandle;
use super::own_broadcast::OwnBroadcastForwarder;
use crate::bvll::{bvll_error, encode_bvll};

/// What the first try at a send came to.
enum Tried<'a> {
    /// The socket took the datagram, or refused it.
    Done(io::Result<usize>),
    /// The socket would have blocked: the datagram waits for it.
    Blocked(OutgoingDatagram<'a>, SocketAddrV4),
}

impl NodeHandle {
    /// Send `npdu` to the B/IP address `mac` as an Original-Unicast-NPDU
    /// (Annex J.3). One of the link's group addresses is refused, unsent
    /// (#1677). The datagram is the BVLL header and `npdu`, sent as one
    /// without copying `npdu` into a buffer (#1978 item 5).
    pub(super) fn send_unicast<'a>(
        &'a self,
        npdu: &'a [u8],
        mac: &'a [u8],
    ) -> impl Future<Output = Result<(), Error>> + Send + 'a {
        self.send_now(move |link| {
            let dest = codec::address_from_slice(mac).map_err(bvll_error)?;
            let outgoing = link
                .unicast(dest, npdu)
                .map_err(|refusal| self.refuse_send(refusal, npdu.len()))?;
            // An Original-Unicast-NPDU is a four-octet header and `npdu`.
            let datagram = OutgoingDatagram::split(&outgoing.message, npdu).ok_or_else(|| {
                Error::encode("a unicast BVLL message is not a header and its NPDU")
            })?;
            Ok((datagram, outgoing.to))
        })
    }

    /// Broadcast `npdu` on this B/IP network: a foreign device sends it to
    /// its BBMD as Distribute-Broadcast-To-Network (Annex J.2.10); otherwise
    /// it goes out as an Original-Broadcast-NPDU to the subnet's broadcast
    /// address (Annex J.4.1), and a BBMD first hands it to `forwarder` for
    /// its BDT peers and foreign devices (Annex J.4.5). On a loopback
    /// interface with a wildcard socket, a local broadcast to
    /// 255.255.255.255 fails with [`LoopbackBroadcastRefused`](super::LoopbackBroadcastRefused),
    /// after a BBMD's fanout is queued.
    pub(super) async fn send_broadcast(
        &self,
        npdu: &[u8],
        forwarder: Option<&OwnBroadcastForwarder>,
    ) -> Result<(), Error> {
        // Decided on a copy of the link, as a foreign device's admission
        // takes the node lock, which no send takes under the link's.
        let link = self.link();
        // The link refuses what it can't carry. A foreign device whose BBMD
        // isn't known yet has nowhere to send (#2308): its node holds the
        // broadcast, or refuses it, below.
        let mut decided = match link.broadcast(npdu) {
            Ok(decided) => Some(decided),
            Err(SendRefusal::NoBbmd) => None,
            Err(refusal) => return Err(self.refuse_send(refusal, npdu.len())),
        };

        // A foreign device asks its node first, which holds the broadcast
        // while the first registration attempt after a start or a move to
        // another BBMD is outstanding, or while it waits for its BBMD,
        // refuses it when that hold is full (#1622), and queues it behind
        // released broadcasts its driver hasn't sent yet. A broadcast it
        // sends goes on the link it decided under: a move since the copy
        // above sends it to the new BBMD. A stopped node refuses it.
        if matches!(link.mode, Mode::Foreign(_)) {
            #[cfg(test)]
            {
                // Taken first, so no guard is held while the hook runs.
                let hook = self
                    .before_admit
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            match self.admit_broadcast(npdu)? {
                None => return Ok(()),
                Some(admitted) if admitted != link || decided.is_none() => {
                    // A move is foreign to foreign, so the mode, and with it
                    // `forward_own` (false for a foreign device), stays.
                    let again = admitted
                        .broadcast(npdu)
                        .map_err(|refusal| self.refuse_send(refusal, npdu.len()))?;
                    decided = Some(again);
                }
                Some(_) => {}
            }
        }
        // Only a waiting foreign device has none, and its node never sends.
        let (outgoing, forward_own) =
            decided.ok_or_else(|| self.refuse_send(SendRefusal::NoBbmd, npdu.len()))?;
        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, &outgoing.message)?;

        // The fanout is queued first, so its targets are fixed before the
        // local frame is on the wire. Whatever happens to it (see
        // OwnBroadcastForwarder::forward) never fails this send, and a
        // failed local send does not withdraw it. A stopped BBMD forwards
        // nothing.
        if forward_own && let Some(forwarder) = forwarder {
            self.with_sending(|_, _| Ok(()))?;
            forwarder.forward(npdu);
        }

        // On a loopback interface with a wildcard socket, the local leg to
        // 255.255.255.255 is refused, counted, after the fanout above: a
        // BBMD's unicast forwards are a separate leg (loopback_guard.rs). A
        // foreign device's broadcast goes to its BBMD's own address, and a
        // B/IP-M node's to its group, so neither is refused.
        let to = outgoing.to;
        if self.refuses_local_broadcast(to) {
            return self.with_sending(|_, _| Err(self.refuse_local_broadcast()));
        }
        self.send_now(|_| Ok((OutgoingDatagram::whole(&buf), to)))
            .await
    }

    /// Send the datagram `decide` makes of the link (see [`SendNow`]).
    pub(super) fn send_now<'a, D>(&'a self, decide: D) -> SendNow<'a, D>
    where
        D: Fn(&Link) -> Result<(OutgoingDatagram<'a>, SocketAddrV4), Error> + Send + Unpin + 'a,
    {
        SendNow {
            node: self,
            decide,
            blocked: None,
        }
    }

    /// Wait for the socket to take `datagram`, holding no socket: each try
    /// runs under the link's read lock, and between tries the send waits for
    /// the relay or the node's stop to wake it. Once the node has stopped it
    /// fails with the stopped error, and nothing goes out.
    async fn send_when_writable(
        &self,
        datagram: OutgoingDatagram<'_>,
        to: SocketAddrV4,
    ) -> Result<(), Error> {
        loop {
            let woken = self.writable.notified();
            tokio::pin!(woken);
            // Before the try, so a stop or the relay after it wakes this.
            woken.as_mut().enable();
            let sent = self.with_sending(|_, socket| Ok(socket.try_send(&datagram, to)))?;
            match sent {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                sent => return sent.map(drop).map_err(Error::Transport),
            }
            self.wants_writable.notify_one();
            woken.await;
        }
    }

    /// Wake the waiting sends each time the socket can take a datagram, for
    /// as long as the node runs: the transport's task for it, which the stop
    /// joins, owns `socket`. It runs only after a send would have blocked.
    pub(super) async fn relay_writable(self: Arc<Self>, socket: Arc<BipSocket>) {
        loop {
            self.wants_writable.notified().await;
            // The poll form, through the socket's one write waker, which
            // nothing else on this socket uses. `writable()` would add a
            // caller of tokio's async readiness, which the receive path
            // awaits for every datagram: with that caller, the optimizer
            // stopped inlining it there, about 215 instructions a request
            // in the counting harness's B/IP round trips (#2617).
            let _ = std::future::poll_fn(|cx| socket.poll_send_ready(cx)).await;
            self.writable.notify_waiters();
        }
    }
}

/// A send of the datagram `decide` makes of the link. Its first poll, once
/// the task's cooperative budget allows it, as any socket send spends it,
/// decides, reports the datagram to the tap and tries it once, all under
/// the link's read lock; only a socket that would block makes it wait,
/// boxed, so no send future above carries the wait. A future, not an
/// `async fn`, so it holds no more than the node, the decision and that
/// box: every response future above carries its size (#1713).
pub(super) struct SendNow<'a, D> {
    node: &'a NodeHandle,
    decide: D,
    blocked: Option<BlockedSend<'a>>,
}

type BlockedSend<'a> = Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;

impl<'a, D> Future for SendNow<'a, D>
where
    D: Fn(&Link) -> Result<(OutgoingDatagram<'a>, SocketAddrV4), Error> + Unpin,
{
    type Output = Result<(), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(blocked) = &mut this.blocked {
            return blocked.as_mut().poll(cx);
        }
        let coop = std::task::ready!(tokio::task::coop::poll_proceed(cx));
        let decide = &this.decide;
        let tried = this.node.with_sending(|link, socket| {
            let (datagram, to) = decide(link)?;
            Ok(first_try(socket, datagram, to))
        });
        match tried {
            Ok(Tried::Done(sent)) => {
                coop.made_progress();
                Poll::Ready(sent.map(drop).map_err(Error::Transport))
            }
            Err(error) => {
                coop.made_progress();
                Poll::Ready(Err(error))
            }
            // The wait keeps no budget: it yields anyway.
            Ok(Tried::Blocked(datagram, to)) => {
                let mut blocked: BlockedSend<'a> =
                    Box::pin(this.node.send_when_writable(datagram, to));
                let polled = blocked.as_mut().poll(cx);
                this.blocked = Some(blocked);
                polled
            }
        }
    }
}

/// Report `datagram` to the tap and try it once.
fn first_try<'a>(
    socket: &BipSocket,
    datagram: OutgoingDatagram<'a>,
    to: SocketAddrV4,
) -> Tried<'a> {
    socket.note_attempt();
    socket.tap_outbound(&datagram, to);
    match socket.try_send(&datagram, to) {
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Tried::Blocked(datagram, to),
        sent => Tried::Done(sent),
    }
}
