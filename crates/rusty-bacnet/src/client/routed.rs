//! A `RoutedTarget` as the target of a typed call (#2617, #2613, #2588):
//! `read_property`, `read_property_multiple`, `read_range`, `read_log_page`,
//! `write_property`, `atomic_read_file`, `atomic_write_file`,
//! `subscribe_cov`, `unsubscribe_cov`, `subscribe_cov_property`,
//! `unsubscribe_cov_property` and `subscribe_cov_property_multiple` take one
//! where they take an address, on a standalone
//! client and on one from `endpoint.client()` alike, as the narrow endpoint
//! client did (#2612). Each call goes through the client's own routed
//! requests, so it is paced, sized and segmented as every other request is.
//!
//! A `str` stays an address on the local link and is never read as a
//! routed one. With `router` given, the request goes through that router.
//! With `router=None` the client uses the route it knows for the station,
//! the router its discovery-table row names, and asks the station
//! with a directed Who-Is first when it has no row, as `device()` does; a station
//! that doesn't answer raises `BacnetLookupError`, which says to pass
//! `router=`, and the client doesn't ask for it again for [`MISS_MEMORY`]
//! unless an I-Am from it arrives. A station on this client's own network,
//! once it knows the number, is sent to directly either way.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use bacnet_client::client::Station;
use bacnet_ops::{DeviceAddress, OpsError};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyString;
use tokio::sync::watch;

use crate::errors::to_py_err;
use crate::high_level::OpsClient;
use crate::types::{PeerAddresses, PyRoutedTarget};

/// What a typed call's `address` names: a station on the local link, as a
/// `str` always has, or a station behind a router.
pub(super) enum Address {
    /// A `str` in the client's address forms, parsed when the call runs.
    Local(String),
    /// A `RoutedTarget`.
    Routed(Routed),
}

impl Address {
    /// `address` as a call's target: a `str` or a `RoutedTarget`, and a
    /// TypeError for anything else, before anything is sent.
    pub(super) fn of(address: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(text) = address.cast::<PyString>() {
            return Ok(Self::Local(text.to_cow()?.into_owned()));
        }
        let Ok(target) = address.cast::<PyRoutedTarget>() else {
            return Err(PyTypeError::new_err(format!(
                "address must be a str or a RoutedTarget, not {}",
                address.get_type().name()?
            )));
        };
        let (network, station, router) = target.get().parts();
        Ok(Self::Routed(Routed {
            network,
            station: station.to_vec(),
            router: router.map(<[u8]>::to_vec),
        }))
    }

    /// The MAC of a local address, parsed for this client's link.
    pub(super) fn local(text: &str, peers: PeerAddresses) -> PyResult<Vec<u8>> {
        peers.parse(text)
    }
}

/// A station on another network, and the router to reach it by, if named.
pub(super) struct Routed {
    network: u16,
    station: Vec<u8>,
    router: Option<Vec<u8>>,
}

impl Routed {
    /// The path to the station on `client`'s link. A named router that is
    /// a broadcast or group address on the local link raises BacnetError:
    /// a routed request goes to its router by unicast. With no router, the
    /// discovery table's row for the station gives it, after one Who-Is to
    /// the station when there is no row (`device()`'s rule);
    /// with none after that, BacnetLookupError, and `misses` keeps the
    /// station so a call soon after raises at once instead of asking again.
    pub(super) async fn path(self, client: &OpsClient, misses: &RecentMisses) -> PyResult<Station> {
        let core = client.low_level();
        if let Some(router) = self.router {
            if core.transport().is_group_destination(&router) {
                return Err(to_py_err(Error::Encoding(
                    "a routed request goes to its router by unicast, and this router is a \
                     broadcast or group address on the local link: pass router=None to use \
                     the route the client learns"
                        .into(),
                )));
            }
            return Ok(Station::routed(&router, self.network, &self.station));
        }
        let address =
            DeviceAddress::new(self.network, &self.station).map_err(PyValueError::new_err)?;
        let style = client.address_style();
        let at = address.clone().localized(core.network_number());
        let key = (self.network, self.station.clone());
        // A row, from a Who-Is or an I-Am the station sent on its own, is the
        // route at once, whatever an earlier miss said.
        if !at.is_local() && at.row(core).await.is_none() {
            // One lookup per station at a time: calls that find one in
            // flight wait for what it finds instead of asking again.
            loop {
                if misses.recent(&key, Instant::now()) {
                    return Err(no_route(&at, self.network, Asked::Recently));
                }
                match misses.turn(&key) {
                    Turn::Wait(mut answer) => {
                        let found = answer.wait_for(Option::is_some).await.map(|found| *found);
                        match found {
                            Ok(Some(Found::Row)) => break,
                            Ok(Some(Found::Missed(asked))) => {
                                return Err(no_route(&at, self.network, asked));
                            }
                            // The call that asked went away before it
                            // knew: ask again.
                            Ok(None) | Err(_) => continue,
                        }
                    }
                    // Asks the station with a directed Who-Is, as device() does.
                    Turn::Ask(asking) => match client.device(address.clone()).await {
                        Ok(device) => {
                            drop(device);
                            misses.forget(&key);
                            asking.finish(Found::Row);
                            break;
                        }
                        Err(OpsError::UnknownDevice { asked, .. }) => {
                            let asked = if asked.is_some() {
                                misses.remember(key.clone(), Instant::now());
                                Asked::Now
                            } else {
                                Asked::Never
                            };
                            asking.finish(Found::Missed(asked));
                            return Err(no_route(&at, self.network, asked));
                        }
                        // The waiters ask again for themselves.
                        Err(error) => return Err(crate::high_level::ops_err(error, style)),
                    },
                }
            }
        } else if at.is_local() {
            // Checks the MAC against the link and sends nothing.
            drop(
                client
                    .device(address.clone())
                    .await
                    .map_err(|error| crate::high_level::ops_err(error, style))?,
            );
            return Ok(Station::local(&self.station));
        }
        let row = at.row(core).await.ok_or_else(|| {
            crate::errors::lookup_error(
                format!("no route to {at}: the station's row left the discovery table"),
                crate::errors::Lookup::unknown(at.to_string()),
            )
        })?;
        Ok(Station::routed(
            &row.mac_address,
            self.network,
            &self.station,
        ))
    }
}

/// How long a station that didn't answer the client's Who-Is is remembered:
/// a call to it with no router in that time raises at once, unless an I-Am
/// from it has given the client a row.
pub(crate) const MISS_MEMORY: Duration = Duration::from_secs(30);

/// The most stations remembered at once; past it the oldest is forgotten.
const MISS_ROWS: usize = 256;

/// A station behind a router: its network and its MAC there.
type StationKey = (u16, Vec<u8>);

/// Stations behind a router that didn't answer the client's Who-Is, and
/// when, and the lookups in flight (#2617). Std mutexes, held only to look
/// up, add or remove, never across an await.
#[derive(Default)]
pub(super) struct RecentMisses {
    missed: Mutex<HashMap<StationKey, Instant>>,
    /// One lookup per station: a call that finds one here waits for what
    /// it finds.
    asking: Mutex<HashMap<StationKey, watch::Receiver<Option<Found>>>>,
}

/// What one lookup found, for the calls that waited on it.
#[derive(Clone, Copy)]
enum Found {
    /// A row: the route.
    Row,
    /// No answer.
    Missed(Asked),
}

/// A call's part in a station's lookup.
enum Turn<'a> {
    /// Ask: no lookup is in flight.
    Ask(Asking<'a>),
    /// Wait for the one in flight.
    Wait(watch::Receiver<Option<Found>>),
}

/// The one lookup in flight for a station. Dropped, finished or not, it
/// leaves the map; a call that waited on an unfinished one asks itself.
struct Asking<'a> {
    misses: &'a RecentMisses,
    station: StationKey,
    found: watch::Sender<Option<Found>>,
}

impl Asking<'_> {
    /// Tell the calls waiting what this lookup found.
    fn finish(&self, found: Found) {
        self.found.send_replace(Some(found));
    }
}

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        // Only this lookup inserts the station's entry, so it is its own.
        self.misses
            .asking
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.station);
    }
}

impl RecentMisses {
    /// Join the lookup in flight for `station`, or start it.
    fn turn(&self, station: &StationKey) -> Turn<'_> {
        let mut asking = self.asking.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(answer) = asking.get(station) {
            return Turn::Wait(answer.clone());
        }
        let (found, answer) = watch::channel(None);
        asking.insert(station.clone(), answer);
        Turn::Ask(Asking {
            misses: self,
            station: station.clone(),
            found,
        })
    }

    /// Whether `station` missed within [`MISS_MEMORY`] of `now`.
    fn recent(&self, station: &StationKey, now: Instant) -> bool {
        let misses = self.missed.lock().unwrap_or_else(PoisonError::into_inner);
        misses
            .get(station)
            .is_some_and(|&missed| now.saturating_duration_since(missed) < MISS_MEMORY)
    }

    /// Keep `station` as missed at `now`, dropping misses past
    /// [`MISS_MEMORY`] and, at [`MISS_ROWS`], the oldest.
    fn remember(&self, station: StationKey, now: Instant) {
        let mut misses = self.missed.lock().unwrap_or_else(PoisonError::into_inner);
        misses.retain(|_, &mut missed| now.saturating_duration_since(missed) < MISS_MEMORY);
        if misses.len() >= MISS_ROWS
            && let Some(oldest) = misses
                .iter()
                .min_by_key(|(_, missed)| **missed)
                .map(|(key, _)| key.clone())
        {
            misses.remove(&oldest);
        }
        misses.insert(station, now);
    }

    /// Forget `station`: it answered.
    fn forget(&self, station: &StationKey) {
        self.missed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(station);
    }
}

/// Whether the client asked the station.
#[derive(Clone, Copy)]
enum Asked {
    /// Just now, with no answer.
    Now,
    /// Within [`MISS_MEMORY`], with no answer: not again yet.
    Recently,
    /// Not at all: this client asks no network (no resolve wait).
    Never,
}

/// The BacnetLookupError for a station at `at` the client has no route to,
/// saying what to do: name its router.
fn no_route(at: &DeviceAddress, network: u16, asked: Asked) -> PyErr {
    let seconds = MISS_MEMORY.as_secs();
    let why = match asked {
        Asked::Now => format!(
            "it didn't answer a station-directed Who-Is, and the client won't ask that \
             station again for {seconds} s unless an I-Am from it arrives"
        ),
        Asked::Recently => format!(
            "it didn't answer a station-directed Who-Is in the last {seconds} s, and no \
             I-Am from it has arrived since"
        ),
        Asked::Never => {
            format!("this client asks no network for a station, network {network} included")
        }
    };
    crate::errors::lookup_error(
        format!(
            "no route to station {at}: {why}. A station that never sends an I-Am, such as an \
             MS/TP slave, needs its router named: RoutedTarget(..., router=...), with the \
             routers on this link from discover_routers()"
        ),
        crate::errors::Lookup::unknown(at.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A miss is kept for [`MISS_MEMORY`], forgotten when the station
    /// answers, and at [`MISS_ROWS`] the oldest goes first.
    #[test]
    fn misses_are_kept_for_a_bounded_time_and_count() {
        let misses = RecentMisses::default();
        let start = Instant::now();
        let station = (5, vec![0x0a]);
        assert!(!misses.recent(&station, start));
        misses.remember(station.clone(), start);
        assert!(misses.recent(&station, start + MISS_MEMORY - Duration::from_millis(1)));
        assert!(!misses.recent(&station, start + MISS_MEMORY));
        misses.forget(&station);
        assert!(!misses.recent(&station, start));

        for octet in 0..MISS_ROWS {
            let later = start + Duration::from_millis(octet as u64);
            misses.remember((7, (octet as u16).to_be_bytes().to_vec()), later);
        }
        let newest = start + Duration::from_secs(1);
        misses.remember((7, vec![0xff, 0xff, 0xff]), newest);
        assert!(!misses.recent(&(7, vec![0, 0]), newest), "the oldest went");
        assert!(misses.recent(&(7, vec![0, 1]), newest));
        assert!(misses.recent(&(7, vec![0xff, 0xff, 0xff]), newest));
        // Expired misses go before the oldest live one.
        misses.remember((8, vec![1]), start + MISS_MEMORY + Duration::from_secs(2));
        assert_eq!(misses.missed.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    /// One lookup per station: a second call waits on the first, which
    /// leaves the map when it ends, finished or dropped.
    #[tokio::test]
    async fn one_lookup_per_station_and_a_dropped_one_lets_the_next_ask() {
        let misses = RecentMisses::default();
        let station = (5, vec![0x0a]);
        let Turn::Ask(first) = misses.turn(&station) else {
            panic!("the first call asks");
        };
        let Turn::Wait(mut waiting) = misses.turn(&station) else {
            panic!("the second waits");
        };
        assert!(
            matches!(misses.turn(&(6, vec![0x0a])), Turn::Ask(_)),
            "another station asks"
        );
        first.finish(Found::Missed(Asked::Now));
        let found = waiting.wait_for(Option::is_some).await.map(|found| *found);
        assert!(matches!(found, Ok(Some(Found::Missed(Asked::Now)))));
        drop(first);
        // Dropped unfinished, a lookup wakes its waiters with nothing.
        let Turn::Ask(second) = misses.turn(&station) else {
            panic!("a finished lookup left the map");
        };
        let Turn::Wait(mut waiting) = misses.turn(&station) else {
            panic!("waits on the second");
        };
        drop(second);
        assert!(waiting.wait_for(Option::is_some).await.is_err());
        assert!(matches!(misses.turn(&station), Turn::Ask(_)));
    }
}
