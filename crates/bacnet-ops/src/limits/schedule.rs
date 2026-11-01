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
    /// The earliest instant a booking `interval` wide might get, as of
    /// `now`: the first hole still ahead and wide enough, or the next free
    /// slot, whichever is sooner. An estimate: [`book`](Self::book) prefers
    /// a hole even when the next slot is earlier, and drops holes that have
    /// begun. `None` when nothing was ever booked.
    pub(super) fn next_free(&self, now: Instant, interval: Duration) -> Option<Instant> {
        let hole = self
            .holes
            .iter()
            .find(|hole| hole.at >= now && hole.width() >= interval)
            .map(|hole| hole.at);
        match (hole, self.next) {
            (Some(hole), Some(next)) => Some(hole.min(next)),
            (hole, next) => hole.or(next),
        }
    }

    /// Book the first free slot at or after `now`, `interval` (not zero)
    /// wide: the front of a slot given back that is still ahead and wide
    /// enough, else the next one. The rest of a wider hole stays a hole,
    /// so it serves more requests at this spacing.
    pub(super) fn book(&mut self, now: Instant, interval: Duration) -> Booking {
        self.holes.retain(|hole| hole.at >= now);
        if let Some(i) = self.holes.iter().position(|hole| hole.width() >= interval) {
            let hole = &mut self.holes[i];
            let front = Booking {
                at: hole.at,
                end: hole.at + interval,
            };
            if front.end < hole.end {
                hole.at = front.end;
            } else {
                self.holes.remove(i);
            }
            return front;
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
            // Merged with the holes it touches, so a wider request fits.
            let i = self.holes.partition_point(|hole| hole.at < booking.at);
            self.holes.insert(i, booking);
            if self
                .holes
                .get(i + 1)
                .is_some_and(|after| after.at == booking.end)
            {
                self.holes[i].end = self.holes.remove(i + 1).end;
            }
            if i > 0 && self.holes[i - 1].end == booking.at {
                self.holes[i - 1].end = self.holes.remove(i).end;
            }
        }
        true
    }
}

impl Booking {
    /// How long until the slot after it may go.
    fn width(&self) -> Duration {
        self.end.saturating_duration_since(self.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-second hole serves ten requests 100 ms apart, front first,
    /// and the eleventh goes after the schedule's last slot.
    #[test]
    fn a_wide_hole_serves_several_requests_at_a_finer_spacing() {
        let start = Instant::now();
        let second = Duration::from_secs(1);
        let mut schedule = Schedule::default();
        let booked: Vec<Booking> = (0..3).map(|_| schedule.book(start, second)).collect();
        assert!(schedule.release(booked[1], start));
        let spacing = Duration::from_millis(100);
        assert_eq!(schedule.next_free(start, spacing), Some(start + second));
        let served: Vec<Instant> = (0..10).map(|_| schedule.book(start, spacing).at).collect();
        let expected: Vec<Instant> = (0..10).map(|i| start + second + spacing * i).collect();
        assert_eq!(served, expected);
        assert_eq!(schedule.book(start, spacing).at, start + second * 3);
    }

    /// Holes given back side by side merge, so a request as wide as both
    /// together fits.
    #[test]
    fn adjacent_holes_merge() {
        let start = Instant::now();
        let second = Duration::from_secs(1);
        let mut schedule = Schedule::default();
        let booked: Vec<Booking> = (0..4).map(|_| schedule.book(start, second)).collect();
        assert!(schedule.release(booked[2], start));
        assert!(schedule.release(booked[1], start));
        assert_eq!(schedule.book(start, second * 2).at, start + second);
        assert_eq!(schedule.next_free(start, second), Some(start + second * 4));
    }
}
