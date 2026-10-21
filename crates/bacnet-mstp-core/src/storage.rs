//! Everything large a core works in, in one caller-owned bundle.

use core::fmt;

use bacnet_mstp_codec::{
    MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_FRAME_LENGTH,
    MAX_STANDARD_MPDU_DATA,
};

use crate::actions::{ActionBuffers, DEFAULT_DELIVERY_SLOTS};
use crate::queue::FixedQueues;

/// A core's receive storage, transmit queues and [`ActionBuffers`], sized
/// together, which [`MasterCore::new`](crate::MasterCore::new) borrows for
/// the core's lifetime.
///
/// `Q` is the queue storage, `DELIVERIES` the delivery slots, `NPDU` the
/// longest NPDU a slot holds (and so the core's own `NPDU`), and `FRAME`
/// the longest frame on the wire, which sizes both the frame being sent
/// and the receive storage. Most callers name one of the two aliases,
/// [`StandardStorage`] or [`ExtendedStorage`], which keep the sizes
/// consistent; a caller with queues of its own (shared with other tasks,
/// say) builds one with [`Self::with_queues`].
///
/// The receive storage always has room for `FRAME` octets, but the core
/// uses only what its configuration takes: an extended frame's
/// [`MAX_EXTENDED_FRAME_LENGTH`] with
/// [`MstpCoreConfig::extended_frames`](crate::MstpCoreConfig::extended_frames)
/// on, a standard frame's [`MAX_STANDARD_FRAME_LENGTH`] without. So one
/// extended-sized bundle serves either setting, and a core without the
/// flag treats an extended frame as a node without COBS support does,
/// counting it invalid at its header.
///
/// [`StandardStorage::new`] and [`ExtendedStorage::new`] are `const fn`s
/// whose result is all zero bits, so on firmware the bundle lives in a
/// zero-initialized static (`.bss`) and costs no flash for an initial
/// image (see the crate docs).
pub struct Storage<
    Q,
    const DELIVERIES: usize = DEFAULT_DELIVERY_SLOTS,
    const NPDU: usize = MAX_STANDARD_MPDU_DATA,
    const FRAME: usize = MAX_STANDARD_FRAME_LENGTH,
> {
    pub(crate) rx: [u8; FRAME],
    pub(crate) queues: Q,
    pub(crate) actions: ActionBuffers<DELIVERIES, NPDU, FRAME>,
}

/// [`Storage`] for a core without extended frames: [`FixedQueues`] of `U`
/// unicast and `B` broadcast slots, `DELIVERIES` delivery slots, all for
/// standard frames of up to 501 octets of data.
pub type StandardStorage<
    const U: usize,
    const B: usize,
    const DELIVERIES: usize = DEFAULT_DELIVERY_SLOTS,
> = Storage<FixedQueues<U, B>, DELIVERIES>;

/// [`Storage`] for a core with
/// [`MstpCoreConfig::extended_frames`](crate::MstpCoreConfig::extended_frames)
/// on: the same as [`StandardStorage`], with every buffer sized for
/// extended frames ([`MAX_EXTENDED_MPDU_DATA`] octets of data,
/// [`MAX_EXTENDED_FRAME_LENGTH`] on the wire).
pub type ExtendedStorage<
    const U: usize,
    const B: usize,
    const DELIVERIES: usize = DEFAULT_DELIVERY_SLOTS,
> = Storage<
    FixedQueues<U, B, MAX_EXTENDED_MPDU_DATA>,
    DELIVERIES,
    MAX_EXTENDED_MPDU_DATA,
    MAX_EXTENDED_FRAME_LENGTH,
>;

impl<Q, const DELIVERIES: usize, const NPDU: usize, const FRAME: usize>
    Storage<Q, DELIVERIES, NPDU, FRAME>
{
    /// Zeroed buffers around `queues`.
    #[must_use]
    pub const fn with_queues(queues: Q) -> Self {
        Self {
            rx: [0; FRAME],
            queues,
            actions: ActionBuffers::new(),
        }
    }

    /// The queues, to read once the core that borrowed them is gone.
    #[must_use]
    pub const fn queues(&self) -> &Q {
        &self.queues
    }

    /// The queues, mutably, while no core borrows them.
    pub fn queues_mut(&mut self) -> &mut Q {
        &mut self.queues
    }
}

impl<
        const U: usize,
        const B: usize,
        const DELIVERIES: usize,
        const NPDU: usize,
        const FRAME: usize,
    > Storage<FixedQueues<U, B, NPDU>, DELIVERIES, NPDU, FRAME>
{
    /// Zeroed storage with empty queues: all zero bits.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_queues(FixedQueues::new())
    }
}

impl<
        const U: usize,
        const B: usize,
        const DELIVERIES: usize,
        const NPDU: usize,
        const FRAME: usize,
    > Default for Storage<FixedQueues<U, B, NPDU>, DELIVERIES, NPDU, FRAME>
{
    fn default() -> Self {
        Self::new()
    }
}

/// The sizes and the queues, not the octets: the core overwrites the
/// buffers before it reads them.
impl<Q: fmt::Debug, const DELIVERIES: usize, const NPDU: usize, const FRAME: usize> fmt::Debug
    for Storage<Q, DELIVERIES, NPDU, FRAME>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Storage")
            .field("rx", &FRAME)
            .field("queues", &self.queues)
            .field("actions", &self.actions)
            .finish()
    }
}

/// The receive storage a core with `extended` frames uses out of `frame`
/// octets.
pub(crate) const fn receive_window(extended: bool, frame: usize) -> usize {
    let wanted = if extended {
        MAX_EXTENDED_FRAME_LENGTH
    } else {
        MAX_STANDARD_FRAME_LENGTH
    };
    if wanted < frame {
        wanted
    } else {
        frame
    }
}
