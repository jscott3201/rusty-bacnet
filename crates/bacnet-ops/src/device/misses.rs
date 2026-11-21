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
/// silent. A station that answered only its network's Who-Is is kept until
/// [`MISS_ROWS`] pushes it out, or it goes silent.
pub const MISS_MEMORY: Duration = Duration::from_secs(30);

/// The most stations remembered at once; past it the oldest goes.
pub(crate) const MISS_ROWS: usize = 256;

/// What a lookup by address learned about a station that missed the
/// station-addressed Who-Is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Miss {
    /// It answered the Who-Is to its network: ask that form first.
    NetworkOnly,
    /// It answered neither form.
    Silent,
}

/// The stations, each with what it missed and when.
#[derive(Debug, Default)]
pub(crate) struct StationMisses {
    rows: HashMap<DeviceAddress, (Miss, Instant)>,
}

impl StationMisses {
    /// What `station` is remembered for at `now`: a silent miss counts for
    /// [`MISS_MEMORY`].
    pub(crate) fn get(&self, station: &DeviceAddress, now: Instant) -> Option<Miss> {
        self.rows
            .get(station)
            .filter(|(miss, at)| live(*miss, *at, now))
            .map(|(miss, _)| *miss)
    }

    /// Keep `station` as `miss` at `now`, dropping silent misses past
    /// [`MISS_MEMORY`] and, at [`MISS_ROWS`], the oldest.
    pub(crate) fn remember(&mut self, station: DeviceAddress, miss: Miss, now: Instant) {
        self.rows.retain(|_, (miss, at)| live(*miss, *at, now));
        if self.rows.len() >= MISS_ROWS
            && !self.rows.contains_key(&station)
            && let Some(oldest) = self
                .rows
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(key, _)| key.clone())
        {
            self.rows.remove(&oldest);
        }
        self.rows.insert(station, (miss, now));
    }

    /// Forget `station`: it answered a Who-Is addressed to it.
    pub(crate) fn forget(&mut self, station: &DeviceAddress) {
        self.rows.remove(station);
    }
}

/// Whether a row kept at `at` still counts at `now`.
fn live(miss: Miss, at: Instant, now: Instant) -> bool {
    miss == Miss::NetworkOnly || now.saturating_duration_since(at) < MISS_MEMORY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(network: u16, mac: &[u8]) -> DeviceAddress {
        DeviceAddress::new(network, mac).unwrap()
    }

    /// A silent miss is kept for [`MISS_MEMORY`], a network-only one until
    /// the cap; one forgotten is gone, and at [`MISS_ROWS`] the oldest goes
    /// first, after the expired.
    #[test]
    fn misses_are_kept_for_a_bounded_time_and_count() {
        let mut misses = StationMisses::default();
        let start = Instant::now();
        let silent = station(5, &[0x0a]);
        let proxied = station(5, &[0x0b]);
        assert_eq!(misses.get(&silent, start), None);
        misses.remember(silent.clone(), Miss::Silent, start);
        misses.remember(proxied.clone(), Miss::NetworkOnly, start);
        let almost = start + MISS_MEMORY - Duration::from_millis(1);
        assert_eq!(misses.get(&silent, almost), Some(Miss::Silent));
        assert_eq!(misses.get(&silent, start + MISS_MEMORY), None);
        let much_later = start + MISS_MEMORY * 100;
        assert_eq!(misses.get(&proxied, much_later), Some(Miss::NetworkOnly));
        misses.forget(&proxied);
        assert_eq!(misses.get(&proxied, start), None);
        misses.forget(&silent);

        for n in 0..MISS_ROWS {
            let later = start + Duration::from_millis(n as u64);
            let mac = u16::try_from(n).unwrap().to_be_bytes();
            misses.remember(station(7, &mac), Miss::NetworkOnly, later);
        }
        let newest = start + Duration::from_secs(1);
        misses.remember(station(7, &[0xff, 0xff, 0xff]), Miss::NetworkOnly, newest);
        assert_eq!(
            misses.get(&station(7, &[0, 0]), newest),
            None,
            "the oldest went"
        );
        assert!(misses.get(&station(7, &[0, 1]), newest).is_some());
        assert_eq!(misses.rows.len(), MISS_ROWS);
        // Expired silent misses go before any live row.
        misses.remember(silent.clone(), Miss::Silent, newest);
        let after = newest + MISS_MEMORY + Duration::from_secs(1);
        misses.remember(station(8, &[1]), Miss::Silent, after);
        assert_eq!(misses.get(&silent, after), None);
        assert_eq!(misses.rows.len(), MISS_ROWS, "the expired one made room");
    }
}
