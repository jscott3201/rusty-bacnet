//! Per-source rate limit for network-layer messages at SC NPDU ingress (#518).
//!
//! Routing-control messages (I-Am-Router-To-Network, Router-Busy and the
//! rest of Clause 6.4) can change a router's table and are relayed to its
//! other ports, so one peer sending them fast costs every port. The queue
//! quota in `npdu_admission` bounds how many NPDUs one source holds at once;
//! this bounds how many network-layer messages one source may hand in over
//! time.
//!
//! Each source has a token bucket: `burst` messages at once, refilling at
//! `per_second`. A message over budget is dropped before it takes a queue
//! slot. Only network-layer messages are charged; APDUs, unicast or
//! broadcast, never touch a bucket, so a control flood can't starve
//! application traffic from that source or any other.
//!
//! A direct peer's source is its verified certificate principal, so one
//! certificate has one budget across VMACs and reconnects. A hub-relayed
//! message's source is its originating VMAC, the hub's registration of its
//! client: a hub client that registers under several VMACs, one after
//! another or through several hubs, gets a budget for each.
//!
//! Bounds: at most [`CONTROL_RATE_SOURCES`] buckets. When the table is full,
//! buckets that have refilled completely are dropped first (losing nothing,
//! as a new bucket starts full). If none has, a new source shares one
//! overflow bucket, so sources already tracked keep their own budgets
//! however many new VMACs appear. The price: a sender holding all 256
//! buckets drained (256 VMACs or certificates, each flooding) leaves every
//! new source only the shared overflow budget for its control messages until
//! some bucket refills.

use std::collections::HashMap;

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use tokio::time::Instant;

use crate::port::ScPrincipal;

use super::npdu_admission::ScIngressPath;

/// Most sources with their own control-message bucket per SC NPDU queue.
pub(crate) const CONTROL_RATE_SOURCES: usize = 256;

/// Per-source limit on inbound network-layer messages for one SC NPDU queue.
///
/// The default allows a burst of 256 and 32 a second after it: room for a
/// router's start-up announcements, discovery and the replies of many
/// routers on one link, and still about a quarter of the hub's own
/// per-sender broadcast budget (#644). A site where one peer legitimately
/// sends more, such as a router fronting many networks that each announce
/// separately, raises it. Both values must be at least 1, and the burst at
/// most 4,096.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScControlRateLimit {
    /// Messages one source may send back to back.
    pub burst: u32,
    /// Messages one source may send each second once its burst is spent.
    pub per_second: u32,
}

impl Default for ScControlRateLimit {
    fn default() -> Self {
        Self::new(256, 32)
    }
}

impl ScControlRateLimit {
    /// A limit of `burst` messages at once and `per_second` after it;
    /// [`validate`](Self::validate) checks it.
    pub const fn new(burst: u32, per_second: u32) -> Self {
        Self { burst, per_second }
    }

    /// Check the limit before any transport I/O or state change.
    pub fn validate(&self) -> Result<(), Error> {
        if self.burst == 0 || self.burst > 4096 || self.per_second == 0 {
            return Err(Error::OutOfRange(format!(
                "BACnet/SC control-message rate needs a burst in 1..=4096 and a \
                 nonzero rate, got burst {} at {}/s",
                self.burst, self.per_second
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    refilled: Instant,
}

impl Bucket {
    fn full(limit: ScControlRateLimit, now: Instant) -> Self {
        Self {
            tokens: f64::from(limit.burst),
            refilled: now,
        }
    }

    fn refill(&mut self, limit: ScControlRateLimit, now: Instant) {
        let elapsed = now.saturating_duration_since(self.refilled).as_secs_f64();
        self.tokens =
            (self.tokens + elapsed * f64::from(limit.per_second)).min(f64::from(limit.burst));
        self.refilled = now;
    }

    fn take(&mut self, limit: ScControlRateLimit, now: Instant) -> bool {
        self.refill(limit, now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn is_full(&self, limit: ScControlRateLimit, now: Instant) -> bool {
        let mut probe = *self;
        probe.refill(limit, now);
        probe.tokens >= f64::from(limit.burst)
    }
}

/// Whose budget a network-layer message spends.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ControlSource {
    /// A verified direct peer, whatever VMAC it claims.
    Principal(ScPrincipal),
    /// Anything else: the ingress path and source VMAC.
    Vmac(ScIngressPath, MacAddr),
}

/// The buckets of one SC NPDU queue. Guarded by the admission lock.
#[derive(Debug)]
pub(crate) struct ControlRate {
    limit: ScControlRateLimit,
    sources: HashMap<ControlSource, Bucket>,
    overflow: Option<Bucket>,
}

impl ControlRate {
    pub(crate) fn new(limit: ScControlRateLimit) -> Self {
        Self {
            limit,
            sources: HashMap::new(),
            overflow: None,
        }
    }

    /// Whether `key` may hand in one more network-layer message now,
    /// charging its bucket if so.
    pub(crate) fn admit(&mut self, key: ControlSource, now: Instant) -> bool {
        let limit = self.limit;
        if let Some(bucket) = self.sources.get_mut(&key) {
            return bucket.take(limit, now);
        }
        if self.sources.len() >= CONTROL_RATE_SOURCES {
            self.sources.retain(|_, bucket| !bucket.is_full(limit, now));
        }
        if self.sources.len() < CONTROL_RATE_SOURCES {
            let mut bucket = Bucket::full(limit, now);
            let admitted = bucket.take(limit, now);
            self.sources.insert(key, bucket);
            return admitted;
        }
        self.overflow
            .get_or_insert_with(|| Bucket::full(limit, now))
            .take(limit, now)
    }

    #[cfg(test)]
    pub(crate) fn tracked(&self) -> usize {
        self.sources.len()
    }
}

/// Whether `npdu` carries a network-layer message rather than an APDU: an
/// ASHRAE NPDU (version 1) with the control octet's message bit set
/// (Clause 6.2.2). Anything shorter or of another version is left to the
/// network layer, which refuses it there.
pub(crate) fn is_network_message(npdu: &[u8]) -> bool {
    matches!(npdu, [0x01, control, ..] if control & 0x80 != 0)
}

#[cfg(test)]
#[path = "control_rate_tests.rs"]
mod tests;
