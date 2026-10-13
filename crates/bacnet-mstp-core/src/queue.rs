//! Bounded, observable MS/TP NPDU queues.
//!
//! The queue implementations deliberately own their payload bytes.  This is
//! important for the core API: a caller may hand an NPDU to the core from a
//! short-lived receive buffer, while the core must retain it until a token is
//! available.  The alloc implementation reserves all of its item storage in
//! its constructor; enqueue and dequeue do not grow a vector.
//!
//! A dequeue lends the NPDU from where it was queued rather than copying it
//! out, so the core builds a frame straight from the queue's slot, with no
//! NPDU-sized buffer on its stack.

use core::fmt;

use bacnet_mstp_codec::{BROADCAST_MAC, MAX_STANDARD_MPDU_DATA};
use bacnet_time::Instant;

use crate::counters::QueueCounters;

/// Destination class used by the two independently bounded queues.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum QueueKind {
    /// A frame addressed to one MAC address.
    Unicast,
    /// A frame addressed to the MS/TP broadcast MAC
    /// ([`BROADCAST_MAC`], 255).
    Broadcast,
}

impl QueueKind {
    /// Classify a destination MAC.
    #[must_use]
    pub const fn from_destination(destination: u8) -> Self {
        if destination == BROADCAST_MAC {
            Self::Broadcast
        } else {
            Self::Unicast
        }
    }
}

/// BACnet network priorities, in increasing service order.
///
/// Clause 6.2.2 assigns the numeric values shown here.  Consequently
/// [`LifeSafety`](Self::LifeSafety) is selected before
/// [`CriticalEquipment`](Self::CriticalEquipment), then [`Urgent`](Self::Urgent),
/// and finally [`Normal`](Self::Normal).
///
/// The field is two bits, so the set is closed: convert from the wire value
/// with `try_from` and back with `u8::from`.
///
/// ```
/// use bacnet_mstp_core::NetworkPriority;
///
/// assert_eq!(NetworkPriority::try_from(3), Ok(NetworkPriority::LifeSafety));
/// assert_eq!(u8::from(NetworkPriority::Urgent), 1);
/// assert!(NetworkPriority::try_from(4).is_err());
/// ```
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum NetworkPriority {
    /// Normal traffic (`0`).
    Normal = 0,
    /// Urgent traffic (`1`).
    Urgent = 1,
    /// Critical-equipment traffic (`2`).
    CriticalEquipment = 2,
    /// Life-safety traffic (`3`).
    LifeSafety = 3,
}

/// The wire priority value, 0 to 3; anything else is refused.
impl TryFrom<u8> for NetworkPriority {
    type Error = PriorityError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Normal),
            1 => Ok(Self::Urgent),
            2 => Ok(Self::CriticalEquipment),
            3 => Ok(Self::LifeSafety),
            _ => Err(PriorityError { value }),
        }
    }
}

/// The Clause 6.2.2 numeric value.
impl From<NetworkPriority> for u8 {
    fn from(priority: NetworkPriority) -> Self {
        priority as u8
    }
}

/// A value [`NetworkPriority::try_from`] refuses: a network priority is two
/// bits, 0 to 3.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub struct PriorityError {
    /// The value offered.
    pub value: u8,
}

impl fmt::Display for PriorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "network priority {} is not 0 to 3", self.value)
    }
}

impl core::error::Error for PriorityError {}

/// Why a queue, or [`MasterCore::enqueue_npdu`](crate::MasterCore::enqueue_npdu),
/// refused an NPDU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QueueRefusal {
    /// The destination queue has no free slot.
    Full,
    /// The payload is too long: larger than the queue's owned slot or, from
    /// [`MasterCore::enqueue_npdu`](crate::MasterCore::enqueue_npdu), than
    /// the station sends
    /// ([`MstpCoreConfig::max_npdu`](crate::MstpCoreConfig::max_npdu)).
    /// A dequeue also returns it when it dropped an item longer than the
    /// caller takes (see [`QueueStorage::dequeue`]).
    Oversize,
    /// The supplied expiry is at or before the enqueue time, or an item
    /// reached its expiry boundary while waiting.
    Expired,
}

impl fmt::Display for QueueRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "the queue is full",
            Self::Oversize => "the NPDU is too long to queue or send",
            Self::Expired => "the NPDU expired",
        })
    }
}

impl core::error::Error for QueueRefusal {}

/// An NPDU taken off its queue, with its octets lent by the queue.
///
/// The item has left the queue, but [`Self::npdu`] borrows the queue: the
/// octets stay where they were queued until the queue is next used
/// mutably, so the caller reads or sends them from there instead of copying
/// them out.
///
/// It is `#[non_exhaustive]`: a [`QueueStorage`] of your own builds one with
/// [`Self::new`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DequeuedNpdu<'a> {
    /// Destination MAC address.
    pub destination: u8,
    /// Whether this was taken from the unicast or broadcast queue.
    pub kind: QueueKind,
    /// BACnet network priority.
    pub priority: NetworkPriority,
    /// Sequence assigned when the item was accepted.
    pub sequence: u64,
    /// The NPDU, lent by the queue.
    pub npdu: &'a [u8],
}

impl<'a> DequeuedNpdu<'a> {
    /// An item for `destination` from the `kind` queue, at `priority`, with
    /// the `sequence` it was accepted under, lending `npdu`.
    #[must_use]
    pub const fn new(
        destination: u8,
        kind: QueueKind,
        priority: NetworkPriority,
        sequence: u64,
        npdu: &'a [u8],
    ) -> Self {
        Self {
            destination,
            kind,
            priority,
            sequence,
            npdu,
        }
    }
}

/// A non-mutating view of the next item selected by queue policy.
///
/// It is `#[non_exhaustive]`: a [`QueueStorage`] of your own builds one with
/// [`Self::new`], then sets [`Self::expires_at`] and [`Self::service_wait`]
/// when they are not `None` and zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QueueItemInfo {
    /// Destination MAC address.
    pub destination: u8,
    /// Queue class.
    pub kind: QueueKind,
    /// BACnet network priority.
    pub priority: NetworkPriority,
    /// Payload length.
    pub len: usize,
    /// When the item was queued, on the core's timeline.
    pub enqueued_at: Instant,
    /// When the item expires, on the same timeline, if it does.
    pub expires_at: Option<Instant>,
    /// Sequence assigned when the item was accepted.
    pub sequence: u64,
    /// Number of service opportunities this item has waited.
    pub service_wait: u8,
}

impl QueueItemInfo {
    /// An item of `len` octets for `destination` in the `kind` queue, at
    /// `priority`, queued at `enqueued_at` under `sequence`; it never
    /// expires and has waited no service opportunity.
    #[must_use]
    pub const fn new(
        destination: u8,
        kind: QueueKind,
        priority: NetworkPriority,
        len: usize,
        enqueued_at: Instant,
        sequence: u64,
    ) -> Self {
        Self {
            destination,
            kind,
            priority,
            len,
            enqueued_at,
            expires_at: None,
            sequence,
            service_wait: 0,
        }
    }
}

/// A bounded storage backend for queued NPDUs: the core's transmit queues.
///
/// [`HeaplessQueues`] implements it, and `AllocQueues` with the `alloc`
/// feature; so may storage of your own, such as queues another task fills
/// too (the Tokio MS/TP transport keeps `AllocQueues` behind a lock that
/// way). It carries the queue policy as well as the items: which item goes
/// next, starvation, expiry and the oversize rule, so a backend of your own
/// carries them out as those two do.
///
/// It is an extension point, and its methods are part of this crate's
/// semver: changing one, or adding one without a default, is a breaking
/// change.
pub trait QueueStorage {
    /// Apply the core's service-opportunity starvation limit.
    fn set_starvation_limit(&mut self, limit: u8);

    /// The longest NPDU one queue slot holds. A core with
    /// [`MstpCoreConfig::extended_frames`](crate::MstpCoreConfig::extended_frames)
    /// on refuses queues whose slots hold less than
    /// [`MAX_EXTENDED_MPDU_DATA`](bacnet_mstp_codec::MAX_EXTENDED_MPDU_DATA);
    /// nothing else reads it.
    ///
    /// The default is a standard frame's 501 octets, so storage that doesn't
    /// say is refused for extended frames: it fails closed. Storage whose
    /// slots hold extended NPDUs overrides it.
    fn max_npdu(&self) -> usize {
        bacnet_mstp_codec::MAX_STANDARD_MPDU_DATA
    }

    /// Add a payload to the destination queue.
    ///
    /// `expires_at` is an absolute time on the same timeline as `now`, the
    /// core's. Expiry is inclusive: an item is expired when
    /// `now >= expires_at`.
    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRefusal>;

    /// Remove expired items at `now`, returning the number removed.
    fn expire(&mut self, now: Instant) -> usize;

    /// Inspect the item that policy would service next.
    fn peek(&self, now: Instant) -> Option<QueueItemInfo>;

    /// Inspect the next item in one destination class.
    ///
    /// The core uses this form when applying its configured
    /// broadcast/unicast service ratio.
    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo>;

    /// Remove the item that policy would service next, after removing the
    /// expired ones, and lend its NPDU until the queue is next used mutably.
    ///
    /// An item longer than `max_len` octets is dropped instead, counted in
    /// `drops_oversize`, and [`QueueRefusal::Oversize`] is returned.
    /// The core passes the longest NPDU its delivery slots hold, its `NPDU`,
    /// so such an item could never be sent; leaving it queued would keep it
    /// at the head.
    fn dequeue(
        &mut self,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        let Some(info) = self.peek(now) else {
            return Ok(None);
        };
        self.dequeue_kind(info.kind, now, max_len)
    }

    /// Remove the next item from one destination class and lend its NPDU,
    /// as [`Self::dequeue`] does, with its oversize rule.
    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal>;

    /// Current depth for one destination class.
    fn depth(&self, kind: QueueKind) -> usize;

    /// One destination class's counters since they were last taken,
    /// without taking them.
    fn counters(&self, kind: QueueKind) -> QueueCounters;

    /// One destination class's counters since they were last taken, which
    /// then start again (see [`QueueCounters`]).
    /// [`MasterCore::take_counters`](crate::MasterCore::take_counters) takes
    /// them with its own.
    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters;

    /// The total number of queued NPDUs.
    fn total_depth(&self) -> usize {
        self.depth(QueueKind::Unicast) + self.depth(QueueKind::Broadcast)
    }
}

/// Queues kept elsewhere, such as in a static, which the core borrows rather
/// than holds.
impl<T: QueueStorage + ?Sized> QueueStorage for &mut T {
    fn set_starvation_limit(&mut self, limit: u8) {
        (**self).set_starvation_limit(limit);
    }

    fn max_npdu(&self) -> usize {
        (**self).max_npdu()
    }

    fn enqueue_npdu(
        &mut self,
        destination: u8,
        payload: &[u8],
        priority: NetworkPriority,
        now: Instant,
        expires_at: Option<Instant>,
    ) -> Result<(), QueueRefusal> {
        (**self).enqueue_npdu(destination, payload, priority, now, expires_at)
    }

    fn expire(&mut self, now: Instant) -> usize {
        (**self).expire(now)
    }

    fn peek(&self, now: Instant) -> Option<QueueItemInfo> {
        (**self).peek(now)
    }

    fn peek_kind(&self, kind: QueueKind, now: Instant) -> Option<QueueItemInfo> {
        (**self).peek_kind(kind, now)
    }

    fn dequeue(
        &mut self,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        (**self).dequeue(now, max_len)
    }

    fn dequeue_kind(
        &mut self,
        kind: QueueKind,
        now: Instant,
        max_len: usize,
    ) -> Result<Option<DequeuedNpdu<'_>>, QueueRefusal> {
        (**self).dequeue_kind(kind, now, max_len)
    }

    fn depth(&self, kind: QueueKind) -> usize {
        (**self).depth(kind)
    }

    fn counters(&self, kind: QueueKind) -> QueueCounters {
        (**self).counters(kind)
    }

    fn take_counters(&mut self, kind: QueueKind) -> QueueCounters {
        (**self).take_counters(kind)
    }

    fn total_depth(&self) -> usize {
        (**self).total_depth()
    }
}

// The slots and selection policy that the two queue backends share.
mod slots;

mod heapless;

pub use heapless::HeaplessQueues;

#[cfg(feature = "alloc")]
mod alloc;

#[cfg(feature = "alloc")]
pub use alloc::AllocQueues;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod tests;
