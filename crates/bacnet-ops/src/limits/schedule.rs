//! One rate's booked slots, and the guard that gives back a slot whose
//! wait was abandoned (#2202): a cancelled tool call or a withdrawn job
//! leaves nothing booked, so later requests don't queue behind slots
//! nobody sends in.

use std::time::Duration;

use tokio::time::Instant;

/// One rate's schedule: the next free slot, and the slots given back
/// before their instant, which the next bookings take first.
#[derive(Debug, Default)]
pub(super) struct Schedule {
    next: Option<Instant>,
    /// Slots given back while still ahead, earliest first.
    holes: Vec<Booking>,
}

/// A booked slot: its instant, and the instant the slot after it may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Booking {
    pub(super) at: Instant,
    end: Instant,
}

impl Schedule {
    /// The next free slot, holes aside.
    pub(super) fn next(&self) -> Option<Instant> {
        self.next
    }

    /// Book the first free slot at or after `now`, `interval` (not zero)
    /// wide: a slot given back that is still ahead and wide enough, else
    /// the next one.
    pub(super) fn book(&mut self, now: Instant, interval: Duration) -> Booking {
        self.holes.retain(|hole| hole.at >= now);
        if let Some(i) = self
            .holes
            .iter()
            .position(|hole| hole.end.saturating_duration_since(hole.at) >= interval)
        {
            return self.holes.remove(i);
        }
        let at = self.next.map_or(now, |next| next.max(now));
        let booking = Booking {
            at,
            end: at + interval,
        };
        self.next = Some(booking.end);
        booking
    }

    /// Drop the schedule: without spacing a request goes `now`, whatever
    /// spaced requests left booked.
    pub(super) fn clear(&mut self, now: Instant) {
        self.next = Some(now);
        self.holes.clear();
    }

    /// Give `booking` back, unless its instant has come by `now` (its
    /// request may have gone). The newest booking rolls the schedule back,
    /// with any slots given back just before it; another becomes a hole.
    /// Returns whether it was given back.
    pub(super) fn release(&mut self, booking: Booking, now: Instant) -> bool {
        if booking.at <= now {
            return false;
        }
        if self.next == Some(booking.end) {
            let mut next = booking.at;
            while let Some(i) = self.holes.iter().position(|hole| hole.end == next) {
                next = self.holes.remove(i).at;
            }
            self.next = Some(next);
        } else {
            let i = self.holes.partition_point(|hole| hole.at < booking.at);
            self.holes.insert(i, booking);
        }
        true
    }
}
