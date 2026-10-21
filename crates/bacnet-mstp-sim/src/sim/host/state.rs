//! Buffered state is owned by one powered incarnation, never queued globally.
use std::collections::VecDeque;

use bacnet_mstp_core::{Duration, Instant, RequestId, TransmitId};

use super::echo::EchoFilter;
use super::local_echo::Return;
use crate::line::Character;
use crate::report::HostStats;

/// A serial write can outlive physical driver release; its completion stamp
/// remains the later of drain return and wire end, even when echo waits.
pub(super) struct Write {
    pub id: TransmitId,
    pub incarnation: u64,
    pub start: Instant,
    pub wire_end: Instant,
    pub watchdog: Instant,
    pub ended: Instant,
    pub frame: Vec<u8>,
    pub hunting: bool,
}

/// Physical provenance follows the bytes, independently of delivery time.
#[derive(Clone, Copy)]
pub(super) struct Received {
    pub character: Character,
    pub wire_end: Instant,
}

/// Bound to an accepted core request, never to the most recent wire frame.
#[derive(Clone, Copy)]
pub(crate) struct WireRequest {
    pub id: RequestId,
    pub source: u8,
    pub incarnation: u64,
    pub end: Instant,
}

pub(super) struct Decision {
    pub request: RequestId,
    pub requester: u8,
    pub postpone: bool,
    pub payload: Vec<u8>,
}

pub(crate) struct HostState {
    /// Opt-in, bounded read recorder for simulator/public-transport pairs.
    #[cfg(test)]
    pub(super) read_capture: Option<Vec<(Instant, Vec<Received>)>>,
    pub(super) local_echo: VecDeque<Return>,
    /// One invariant report per sustained full-queue episode, with every
    /// refused character still counted in the run's compact diagnostics.
    pub(super) local_echo_overflow: bool,
    pub(super) input: VecDeque<Received>,
    /// Provenance for the filter's held prefix, bounded by the sent frame.
    pub(super) held_at: VecDeque<Instant>,
    pub(super) decisions: VecDeque<Decision>,
    /// Physical stop bit of the octet currently entering the core.
    pub(super) receiving: Option<Instant>,
    pub(crate) request: Option<WireRequest>,
    pub(super) ready: Option<Instant>,
    pub(super) write: Option<Write>,
    pub(super) filter: EchoFilter,
    pub(crate) stats: HostStats,
    pub(crate) activity: Option<Instant>,
    /// A deferred completion can expose an already overdue core timer.
    pub(crate) completion_reported: Option<Instant>,
    /// Cumulative receive oracle, deferred until input has reached the core.
    pub(super) expected: VecDeque<(Instant, u32)>,
}

impl HostState {
    pub(crate) fn writing(&self) -> bool {
        self.write.is_some()
    }

    pub(crate) fn unread_until(&self) -> Option<Instant> {
        self.ready.filter(|_| !self.input.is_empty())
    }

    pub(crate) fn new(frame_abort: Duration) -> Self {
        Self {
            #[cfg(test)]
            read_capture: None,
            local_echo: VecDeque::new(),
            local_echo_overflow: false,
            input: VecDeque::new(),
            held_at: VecDeque::new(),
            decisions: VecDeque::new(),
            receiving: None,
            request: None,
            ready: None,
            write: None,
            filter: EchoFilter::new(frame_abort),
            stats: HostStats::default(),
            activity: None,
            expected: VecDeque::new(),
            completion_reported: None,
        }
    }

    pub(crate) fn next(&self) -> Option<Instant> {
        if let Some(write) = &self.write {
            if !write.hunting {
                return Some(write.ended);
            }
        }
        self.ready
            .into_iter()
            .chain(self.filter.deadline())
            .chain(self.expected.front().map(|(at, _)| *at))
            .min()
    }

    /// One live scheduler candidate, owned by this incarnation. Removing the
    /// state on power loss cancels it without stale global character timers.
    pub(crate) fn next_local_echo(&self) -> Option<Instant> {
        self.local_echo.front().map(|echo| echo.available_at)
    }
}
