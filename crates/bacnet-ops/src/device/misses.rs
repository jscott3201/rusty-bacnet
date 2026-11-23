//! The client's one memory of the stations on other networks that a lookup
//! by address could not reach the usual way (#2599, #2617): those that
//! answer only a Who-Is to their network, and those that answered nothing.
//!
//! A lookup by address first sends a Who-Is addressed to the station. A
//! station that cannot answer that form, such as an MS/TP slave whose
//! slave proxy answers for it (Clause 16.10.2: the proxy answers a Who-Is
//! its own device receives, with the slave's MAC as the source), is asked
//! once more with a Who-Is to its network, and the station is remembered
//! here so the next lookup asks its network at once. A station that
//! answered neither is remembered for [`MISS_MEMORY`], for callers that
//! fail fast instead of asking again ([`Client::unanswered_recently`]).

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

use super::DeviceAddress;
#[cfg(doc)]
use crate::Client;

/// How long a station that answered neither Who-Is is remembered as
/// silent. Which form a station answers is a separate fact: a station that
/// answered only its network's Who-Is is asked that way first until the cap
/// of 256 remembered stations pushes it out, whether or not it was silent
/// since.
pub const MISS_MEMORY: Duration = Duration::from_secs(30);

/// The most stations remembered at once; past it the oldest goes.
pub(crate) const MISS_ROWS: usize = 256;

/// What a lookup by address learned about a station that missed the
/// station-addressed Who-Is. The two facts are kept apart, so one lost
/// I-Am from a proxied slave does not send its next lookup back to the
/// station-addressed form (#2599).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Known {
    /// It answered the Who-Is to its network: ask that form first.
    pub(crate) network_form: bool,
    /// It answered neither form within the last [`MISS_MEMORY`].
    pub(crate) silent: bool,
}

/// One remembered station.
#[derive(Debug, Clone, Copy)]
struct Row {
    /// It answered the Who-Is to its network.
    network_form: bool,
    /// When it last answered neither form, if it did.
    silent_at: Option<Instant>,
    /// When the row last changed: the oldest goes first at the cap.
    at: Instant,
}

impl Row {
    /// What the row says at `now`.
    fn known(&self, now: Instant) -> Known {
        Known {
            network_form: self.network_form,
            silent: self
                .silent_at
                .is_some_and(|at| now.saturating_duration_since(at) < MISS_MEMORY),
        }
    }
}

/// The stations, each with what it missed and when.
#[derive(Debug, Default)]
pub(crate) struct StationMisses {
    rows: HashMap<DeviceAddress, Row>,
}

impl StationMisses {
    /// What `station` is remembered for at `now`; nothing for a station not
    /// kept. A silent miss counts for [`MISS_MEMORY`].
    pub(crate) fn get(&self, station: &DeviceAddress, now: Instant) -> Known {
        self.rows
            .get(station)
            .map(|row| row.known(now))
            .unwrap_or_default()
    }

    /// `station` answered the Who-Is to its network at `now`: ask it that
    /// way first, and it is no longer silent.
    pub(crate) fn answered_network_form(&mut self, station: DeviceAddress, now: Instant) {
        self.keep(station, now, |row| {
            row.network_form = true;
            row.silent_at = None;
        });
    }

    /// `station` answered neither form at `now`. Which form it answers, if
    /// known, stays as it was.
    pub(crate) fn went_silent(&mut self, station: DeviceAddress, now: Instant) {
        self.keep(station, now, |row| row.silent_at = Some(now));
    }

    /// Update `station`'s row at `now` with `change`, dropping rows that say
    /// nothing any more and, at [`MISS_ROWS`], the oldest.
    fn keep(&mut self, station: DeviceAddress, now: Instant, change: impl FnOnce(&mut Row)) {
        self.rows
            .retain(|_, row| row.known(now) != Known::default());
        if self.rows.len() >= MISS_ROWS
            && !self.rows.contains_key(&station)
            && let Some(oldest) = self
                .rows
                .iter()
                .min_by_key(|(_, row)| row.at)
                .map(|(key, _)| key.clone())
        {
            self.rows.remove(&oldest);
        }
        let row = self.rows.entry(station).or_insert(Row {
            network_form: false,
            silent_at: None,
            at: now,
        });
        change(row);
        row.at = now;
    }

    /// Forget `station`: it answered a Who-Is addressed to it.
    pub(crate) fn forget(&mut self, station: &DeviceAddress) {
        self.rows.remove(station);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(network: u16, mac: &[u8]) -> DeviceAddress {
        DeviceAddress::new(network, mac).unwrap()
    }

    const NETWORK: Known = Known {
        network_form: true,
        silent: false,
    };
    const SILENT: Known = Known {
        network_form: false,
        silent: true,
    };

    /// A silent miss is kept for [`MISS_MEMORY`], a network-only one until
    /// the cap; one forgotten is gone, and at [`MISS_ROWS`] the oldest goes
    /// first, after the expired.
    #[test]
    fn misses_are_kept_for_a_bounded_time_and_count() {
        let mut misses = StationMisses::default();
        let start = Instant::now();
        let silent = station(5, &[0x0a]);
        let proxied = station(5, &[0x0b]);
        assert_eq!(misses.get(&silent, start), Known::default());
        misses.went_silent(silent.clone(), start);
        misses.answered_network_form(proxied.clone(), start);
        let almost = start + MISS_MEMORY - Duration::from_millis(1);
        assert_eq!(misses.get(&silent, almost), SILENT);
        assert_eq!(misses.get(&silent, start + MISS_MEMORY), Known::default());
        let much_later = start + MISS_MEMORY * 100;
        assert_eq!(misses.get(&proxied, much_later), NETWORK);
        misses.forget(&proxied);
        assert_eq!(misses.get(&proxied, start), Known::default());
        misses.forget(&silent);

        // Two-octet stations, LonTalk unicasts: a first octet of 0x00 or
        // 0xFF would be a broadcast or a group, no station (#2709).
        let mac = |n: usize| (0x0100 + u16::try_from(n).unwrap()).to_be_bytes();
        for n in 0..MISS_ROWS {
            let later = start + Duration::from_millis(n as u64);
            misses.answered_network_form(station(7, &mac(n)), later);
        }
        let newest = start + Duration::from_secs(1);
        // Three octets, so none of the two-octet stations above; not all
        // ones, which is no station (#2709).
        misses.answered_network_form(station(7, &[0x3f, 0xff, 0xff]), newest);
        assert_eq!(
            misses.get(&station(7, &mac(0)), newest),
            Known::default(),
            "the oldest went"
        );
        assert_eq!(misses.get(&station(7, &mac(1)), newest), NETWORK);
        assert_eq!(misses.rows.len(), MISS_ROWS);
        // Expired silent misses go before any live row.
        misses.went_silent(silent.clone(), newest);
        let after = newest + MISS_MEMORY + Duration::from_secs(1);
        misses.went_silent(station(8, &[1]), after);
        assert_eq!(misses.get(&silent, after), Known::default());
        assert_eq!(misses.rows.len(), MISS_ROWS, "the expired one made room");
    }

    /// A station known to answer only its network's Who-Is keeps that fact
    /// through a silent spell, and after it; an answer clears the silence.
    #[test]
    fn silence_does_not_erase_the_network_form() {
        let mut misses = StationMisses::default();
        let start = Instant::now();
        let proxied = station(5, &[0x0a]);
        misses.answered_network_form(proxied.clone(), start);
        misses.went_silent(proxied.clone(), start);
        let both = Known {
            network_form: true,
            silent: true,
        };
        assert_eq!(misses.get(&proxied, start), both);
        assert_eq!(misses.get(&proxied, start + MISS_MEMORY), NETWORK);
        // Kept past the window even when other rows are swept.
        misses.went_silent(station(8, &[1]), start + MISS_MEMORY * 2);
        assert_eq!(misses.get(&proxied, start + MISS_MEMORY * 2), NETWORK);
        misses.went_silent(proxied.clone(), start);
        misses.answered_network_form(proxied.clone(), start);
        assert_eq!(misses.get(&proxied, start), NETWORK);
    }
}
