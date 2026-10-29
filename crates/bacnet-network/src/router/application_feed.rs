//! The router's local deliveries, handed to the stack or stacks on its
//! application ports (#1808, #2069).
//!
//! One task takes each APDU off the router's local queue and passes it up
//! as an NPDU whose SNET and SADR name its origin. With the
//! [whole entity](super::StartedRouter::into_application_port) on one port,
//! everything goes to it. With the entity
//! [split](super::StartedRouter::into_application_ports) between a client
//! and a server, it goes by what it is:
//!
//! - a Confirmed-Request goes to the server, unless it is a COV
//!   notification, which only the client takes, or an event notification,
//!   which goes to both: the server's answer is sent, and the client's is
//!   dropped, unless the server's port can't take it, when the client
//!   answers;
//! - an Unconfirmed-Request goes to both: the client takes I-Am, I-Have and
//!   notifications, the server Who-Is, Who-Has and the rest, and each
//!   ignores what it doesn't use;
//! - an answer (Simple-ACK, Complex-ACK, Error, Reject, and a Segment-ACK
//!   or Abort with its server bit set, so sent by the side that answers)
//!   goes to whichever stack sent the request it answers, found by device
//!   and invoke ID ([`Transactions`]) among requests open or closed in the
//!   last few seconds, or to the client when none did, unless the client
//!   could take it for a request that went out under another ID: that one
//!   is dropped;
//! - a Segment-ACK or Abort with the server bit clear comes from a device
//!   acting as the requester, so it goes to the server.
//!
//! # Queues
//!
//! Each port holds a few NPDUs, and the task takes an APDU off the router's
//! queue only once each consumer that is keeping up has room for it, so the
//! router's queue, with its admission accounting and per-source quota, stays
//! the buffer that fills and drops. A consumer that gives no room for
//! [`STALL`] is taken as stalled: the task stops waiting for it, and the
//! APDUs meant for it are dropped while its queue stays full, until it has
//! room again. A stalled consumer therefore holds the other one up for at
//! most [`STALL`] each time it frees a slot and then stays full again. When
//! no consumer has room, the task waits for one, and the router's queue
//! drops. The router itself never waits.

use std::borrow::Cow;
use std::sync::{Arc, Mutex, PoisonError};

use bacnet_npdu::{Address, GLOBAL_NETWORK, NetworkPriority, Npdu};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::error::Error;
use bytes::Bytes;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, timeout_at};
use tracing::debug;

use super::application_replies::Replies;
use super::application_transactions::{Owner, Side, Transactions};
use super::options::sealed::ApduSource;
use crate::layer::ReceivedApdu;

/// The NPDUs each port holds for the stack above. Few: see the module docs.
pub(super) const QUEUE: usize = 4;

/// How long the task waits for room in one consumer's queue before it
/// takes that consumer as stalled.
pub(super) const STALL: Duration = Duration::from_millis(100);

// `RouterApplicationPorts`' docs and docs/rust-api.md name it: change them
// together.
const _: () = assert!(STALL.as_millis() == 100);

/// The consumers' queues, by slot: 0 is the client or the whole entity,
/// 1 the server.
#[derive(Default)]
pub(super) struct Consumers {
    slots: [Option<mpsc::Sender<ReceivedNpdu>>; 2],
    /// Set once the router's queue has closed: a port started after that
    /// gets a closed queue.
    closed: bool,
}

/// The slot of a port on `side`, or of the whole entity's.
pub(super) fn slot(side: Option<Side>) -> usize {
    match side {
        None | Some(Side::Client) => 0,
        Some(Side::Server) => 1,
    }
}

/// What the ports of one application entity share: the router's local
/// queue until the first start takes it, the consumers' queues, the reply
/// plans of the requests each received, and, when split, who sent which
/// request.
pub(super) struct Feed {
    consumers: Arc<watch::Sender<Consumers>>,
    source: Mutex<Option<ApduSource>>,
    task: Mutex<Option<JoinHandle<()>>>,
    pub(super) replies: [Arc<Replies>; 2],
    /// `None` for the whole entity on one port.
    pub(super) transactions: Option<Arc<Transactions>>,
}

impl Feed {
    /// The feed from `source`, to one port (`split` false) or two.
    pub(super) fn new(source: ApduSource, split: bool) -> Self {
        Self {
            consumers: Arc::new(watch::Sender::new(Consumers::default())),
            source: Mutex::new(Some(source)),
            task: Mutex::new(None),
            replies: Default::default(),
            transactions: split.then(Arc::default),
        }
    }

    /// Give `slot` a new queue, start the task if it hasn't started, and
    /// return the queue's receiving end.
    pub(super) fn attach(&self, slot: usize) -> mpsc::Receiver<ReceivedNpdu> {
        let (tx, rx) = mpsc::channel(QUEUE);
        self.consumers.send_modify(|consumers| {
            if !consumers.closed {
                consumers.slots[slot] = Some(tx);
            }
        });
        let source = self
            .source
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(source) = source {
            let task = tokio::spawn(run(
                source,
                Arc::clone(&self.consumers),
                Router {
                    replies: self.replies.clone(),
                    transactions: self.transactions.clone(),
                },
            ));
            *self.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
        }
        rx
    }

    /// Close `slot`'s queue: the task drops what it would have handed it.
    pub(super) fn detach(&self, slot: usize) {
        self.consumers
            .send_modify(|consumers| consumers.slots[slot] = None);
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        if let Some(task) = self
            .task
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

/// `future`, or `None` once the consumers change first.
async fn unless_changed<F: Future>(
    consumers: &mut watch::Receiver<Consumers>,
    future: F,
) -> Option<F::Output> {
    tokio::select! {
        output = future => Some(output),
        _ = consumers.changed() => None,
    }
}

/// Take APDUs off `apdus` and hand them up, as the module docs say, until
/// the router's queue closes. No lock is held across an await: the waits
/// are for room in the consumers' queues, the router's queue, and a change
/// of consumers, which restarts the round.
async fn run(mut apdus: ApduSource, consumers: Arc<watch::Sender<Consumers>>, router: Router) {
    let mut changes = consumers.subscribe();
    let mut stalled = [false; 2];
    loop {
        let senders = changes.borrow_and_update().slots.clone();
        let live = senders
            .each_ref()
            .map(|tx| tx.as_ref().filter(|tx| !tx.is_closed()));
        if live.iter().all(Option::is_none) {
            if changes.changed().await.is_err() {
                return;
            }
            continue;
        }
        // Room in each consumer that is keeping up, each within STALL.
        let deadline = Instant::now() + STALL;
        let rooms = async {
            tokio::join!(
                room(live[0], stalled[0], deadline),
                room(live[1], stalled[1], deadline)
            )
        };
        let Some(rooms) = unless_changed(&mut changes, rooms).await else {
            continue;
        };
        let mut permits = [None, None];
        for (index, room) in <[Room<'_>; 2]>::from(rooms).into_iter().enumerate() {
            match room {
                Room::Permit(permit) => {
                    stalled[index] = false;
                    permits[index] = Some(permit);
                }
                Room::Stalled => {
                    if !stalled[index] {
                        debug!(slot = index, "router application: a consumer stalled");
                    }
                    stalled[index] = true;
                }
                Room::None => {}
            }
        }
        if permits.iter().all(Option::is_none) {
            // Nobody has room: wait for the first that does.
            let first = async {
                match live {
                    [Some(a), Some(b)] => tokio::select! {
                        permit = a.reserve() => (0, permit),
                        permit = b.reserve() => (1, permit),
                    },
                    [Some(a), None] => (0, a.reserve().await),
                    [None, Some(b)] => (1, b.reserve().await),
                    [None, None] => unreachable!("one consumer is live"),
                }
            };
            match unless_changed(&mut changes, first).await {
                Some((index, Ok(permit))) => {
                    stalled[index] = false;
                    permits[index] = Some(permit);
                }
                // Closed or changed: look again.
                _ => continue,
            }
        }
        let apdu = match unless_changed(&mut changes, apdus.recv()).await {
            None => continue,
            Some(Some(apdu)) => apdu,
            Some(None) => break,
        };
        // A stalled consumer gets an APDU only if it has room now. Which
        // consumers can take it is settled before delivery, so a reply plan
        // goes to a stack that receives its request.
        for (index, permit) in permits.iter_mut().enumerate() {
            if permit.is_none() {
                *permit = live[index].and_then(|tx| tx.try_reserve().ok());
            }
        }
        let room = permits.each_ref().map(Option::is_some);
        for (index, npdu) in router.deliver(apdu, room).into_iter().enumerate() {
            let Some(npdu) = npdu else { continue };
            match permits[index].take() {
                Some(permit) => {
                    permit.send(npdu);
                    stalled[index] = false;
                }
                None => debug!(
                    slot = index,
                    "router application: an APDU dropped for a consumer without room"
                ),
            }
        }
    }
    // The router's queue closed: so do the consumers'.
    consumers.send_modify(|consumers| {
        consumers.slots = [None, None];
        consumers.closed = true;
    });
}

/// What a consumer's queue offers at the start of a round.
enum Room<'a> {
    /// Room for one NPDU.
    Permit(mpsc::Permit<'a, ReceivedNpdu>),
    /// No room within [`STALL`], or, for a stalled consumer, none now.
    Stalled,
    /// No consumer, or a closed queue.
    None,
}

/// What `tx` offers: room by `deadline`, or, when `stalled`, room now.
async fn room(
    tx: Option<&mpsc::Sender<ReceivedNpdu>>,
    stalled: bool,
    deadline: Instant,
) -> Room<'_> {
    let Some(tx) = tx else {
        return Room::None;
    };
    if stalled {
        return match tx.try_reserve() {
            Ok(permit) => Room::Permit(permit),
            Err(mpsc::error::TrySendError::Full(())) => Room::Stalled,
            Err(mpsc::error::TrySendError::Closed(())) => Room::None,
        };
    }
    match timeout_at(deadline, tx.reserve()).await {
        Ok(Ok(permit)) => Room::Permit(permit),
        Ok(Err(_)) => Room::None,
        Err(_) => Room::Stalled,
    }
}

/// What the task needs to hand an APDU up.
struct Router {
    replies: [Arc<Replies>; 2],
    transactions: Option<Arc<Transactions>>,
}

impl Router {
    /// `apdu` as the NPDU each slot gets, if any: with its origin as SNET
    /// and SADR, an answer's invoke ID given back as its stack chose it, and
    /// a Confirmed-Request's reply plan kept for the stack that answers it,
    /// of those with `room` for it. Nothing, logged, when it has no origin
    /// that can be written.
    fn deliver(&self, apdu: ReceivedApdu, room: [bool; 2]) -> [Option<ReceivedNpdu>; 2] {
        let (network, mac) = match (&apdu.source_network, apdu.ingress_network) {
            (Some(source), _) => (source.network, source.mac_address.as_slice()),
            (None, Some(network)) => (network, apdu.source_mac.as_slice()),
            (None, None) => {
                debug!("router application: an APDU with no ingress network");
                return [None, None];
            }
        };
        let mut payload = Cow::Borrowed(&apdu.apdu[..]);
        let slots = match &self.transactions {
            None => [true, false],
            Some(transactions) => match classify(&apdu.apdu) {
                Route::To(side) => to(side),
                Route::Both | Route::ServerAnswers => [true, true],
                Route::Answer { invoke_id, ends } => {
                    match transactions.answer(network, mac, invoke_id, ends) {
                        Owner::Stack(side, local) => {
                            if local != invoke_id {
                                payload.to_mut()[1] = local;
                            }
                            to(side)
                        }
                        Owner::Unknown => to(Side::Client),
                        Owner::Ambiguous => {
                            debug!(
                                invoke_id,
                                "router application: an answer that would match a translated request"
                            );
                            return [None, None];
                        }
                    }
                }
                Route::Neither => {
                    debug!("router application: an APDU neither stack takes");
                    return [None, None];
                }
            },
        };
        let npdu = Npdu {
            expecting_reply: false,
            priority: NetworkPriority::Normal,
            destination: apdu.global_broadcast.then_some(Address {
                network: GLOBAL_NETWORK,
                mac: &[],
            }),
            source: Some(Address { network, mac }),
            hop_count: 255,
            message_type: None,
            vendor_id: None,
            payload: &payload,
            payload_offset: 0,
        };
        let mut buf = vec![0; npdu.encoded_len()];
        if let Err(error) = npdu.encode_into(&mut buf) {
            debug!(%error, "router application: an APDU with no writable source");
            return [None, None];
        }
        if let Some(response) = apdu.router_response {
            // A Confirmed-Request. One that goes to both is the server's to
            // answer, and the client's answer is swallowed, when the server
            // gets it; else the client answers.
            if slots == [true, true] && room[1] {
                self.replies[1].save(network, mac, &apdu.apdu, response);
                self.replies[0].silence(network, mac, &apdu.apdu);
            } else {
                let slot = if slots == [true, true] {
                    0
                } else {
                    usize::from(slots[1])
                };
                self.replies[slot].save(network, mac, &apdu.apdu, response);
            }
        }
        let npdu = Bytes::from(buf);
        let up = |npdu: Bytes| ReceivedNpdu {
            npdu,
            source_mac: apdu.source_mac.clone(),
            link_layer_group: apdu.is_group && !apdu.global_broadcast,
            data_attributes: apdu.data_attributes.clone(),
            provenance: apdu.provenance.clone(),
            // Answers go back through the saved plan, not on the arrival
            // link's reply paths.
            direct_response: None,
            reply_tx: None,
        };
        slots.map(|wanted| wanted.then(|| up(npdu.clone())))
    }
}

/// The slots of `side` alone.
fn to(side: Side) -> [bool; 2] {
    match side {
        Side::Client => [true, false],
        Side::Server => [false, true],
    }
}

/// Where an APDU delivered to a split entity goes, by its PDU type and
/// server bit, before any invoke ID is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    /// A request, or a requester's Segment-ACK or Abort: to `Side`.
    To(Side),
    /// An Unconfirmed-Request: to both.
    Both,
    /// A ConfirmedEventNotification: to both, for the server to answer.
    ServerAnswers,
    /// An answer to a request one of the stacks sent, to be matched by
    /// `invoke_id`; `ends` when it closes that request.
    Answer {
        /// Its invoke ID.
        invoke_id: u8,
        /// Whether no more of the exchange follows.
        ends: bool,
    },
    /// Malformed, or no APDU type: neither stack takes it.
    Neither,
}

/// [`Route`] for `apdu`.
pub(super) fn classify(apdu: &[u8]) -> Route {
    let Some(&first) = apdu.first() else {
        return Route::Neither;
    };
    let server_bit = first & 0x01 != 0;
    let answer = |ends| match apdu.get(1) {
        Some(&invoke_id) => Route::Answer { invoke_id, ends },
        None => Route::Neither,
    };
    match first >> 4 {
        0x0 => {
            // The service choice follows the sequence number and window of a
            // segmented request, else the invoke ID.
            let service = apdu.get(if first & 0x08 != 0 { 5 } else { 3 });
            match service {
                // ConfirmedCOVNotification: only the client takes it.
                Some(1) => Route::To(Side::Client),
                // ConfirmedEventNotification: the client hears it, and the
                // server takes it too, for its Notification Forwarders.
                Some(2) => Route::ServerAnswers,
                _ => Route::To(Side::Server),
            }
        }
        0x1 => Route::Both,
        0x2 | 0x5 | 0x6 => answer(true),
        // A Complex-ACK with more segments to follow leaves it open.
        0x3 => answer(first & 0x04 == 0),
        0x4 if server_bit => answer(false),
        0x7 if server_bit => answer(true),
        0x4 | 0x7 => Route::To(Side::Server),
        _ => Route::Neither,
    }
}

/// `apdu`, which `side` sends to `to`, with the invoke ID it goes out
/// under: a Confirmed-Request's, or a requester's Segment-ACK's or Abort's,
/// translated when the two stacks' requests to that device share an ID.
/// Refused when every invoke ID to that device is in use.
pub(super) fn outbound<'a>(
    transactions: &Transactions,
    side: Side,
    to: Address<'_>,
    apdu: &'a [u8],
) -> Result<Cow<'a, [u8]>, Error> {
    // A broadcast is no transaction.
    if to.mac.is_empty() {
        return Ok(Cow::Borrowed(apdu));
    }
    let (Some(&first), Some(&id1), Some(&id2)) = (apdu.first(), apdu.get(1), apdu.get(2)) else {
        return Ok(Cow::Borrowed(apdu));
    };
    let server_bit = first & 0x01 != 0;
    let (at, local, wire) = match first >> 4 {
        0x0 => {
            let wire = transactions.request(side, to, id2).ok_or_else(|| {
                Error::Transport(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "every invoke ID to the device is in use by the router's application",
                ))
            })?;
            (2, id2, wire)
        }
        0x4 if !server_bit => (1, id1, transactions.requester(side, to, id1, false)),
        0x7 if !server_bit => (1, id1, transactions.requester(side, to, id1, true)),
        _ => return Ok(Cow::Borrowed(apdu)),
    };
    if wire == local {
        return Ok(Cow::Borrowed(apdu));
    }
    let mut apdu = apdu.to_vec();
    apdu[at] = wire;
    Ok(Cow::Owned(apdu))
}

#[cfg(test)]
#[path = "application_feed_tests.rs"]
mod tests;
