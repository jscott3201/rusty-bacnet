//! The state a test shares with its transport: counters, settable limits and
//! the built-in send controls.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use bacnet_npdu::NpduLimits;
use bacnet_types::error::Error;
use tokio::sync::{Semaphore, mpsc};

use super::SendLog;
use crate::port::ReceivedNpdu;

/// What the inner transport reports, as of the test transport's last query
/// or lifecycle call; see [`TestTransportHandle::inner_limits`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InnerLimits {
    /// The inner transport's local receive capacity.
    pub receive_capacity: u16,
    /// The inner transport's egress APDU limit.
    pub egress_limit: u16,
    /// The inner transport's NPDU limits.
    pub max_npdu: NpduLimits,
}

/// Values a test sets; `None` leaves the answer to the inner transport, or
/// to the default without one.
#[derive(Default)]
pub(super) struct Settings {
    pub(super) receive_capacity: Option<u16>,
    pub(super) egress_limit: Option<u16>,
    pub(super) max_npdu: Option<NpduLimits>,
}

pub(super) struct Shared {
    pub(super) sent: SendLog,
    pub(super) starts: AtomicUsize,
    pub(super) stops: AtomicUsize,
    pub(super) aborts: AtomicUsize,
    pub(super) prepares: AtomicUsize,
    pub(super) settings: Mutex<Settings>,
    pub(super) inner_limits: Mutex<Option<InnerLimits>>,
    /// The sender of the channel the last `StartMode::Fresh` start made,
    /// until a stop or abort drops it.
    pub(super) inbound: Mutex<Option<mpsc::Sender<ReceivedNpdu>>>,
    fail_next: AtomicBool,
    block_next: AtomicBool,
    blocked: Semaphore,
    release: Semaphore,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            sent: SendLog::default(),
            starts: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            aborts: AtomicUsize::new(0),
            prepares: AtomicUsize::new(0),
            settings: Mutex::default(),
            inner_limits: Mutex::new(None),
            inbound: Mutex::new(None),
            fail_next: AtomicBool::new(false),
            block_next: AtomicBool::new(false),
            blocked: Semaphore::new(0),
            release: Semaphore::new(0),
        }
    }
}

impl Shared {
    pub(super) fn settings(&self) -> std::sync::MutexGuard<'_, Settings> {
        self.settings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) async fn controls(&self) -> Result<(), Error> {
        let fail_once = self.fail_next.swap(false, Ordering::SeqCst);
        if self.block_next.swap(false, Ordering::SeqCst) {
            self.blocked.add_permits(1);
            self.release.acquire().await.unwrap().forget();
        }
        if fail_once {
            return Err(Error::Encoding(
                "injected test transport send failure".into(),
            ));
        }
        Ok(())
    }
}

/// Cloneable view of a transport's log, counters and built-in send controls.
///
/// Take it from the builder or the transport before the transport moves into
/// the stack under test.
#[derive(Clone)]
pub struct TestTransportHandle {
    pub(super) shared: std::sync::Arc<Shared>,
}

impl TestTransportHandle {
    /// The send log.
    pub fn sent(&self) -> SendLog {
        self.shared.sent.clone()
    }

    /// Calls to `start`, counted before the start mode runs.
    pub fn starts(&self) -> usize {
        self.shared.starts.load(Ordering::SeqCst)
    }

    /// Calls to `stop`, counted before the stop hook runs.
    pub fn stops(&self) -> usize {
        self.shared.stops.load(Ordering::SeqCst)
    }

    /// Calls to `abort`.
    pub fn aborts(&self) -> usize {
        self.shared.aborts.load(Ordering::SeqCst)
    }

    /// Calls to `prepare_for_routing`, whether or not they reach an inner
    /// transport.
    pub fn prepares(&self) -> usize {
        self.shared.prepares.load(Ordering::SeqCst)
    }

    /// The sender of the channel the latest [`StartMode::Fresh`] start made,
    /// or `None` before a start and after a stop or abort has dropped it.
    /// Receivers see the link close once the test drops the clones it took.
    ///
    /// [`StartMode::Fresh`]: super::StartMode::Fresh
    pub fn inbound(&self) -> Option<mpsc::Sender<ReceivedNpdu>> {
        self.shared
            .inbound
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// What the inner transport reported at the test transport's last limit
    /// query, start, stop or `prepare_for_routing`, or `None` when there is
    /// no inner transport or none of those has happened yet.
    pub fn inner_limits(&self) -> Option<InnerLimits> {
        *self
            .shared
            .inner_limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Change what the transport reports as its receive capacity (1476 at
    /// first), as a real link might after start (#2093).
    pub fn set_receive_capacity(&self, capacity: u16) {
        self.shared.settings().receive_capacity = Some(capacity);
    }

    /// Change what the transport reports as its egress limit (1476 at first).
    pub fn set_egress_limit(&self, limit: u16) {
        self.shared.settings().egress_limit = Some(limit);
    }

    /// Change what the transport reports as its NPDU limits, unless an
    /// [`on_max_npdu`](super::TestTransportBuilder::on_max_npdu) hook answers.
    pub fn set_max_npdu(&self, limits: NpduLimits) {
        self.shared.settings().max_npdu = Some(limits);
    }

    /// Fail the next recorded send only.
    pub fn fail_next_send(&self) {
        self.shared.fail_next.store(true, Ordering::SeqCst);
    }

    /// Hold the next recorded send only, until released.
    pub fn block_next_send(&self) {
        self.shared.block_next.store(true, Ordering::SeqCst);
    }

    /// Let `permits` held sends (current or future) continue.
    pub fn release_sends(&self, permits: usize) {
        self.shared.release.add_permits(permits);
    }

    /// Wait until one more send has become held. Each held send is counted
    /// once, so a send held before the wait still satisfies it.
    pub async fn wait_blocked(&self) {
        self.shared.blocked.acquire().await.unwrap().forget();
    }
}
