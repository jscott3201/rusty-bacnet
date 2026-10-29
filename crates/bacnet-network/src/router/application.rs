//! The router's application entity as a [`TransportPort`], so a stack built
//! for one link, such as `bacnet-client`'s `BACnetClient` with its
//! transaction state machine, runs on the router itself (#1808); or as two,
//! so a client and a server share it (#2069).
//!
//! # The seam
//!
//! A router's application is reachable at the router's address on each of
//! its networks (Clause 6.1), but it has no link of its own. The port gives
//! it one, with no wire behind it:
//!
//! - **Sends** go to [`BACnetRouter::originate`]. The stack above writes an
//!   NPDU as it would for any remote device: DNET and DADR, or a broadcast to
//!   one network or to all of them. The port reads the destination, the
//!   APDU, the priority and the expecting-reply bit back out of it, and the
//!   router's table picks the route and the next hop. The link MAC the
//!   stack sends to plays no part, so `router_mac` arguments are ignored. An
//!   NPDU with no DNET, which would be a local send on a link the
//!   application lacks, and network messages, which are the router's own
//!   business, are refused.
//! - **Answers** to the confirmed requests the stack received go back
//!   through each request's saved reply plan instead
//!   ([`application_replies`](super::application_replies)): from the
//!   endpoint that was asked, along the path the request came by.
//! - **Receives** come from the router's local APDU receiver, through the
//!   [feed](super::application_feed): every APDU the router delivers to its
//!   application is handed up as an NPDU whose SNET and SADR say where it
//!   came from. An APDU a router relayed keeps its SNET and SADR. One from a
//!   device on a directly attached network gets that port's network as its
//!   SNET and the device's link MAC as its SADR, which is the station the
//!   request was addressed to, so its reply completes the transaction keyed
//!   to the routed destination. The link source MAC is passed up unchanged.
//! - **Limits** follow the router's ports: the smallest NPDU limits among
//!   them as they are now, which change when a port's transport publishes a
//!   new one (#2070).
//!
//! The router chooses the source address of each request
//! ([`SourceSelection`], the egress port's own by default), and its replies
//! reach the local receiver the way any APDU for the router does.

use std::io;
use std::sync::{Arc, Weak};

use bacnet_npdu::{MAX_NPDU, Npdu, NpduLimits};
use bacnet_router_core::router::{OriginateError, Request, SourceSelection};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bacnet_types::error::Error;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::application_feed::{Feed, outbound, slot};
use super::application_replies::Plan;
use super::application_transactions::Side;
use super::dispatch::{Outputs, Shared};
use super::originate::{OriginateFailure, originate};
use super::{BACnetRouter, LocalApduReceiver, StartedRouter};

/// Room for the network header in front of an APDU: what B/IP leaves
/// between its 1,497-octet NPDU and 1,476-octet APDU.
const NPCI_ROOM: u16 = (MAX_NPDU - 1476) as u16;

/// The application entity of a running [`BACnetRouter`] as a
/// [`TransportPort`] (#1808): give it to a stack that runs on a transport,
/// such as `bacnet-client`'s `BACnetClient`, and the stack's requests go
/// through the router's table, and its replies come back from the router.
///
/// Made by [`StartedRouter::into_application_port`], from the router's own
/// local APDU receiver: the stack on the port gets every APDU the router
/// delivers to its application. [`StartedRouter::into_application_ports`]
/// makes two instead, for a client and a server ([`RouterApplicationPorts`]).
/// A remote device is addressed by its network and MAC, as a routed device
/// (with any MAC as the router's; the table picks the next hop), whether its
/// network is directly attached or not. A device's reply carries its network
/// and MAC as its source either way. The port has no local network, so a
/// send with no DNET, such as a local broadcast, is refused with
/// [`Error::Encoding`], and so is a network message.
///
/// The stack's answers to confirmed requests it receives, such as the
/// acknowledgement of a confirmed COV notification, go back through the
/// reply plan the router saved with each request, for up to 30 seconds and
/// 64 requests at a time, as a server's replies do. A send the router
/// refuses there fails with the transport error the plan reports.
///
/// A refusal from [`BACnetRouter::originate`] fails the send:
/// [`OriginateError::TooLong`] as [`Error::SendTooLong`], every other one
/// as [`Error::Transport`] carrying the [`OriginateFailure`], which
/// [`OriginateFailure::from_error`] gets back. A network the table lacks
/// fails with [`OriginateError::NoRoute`] and starts a search for it, as
/// `originate` does; once the router has learned the route, a retry goes.
///
/// [`TransportPort::max_npdu`] reports the smallest NPDU limits among the
/// router's ports as they are now, and [`TransportPort::max_npdu_changes`]
/// publishes each change once the port has started, such as a BACnet/SC
/// port's on hub failover (#2070). [`TransportPort::egress_apdu_limit`]
/// follows them, so a stack that reads it for each request, as
/// `BACnetClient` does, builds the next request for the new limit.
///
/// [`TransportPort::local_receive_apdu_capacity`] is different: it is fixed
/// from the router ports' limits when the port is made, and a later fall
/// leaves it alone (#2087). The port can't know a future failover's limit,
/// so there is nothing smaller to declare at creation, and the trait keeps
/// the declaration stable so the stacks above need not follow a moving one.
/// A stack may therefore advertise more, such as a server's
/// Max_APDU_Length_Accepted, than a shrunken path carries back. The router
/// guards that path instead: a message routed onto it that is too long is
/// refused with reject reason 4 (message too long) to its source, and the
/// port's own send that is too long for it fails with
/// [`Error::SendTooLong`].
///
/// [`TransportPort::stop`] closes the port's queue. Stopping the router
/// closes its local receiver, which closes the port's queue too, and every
/// later send fails with [`OriginateFailure::Stopped`].
///
/// ```no_run
/// # use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort};
/// # use bacnet_transport::loopback::LoopbackTransport;
/// # async fn run(ports: Vec<RouterPort<LoopbackTransport>>) -> Result<(), bacnet_types::error::Error> {
/// let started = BACnetRouter::start(ports, RouterOptions::new()).await?;
/// let (router, port, _controls) = started.into_application_port();
/// // `BACnetClient::generic_builder().transport(port).build()` gives a
/// // client whose requests the router originates.
/// # Ok(())
/// # }
/// ```
pub struct RouterApplicationPort {
    outputs: Weak<Outputs>,
    shared: Arc<Shared>,
    feed: Arc<Feed>,
    /// `None` for the whole entity.
    side: Option<Side>,
    started: bool,
    local_mac: MacAddr,
    /// The router ports' smallest limits, as they change.
    limits: watch::Receiver<NpduLimits>,
    /// Those limits when the port was made, which fix its receive capacity.
    initial: NpduLimits,
    cap: Option<NpduLimits>,
    /// [`Self::max_npdu`]'s changes, for [`TransportPort::max_npdu_changes`].
    published: watch::Sender<NpduLimits>,
    /// The task that publishes them, once started.
    follow: Option<JoinHandle<()>>,
    source: SourceSelection,
}

/// The router's application entity split between a client and a server
/// (#2069), from [`StartedRouter::into_application_ports`]: give
/// [`Self::client`] to a `BACnetClient` and [`Self::server`] to a
/// `BACnetServer`, each with its `generic_builder`.
///
/// Both send as [`RouterApplicationPort`] does. What the router delivers is
/// shared out by what it is:
///
/// - a Confirmed-Request goes to the server, except a
///   ConfirmedCOVNotification, which goes to the client, and a
///   ConfirmedEventNotification, which goes to both: the server's answer is
///   sent and the client's dropped, unless the server's port can't take it
///   (not started, stopped or full), when the client's answer is sent. A
///   ConfirmedAuditNotification goes to the server alone;
/// - an Unconfirmed-Request goes to both, so the client hears I-Am, I-Have
///   and unconfirmed notifications, and the server Who-Is and Who-Has;
/// - an answer (Simple-ACK, Complex-ACK, Error, Reject, and a Segment-ACK or
///   Abort sent by the answering side) goes to the port whose stack sent the
///   request, matched by device and invoke ID among requests open or closed
///   within the last 10 seconds, or to the client when neither did. One the
///   client could mistake for a request that went out under another ID is
///   dropped;
/// - a Segment-ACK or Abort sent by a requester goes to the server.
///
/// The two stacks choose invoke IDs on their own. When both have a request
/// open to one device under the same ID, the later one goes out under
/// another ID free for that device, and its answers come back under its own.
/// Up to 1,024 requests are tracked, each for 60 seconds from its last
/// segment, retry or answer; past that the least recently used goes. Each
/// of the stacks' waits for one answer or segment, such as a client's APDU
/// timeout, must be shorter than 60 seconds, or a late answer goes
/// unmatched.
///
/// Each port's queue is small, and the router's local queue stays the one
/// that fills and drops. A stack that stops taking what its port holds for
/// 100 ms is taken as stalled: the other no longer waits for it, and what
/// arrives for it is dropped while its queue is full. It holds the other up
/// for at most 100 ms each time it frees a slot and stays full again.
///
/// ```no_run
/// # use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort};
/// # use bacnet_transport::loopback::LoopbackTransport;
/// # async fn run(ports: Vec<RouterPort<LoopbackTransport>>) -> Result<(), bacnet_types::error::Error> {
/// let started = BACnetRouter::start(ports, RouterOptions::new()).await?;
/// let (router, ports, _controls) = started.into_application_ports();
/// // `BACnetServer::generic_builder().transport(ports.server)` hosts a
/// // device on the router; `BACnetClient::generic_builder()
/// // .transport(ports.client)` reads others through it.
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub struct RouterApplicationPorts {
    /// The port for the client: the answers to its requests, notifications
    /// and every Unconfirmed-Request.
    pub client: RouterApplicationPort,
    /// The port for the server: the requests to the entity, the answers to
    /// its own requests and every Unconfirmed-Request.
    pub server: RouterApplicationPort,
}

impl<A: LocalApduReceiver, C> StartedRouter<A, C> {
    /// The router, its application entity as a [`TransportPort`] fed by
    /// the router's own local APDU receiver, raw or tracked, and the
    /// network-control receiver, if one was asked for (#1808). See
    /// [`RouterApplicationPort`].
    ///
    /// The port reports the smallest NPDU limits of the router's ports, as
    /// they change, and the longest of the router's MACs on them as its
    /// own: a request routed on from any port with the router's address as
    /// its source then fits.
    pub fn into_application_port(self) -> (BACnetRouter, RouterApplicationPort, Option<C>) {
        let port = self.router.application_port(self.apdus);
        (self.router, port, self.network_control)
    }

    /// [`Self::into_application_port`], with the entity split between a
    /// client and a server (#2069). See [`RouterApplicationPorts`], whose
    /// invoke-ID tracking needs each stack's APDU and segment timeouts under
    /// 60 seconds.
    pub fn into_application_ports(self) -> (BACnetRouter, RouterApplicationPorts, Option<C>) {
        let ports = self.router.application_ports(self.apdus);
        (self.router, ports, self.network_control)
    }
}

impl BACnetRouter {
    /// [`StartedRouter::into_application_port`], with `apdus`, which must
    /// be this router's own local receiver.
    pub(super) fn application_port<A: LocalApduReceiver>(&self, apdus: A) -> RouterApplicationPort {
        let feed = Arc::new(Feed::new(apdus.into_source(), false));
        self.port_on(feed, None)
    }

    /// [`StartedRouter::into_application_ports`], with `apdus`, which must
    /// be this router's own local receiver.
    pub(super) fn application_ports<A: LocalApduReceiver>(
        &self,
        apdus: A,
    ) -> RouterApplicationPorts {
        let feed = Arc::new(Feed::new(apdus.into_source(), true));
        RouterApplicationPorts {
            client: self.port_on(Arc::clone(&feed), Some(Side::Client)),
            server: self.port_on(feed, Some(Side::Server)),
        }
    }

    fn port_on(&self, feed: Arc<Feed>, side: Option<Side>) -> RouterApplicationPort {
        let local_mac = {
            let router = self.shared.engine.lock();
            let longest = router.ports().iter().map(|port| port.mac.as_slice()).fold(
                &[][..],
                |longest, mac| {
                    if mac.len() > longest.len() {
                        mac
                    } else {
                        longest
                    }
                },
            );
            MacAddr::from_slice(longest)
        };
        let limits = self.shared.application_limits.subscribe();
        let initial = *limits.borrow();
        RouterApplicationPort {
            outputs: self.outputs.clone(),
            shared: Arc::clone(&self.shared),
            feed,
            side,
            started: false,
            local_mac,
            limits,
            initial,
            cap: None,
            published: watch::Sender::new(initial),
            follow: None,
            source: SourceSelection::default(),
        }
    }
}

impl RouterApplicationPort {
    /// The router's address each request names as its source, by default
    /// [`SourceSelection::EgressPort`]: see [`Request::source`].
    #[must_use]
    pub fn with_source(mut self, source: SourceSelection) -> Self {
        self.source = source;
        self
    }

    /// Report at most `max_npdu` from [`TransportPort::max_npdu`], below
    /// the router ports' smallest limits when those are larger. The router
    /// still refuses a request too long for the port it leaves by.
    #[must_use]
    pub fn with_max_npdu(mut self, max_npdu: NpduLimits) -> Self {
        self.cap = Some(max_npdu);
        self.published
            .send_replace(capped(Some(max_npdu), *self.limits.borrow()));
        self
    }

    /// Send the NPDU a stack above wrote: an answer through its request's
    /// saved plan, anything else originated.
    async fn send(&self, npdu: &[u8]) -> Result<(), Error> {
        let npdu = Npdu::decode(npdu)
            .map_err(|error| Error::Encoding(format!("router application send: {error}")))?;
        if npdu.message_type.is_some() {
            return Err(Error::Encoding(
                "the router's application port carries APDUs only; network messages are \
                 the router's own"
                    .into(),
            ));
        }
        let Some(destination) = npdu.destination else {
            return Err(Error::Encoding(
                "the router's application has no network of its own: address a device by \
                 its network, or broadcast to one network or to all"
                    .into(),
            ));
        };
        let replies = &self.feed.replies[slot(self.side)];
        match replies.plan_for(destination, npdu.payload) {
            Some(Plan::Send(response)) => {
                return replies
                    .send(&response, npdu.payload, npdu.expecting_reply, npdu.priority)
                    .await;
            }
            // The server answers this one.
            Some(Plan::Silenced) => return Ok(()),
            None => {}
        }
        let apdu = match (self.side, &self.feed.transactions) {
            (Some(side), Some(transactions)) => {
                outbound(transactions, side, destination, npdu.payload)?
            }
            _ => npdu.payload.into(),
        };
        let mut request = Request::new(destination, &apdu);
        request.expecting_reply = npdu.expecting_reply;
        request.priority = npdu.priority;
        request.source = self.source;
        originate(&self.outputs, &self.shared, &request).map_err(failure_error)
    }

    fn end_tasks(&mut self) {
        self.feed.detach(slot(self.side));
        if let Some(task) = self.follow.take() {
            task.abort();
        }
    }

    /// How many reply plans the port keeps now.
    #[cfg(test)]
    pub(super) fn saved_replies(&self) -> usize {
        self.feed.replies[slot(self.side)].len()
    }
}

impl Drop for RouterApplicationPort {
    fn drop(&mut self) {
        self.end_tasks();
    }
}

impl std::fmt::Debug for RouterApplicationPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterApplicationPort")
            .field("side", &self.side)
            .field("local_mac", &self.local_mac)
            .field("max_npdu", &self.max_npdu())
            .field("source", &self.source)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

impl TransportPort for RouterApplicationPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        if std::mem::replace(&mut self.started, true) {
            return Err(Error::Encoding(
                "router application port already started".to_string(),
            ));
        }
        // Publish each change of the router's limits, capped. Nothing is
        // locked: a watch borrow, then a send that never waits.
        let mut limits = self.limits.clone();
        let published = self.published.clone();
        let cap = self.cap;
        self.follow = Some(tokio::spawn(async move {
            loop {
                let now = capped(cap, *limits.borrow_and_update());
                published.send_if_modified(|current| std::mem::replace(current, now) != now);
                if limits.changed().await.is_err() {
                    return;
                }
            }
        }));
        Ok(self.feed.attach(slot(self.side)))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        self.end_tasks();
        Ok(())
    }

    fn abort(&mut self) {
        self.end_tasks();
    }

    async fn send_unicast(&self, npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        self.send(npdu).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send(npdu).await
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// Fixed when the port is made: a declaration that limit changes leave
    /// alone (#2087). After a router port's limit falls, the router refuses
    /// what no longer fits that path, with reject reason 4 for a routed
    /// message and [`Error::SendTooLong`] for this port's own send.
    fn local_receive_apdu_capacity(&self) -> u16 {
        apdu_room(capped(self.cap, self.initial).unicast)
    }

    fn egress_apdu_limit(&self) -> u16 {
        apdu_room(self.max_npdu().unicast)
    }

    fn max_npdu(&self) -> NpduLimits {
        capped(self.cap, *self.limits.borrow())
    }

    fn max_npdu_changes(&self) -> Option<watch::Receiver<NpduLimits>> {
        Some(self.published.subscribe())
    }
}

/// `limits`, at most `cap`.
fn capped(cap: Option<NpduLimits>, limits: NpduLimits) -> NpduLimits {
    match cap {
        Some(cap) => NpduLimits {
            unicast: limits.unicast.min(cap.unicast),
            broadcast: limits.broadcast.min(cap.broadcast),
        },
        None => limits,
    }
}

/// The APDU room in an NPDU of `max_npdu` octets, at most 1,476.
fn apdu_room(max_npdu: u16) -> u16 {
    max_npdu.saturating_sub(NPCI_ROOM).min(1476)
}

/// The send error for an [`originate`] refusal.
fn failure_error(failure: OriginateFailure) -> Error {
    let kind = match failure {
        OriginateFailure::Refused(OriginateError::TooLong { octets, limit }) => {
            return Error::SendTooLong {
                length: octets,
                limit,
            };
        }
        OriginateFailure::Refused(OriginateError::NoRoute | OriginateError::Unreachable) => {
            io::ErrorKind::NetworkUnreachable
        }
        OriginateFailure::Refused(OriginateError::Busy) | OriginateFailure::QueueFull => {
            io::ErrorKind::WouldBlock
        }
        OriginateFailure::Stopped => io::ErrorKind::NotConnected,
        _ => io::ErrorKind::InvalidInput,
    };
    Error::Transport(io::Error::new(kind, failure))
}

impl OriginateFailure {
    /// The refusal behind `error`, when it is a send of a
    /// [`RouterApplicationPort`] that [`BACnetRouter::originate`] refused,
    /// other than for length ([`Error::SendTooLong`]).
    #[must_use]
    pub fn from_error(error: &Error) -> Option<Self> {
        match error {
            Error::Transport(error) => error.get_ref()?.downcast_ref::<Self>().copied(),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "application_tests.rs"]
mod tests;
