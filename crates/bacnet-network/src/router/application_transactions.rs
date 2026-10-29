//! Which of the two stacks on the router's application entity owns each
//! confirmed request it sent, so that every answer reaches the stack that
//! asked (#2069).
//!
//! A client and a server on [split ports](super::RouterApplicationPorts)
//! both send confirmed requests from the one entity: the client its reads
//! and writes, the server its notifications, writes to Command targets and
//! binding probes. An answer names only the device it came from and the
//! invoke ID, and the two stacks pick their invoke IDs on their own, so the
//! table here keeps, for each request, who sent it, to which device, with
//! which ID.
//!
//! When both stacks have a request to one device under one invoke ID, the
//! later one goes out under another ID that device has free, and its
//! answers are given back under the ID its stack chose. Until then, an ID
//! goes out unchanged.
//!
//! A request its final answer closes is kept for [`CLOSED`] more, so that
//! the requester's last Segment-ACK, and a device's repeat of its final
//! segment or answer, still go by its IDs.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use bacnet_npdu::Address;
use bacnet_types::MacAddr;
use tokio::time::{Duration, Instant};

/// The most requests tracked at once, open or recently closed; a new one
/// past it pushes out the one used least recently, even if still open.
pub(super) const CAPACITY: usize = 1024;

/// How long an open request is kept since it was last sent or answered.
/// Each of the stacks' waits for one answer or segment (its APDU and
/// segment timeouts) must be shorter, or a late answer goes unmatched.
pub(super) const LIFETIME: Duration = Duration::from_secs(60);

/// How long a closed request is kept since its final answer.
pub(super) const CLOSED: Duration = Duration::from_secs(10);

// docs/rust-api.md names these numbers: change them together.
const _: () = assert!(CAPACITY == 1024 && LIFETIME.as_secs() == 60 && CLOSED.as_secs() == 10);

/// The two stacks that share the entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    /// The stack that sent the request as a client, such as `BACnetClient`.
    Client,
    /// The stack that serves the entity, such as `BACnetServer`.
    Server,
}

/// Who an answer goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Owner {
    /// The stack whose request it answers, open or recently closed, and
    /// the invoke ID that stack chose.
    Stack(Side, u8),
    /// No request matches.
    Unknown,
    /// No request matches under this ID, but a translated one went out
    /// under another while its stack knows it by this one: handing the
    /// answer up unchanged could complete the wrong transaction.
    Ambiguous,
}

/// One request.
struct Request {
    side: Side,
    network: u16,
    mac: MacAddr,
    /// The invoke ID its stack chose.
    local: u8,
    /// The invoke ID it went out under.
    wire: u8,
    used_at: Instant,
    closed: bool,
}

impl Request {
    fn to(&self, network: u16, mac: &[u8]) -> bool {
        self.network == network && self.mac.as_slice() == mac
    }

    fn live(&self, now: Instant) -> bool {
        let keep = if self.closed { CLOSED } else { LIFETIME };
        now.duration_since(self.used_at) < keep
    }
}

/// The requests the two stacks sent, oldest use first.
#[derive(Default)]
pub(super) struct Transactions {
    requests: Mutex<VecDeque<Request>>,
}

impl Transactions {
    fn requests(&self) -> MutexGuard<'_, VecDeque<Request>> {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        requests.retain(|request| request.live(now));
        requests
    }

    /// The invoke ID a Confirmed-Request (or one segment of it) that `side`
    /// sends to `to` under its own `local` ID goes out under: the one the
    /// request already has, which a new request under a closed one's ID
    /// reopens, else `local` unless the other stack's request to that
    /// device holds it, else the next ID free there. `None` when every ID
    /// to that device is taken.
    pub(super) fn request(&self, side: Side, to: Address<'_>, local: u8) -> Option<u8> {
        let mut requests = self.requests();
        let now = Instant::now();
        if let Some(index) = position(&requests, side, to, local) {
            let request = refresh(&mut requests, index, now);
            request.closed = false;
            return Some(request.wire);
        }
        // One pass over the table, then over the IDs.
        let mut taken = [false; 256];
        for request in requests
            .iter()
            .filter(|request| request.to(to.network, to.mac))
        {
            taken[usize::from(request.wire)] = true;
        }
        let wire = (0..=u8::MAX)
            .map(|step| local.wrapping_add(step))
            .find(|&wire| !taken[usize::from(wire)])?;
        if requests.len() == CAPACITY {
            requests.pop_front();
        }
        requests.push_back(Request {
            side,
            network: to.network,
            mac: MacAddr::from_slice(to.mac),
            local,
            wire,
            used_at: now,
            closed: false,
        });
        Some(wire)
    }

    /// The invoke ID a Segment-ACK or Abort that `side` sends as the
    /// requester, to `to` under its own `local` ID, goes out under: its
    /// request's, open or recently closed, or `local` when it has none. An
    /// Abort (`ends`) closes the request.
    pub(super) fn requester(&self, side: Side, to: Address<'_>, local: u8, ends: bool) -> u8 {
        let mut requests = self.requests();
        let Some(index) = position(&requests, side, to, local) else {
            return local;
        };
        let request = refresh(&mut requests, index, Instant::now());
        request.closed |= ends;
        request.wire
    }

    /// Who an answer from `network` and `mac` under `wire` goes to. An
    /// answer that ends the request (`ends`) closes it.
    pub(super) fn answer(&self, network: u16, mac: &[u8], wire: u8, ends: bool) -> Owner {
        let mut requests = self.requests();
        let Some(index) = requests
            .iter()
            .position(|request| request.to(network, mac) && request.wire == wire)
        else {
            let shadowed = requests.iter().any(|request| {
                request.to(network, mac) && request.local == wire && request.wire != wire
            });
            return if shadowed {
                Owner::Ambiguous
            } else {
                Owner::Unknown
            };
        };
        let request = refresh(&mut requests, index, Instant::now());
        request.closed |= ends;
        Owner::Stack(request.side, request.local)
    }

    /// How many requests are kept now, open or closed.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.requests().len()
    }
}

/// Where `side`'s request to `to` under `local` is kept.
fn position(requests: &VecDeque<Request>, side: Side, to: Address<'_>, local: u8) -> Option<usize> {
    requests.iter().position(|request| {
        request.side == side && request.to(to.network, to.mac) && request.local == local
    })
}

/// Mark `requests[index]` used at `now`, moving it to the back, and return
/// it.
fn refresh(requests: &mut VecDeque<Request>, index: usize, now: Instant) -> &mut Request {
    let mut request = requests.remove(index).expect("the index is in range");
    request.used_at = now;
    requests.push_back(request);
    requests.back_mut().expect("just pushed")
}
